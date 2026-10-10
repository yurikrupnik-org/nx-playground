//! `butler ui` — the web UIs of the dev loop (Flux, Grafana, Prometheus, ...),
//! registered in butler.toml `[ui.<name>]`, and the local ports they and
//! everything else listen on ([`ports`]). The file is the workspace's
//! butler.toml, else the user default ([`settings::resolve`]); every function
//! here takes that file's path.
//!
//! butler.toml stays a hand-written file: every change goes through
//! `toml_edit`, so its comments and layout survive, and the edited text must
//! pass the same validation as `butler.toml` itself before it is written. The
//! CLI and the web UI ([`serve`]) call the same functions.

pub mod ports;
pub mod serve;

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::settings::{self, Root};

/// One registered UI, with its URL taken apart.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Ui {
    pub name: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    /// The host is this machine, so [`ports::used`] can tell whether anything
    /// listens behind it.
    pub local: bool,
}

impl Ui {
    fn new(name: &str, url: &str) -> Result<Self> {
        let parsed =
            Url::parse(url).wrap_err_with(|| format!("{} [ui.{name}] url", settings::FILE))?;
        Ok(Self {
            name: name.to_string(),
            url: url.to_string(),
            host: parsed.host.to_string(),
            port: parsed.port,
            local: crate::web::is_loopback_name(parsed.host),
        })
    }
}

/// Every registered UI, sorted by name.
pub fn load(file: &Path) -> Result<Vec<Ui>> {
    from_settings(&Root::load(file)?)
}

fn from_settings(settings: &Root) -> Result<Vec<Ui>> {
    settings
        .ui
        .iter()
        .map(|(name, link)| {
            check_name(name)?;
            Ui::new(name, &link.url)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Changes

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddRequest {
    pub name: String,
    pub url: String,
}

pub fn add(file: &Path, req: &AddRequest) -> Result<()> {
    let name = req.name.trim();
    let url = req.url.trim();
    check_name(name)?;
    Url::parse(url)?;
    edit(file, |uis| {
        if uis.contains_key(name) {
            bail!("UI `{name}` already exists; change it with `butler ui set {name}`");
        }
        let mut table = Table::new();
        table.insert("url", toml_edit::value(url));
        uis.insert(name, Item::Table(table));
        Ok(())
    })
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRequest {
    pub url: Option<String>,
    /// Applied after `url`: rewrites only the URL's port.
    pub port: Option<u16>,
}

/// Change a UI's URL and/or just its port, keeping the value's comment.
pub fn set(file: &Path, name: &str, req: &SetRequest) -> Result<()> {
    let url = req.url.as_deref().map(str::trim).filter(|u| !u.is_empty());
    if url.is_none() && req.port.is_none() {
        bail!("nothing to change: give a url and/or a port");
    }
    edit(file, |uis| {
        let known = names(uis);
        let slot = uis
            .get_mut(name)
            .and_then(Item::as_table_mut)
            .and_then(|t| t.get_mut("url"))
            .and_then(Item::as_value_mut)
            .ok_or_else(|| unknown(name, &known))?;
        let mut next = match url {
            Some(url) => url.to_string(),
            None => slot
                .as_str()
                .ok_or_else(|| eyre!("[ui.{name}] url must be a string"))?
                .to_string(),
        };
        if let Some(port) = req.port {
            next = Url::parse(&next)?.with_port(port);
        } else {
            Url::parse(&next)?;
        }
        let decor = slot.decor().clone();
        *slot = Value::from(next);
        *slot.decor_mut() = decor;
        Ok(())
    })
}

pub fn remove(file: &Path, name: &str) -> Result<()> {
    edit(file, |uis| {
        let known = names(uis);
        uis.remove(name)
            .map(drop)
            .ok_or_else(|| unknown(name, &known))
    })
}

/// Apply `change` to butler.toml's `ui` table, validate the whole result the
/// way `butler.toml` is loaded, and only then write it.
fn edit(file: &Path, change: impl FnOnce(&mut Table) -> Result<()>) -> Result<()> {
    let raw = std::fs::read_to_string(file).wrap_err_with(|| {
        format!(
            "reading {}; `butler ui` keeps its UIs there",
            file.display()
        )
    })?;
    let mut doc: DocumentMut = raw
        .parse()
        .wrap_err_with(|| format!("parsing {}", settings::FILE))?;
    let uis = doc
        .as_table_mut()
        .entry("ui")
        .or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(true);
            Item::Table(table)
        })
        .as_table_mut()
        .ok_or_else(|| {
            eyre!(
                "{}: `ui` must be a table of [ui.<name>] tables",
                settings::FILE
            )
        })?;
    change(uis)?;
    let text = doc.to_string();
    from_settings(&Root::parse(&text)?)?;
    std::fs::write(file, text).wrap_err_with(|| format!("writing {}", file.display()))
}

fn names(uis: &Table) -> Vec<String> {
    uis.iter().map(|(k, _)| k.to_string()).collect()
}

fn unknown(name: &str, known: &[String]) -> eyre::Report {
    if known.is_empty() {
        eyre!("no UI named `{name}`; none are registered")
    } else {
        eyre!("no UI named `{name}`; known: {}", known.join(", "))
    }
}

/// Names are TOML keys and URL path segments of the web UI's API.
fn check_name(name: &str) -> Result<()> {
    let valid = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        bail!(
            "UI name `{name}` must be letters, digits, `-` or `_`, starting with a letter or digit"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// URLs

/// An `http(s)://host[:port]/rest` URL, split where butler needs it.
#[derive(Debug, PartialEq, Eq)]
struct Url<'a> {
    scheme: &'a str,
    /// IPv6 hosts keep their brackets.
    host: &'a str,
    port: u16,
    /// Path, query and fragment, verbatim.
    rest: &'a str,
}

impl<'a> Url<'a> {
    fn parse(url: &'a str) -> Result<Self> {
        let (scheme, after) = url
            .split_once("://")
            .ok_or_else(|| eyre!("`{url}` is not an absolute URL"))?;
        let default_port = match scheme {
            "http" => 80,
            "https" => 443,
            _ => bail!("`{url}`: only http:// and https:// UIs can be opened in a browser"),
        };
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let (authority, rest) = after.split_at(end);
        if authority.contains('@') {
            bail!("`{url}`: credentials do not belong in butler.toml");
        }
        let (host, port) = if authority.starts_with('[') {
            let close = authority
                .find(']')
                .ok_or_else(|| eyre!("`{url}`: unterminated IPv6 host"))?;
            let (host, tail) = authority.split_at(close + 1);
            (host, tail.strip_prefix(':'))
        } else {
            match authority.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        if host.is_empty() || host == "[]" {
            bail!("`{url}` has no host");
        }
        let port = match port {
            None | Some("") => default_port,
            Some(p) => match p.parse::<u16>() {
                Ok(p) if p != 0 => p,
                _ => bail!("`{url}`: port `{p}` is not 1-65535"),
            },
        };
        Ok(Self {
            scheme,
            host,
            port,
            rest,
        })
    }

    /// The same URL on another port; always written out explicitly.
    fn with_port(&self, port: u16) -> String {
        format!("{}://{}:{port}{}", self.scheme, self.host, self.rest)
    }

    /// The request target: the path and query, never the fragment.
    fn target(&self) -> &'a str {
        let path = self.rest.split('#').next().unwrap_or_default();
        if path.is_empty() || path.starts_with('?') {
            "/"
        } else {
            path
        }
    }
}

// ---------------------------------------------------------------------------
// Probing

/// Whether a UI answers, and whether a page on another origin may frame it.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub reachable: bool,
    /// HTTP status of the UI's own URL; unset for https, which is only
    /// connected to.
    pub status: Option<u16>,
    /// Unset when unknown (https, or unreachable).
    pub embeddable: Option<bool>,
    /// Why not reachable, or which header refuses framing.
    pub detail: Option<String>,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Connect to the UI, and for http read its response headers: a UI that
/// sends `X-Frame-Options`, or a CSP `frame-ancestors` without `*`, renders
/// blank inside the dashboard's frame and must be opened in its own tab.
pub fn probe(ui: &Ui) -> Probe {
    let unreachable = |detail: String| Probe {
        reachable: false,
        status: None,
        embeddable: None,
        detail: Some(detail),
    };
    // `Ui` came from a validated url.
    let Ok(url) = Url::parse(&ui.url) else {
        return unreachable(format!("invalid url {}", ui.url));
    };
    let host = url.host.trim_start_matches('[').trim_end_matches(']');
    let addrs = match (host, url.port).to_socket_addrs() {
        Ok(addrs) => addrs.collect::<Vec<_>>(),
        Err(e) => return unreachable(format!("resolving {host}: {e}")),
    };
    let Some(mut stream) = addrs
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, PROBE_TIMEOUT).ok())
    else {
        return unreachable(format!("nothing answers on {}:{}", url.host, url.port));
    };
    if url.scheme != "http" {
        return Probe {
            reachable: true,
            status: None,
            embeddable: None,
            detail: None,
        };
    }
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: butler-ui\r\nConnection: close\r\n\r\n",
        url.target(),
        url.host,
        url.port
    );
    let head = stream
        .set_read_timeout(Some(PROBE_TIMEOUT))
        .and_then(|()| stream.write_all(request.as_bytes()))
        .and_then(|()| read_head(&mut stream));
    match head {
        Ok(head) => probe_from_head(&head),
        Err(e) => unreachable(format!("reading the response: {e}")),
    }
}

/// Bytes up to the end of the response headers (capped at 64 KiB).
fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 64 * 1024 {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

fn probe_from_head(head: &str) -> Probe {
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok());
    let mut blocked = None;
    for line in lines.take_while(|l| !l.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("x-frame-options") {
            blocked = Some(format!("X-Frame-Options: {value}"));
        } else if name.eq_ignore_ascii_case("content-security-policy")
            && let Some(ancestors) = value
                .split(';')
                .map(str::trim)
                .find(|d| d.starts_with("frame-ancestors"))
            && !ancestors.split_whitespace().skip(1).any(|s| s == "*")
        {
            blocked = Some(format!("Content-Security-Policy: {ancestors}"));
        }
    }
    Probe {
        reachable: status.is_some(),
        status,
        embeddable: status.map(|_| blocked.is_none()),
        detail: if status.is_none() {
            Some("not an HTTP response".to_string())
        } else {
            blocked
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parts_and_default_ports() {
        let u = Url::parse("http://localhost:55505/#/alerts").unwrap();
        assert_eq!(
            (u.host, u.port, u.rest, u.target()),
            ("localhost", 55505, "/#/alerts", "/")
        );
        let u = Url::parse("https://grafana.example").unwrap();
        assert_eq!((u.host, u.port, u.target()), ("grafana.example", 443, "/"));
        let u = Url::parse("http://[::1]:9090/query?g0.expr=up").unwrap();
        assert_eq!(
            (u.host, u.port, u.target()),
            ("[::1]", 9090, "/query?g0.expr=up")
        );
    }

    #[test]
    fn rejects_what_a_browser_tab_cannot_open() {
        for bad in [
            "localhost:3000",
            "ftp://host/",
            "http://:80/",
            "http://host:0/",
            "http://host:99999/",
            "http://user:pw@host/",
        ] {
            assert!(Url::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn with_port_keeps_path_query_and_fragment() {
        let u = Url::parse("http://localhost/#/alerts?silenced=false").unwrap();
        assert_eq!(
            u.with_port(9093),
            "http://localhost:9093/#/alerts?silenced=false"
        );
    }

    #[test]
    fn names_must_be_toml_keys_and_path_segments() {
        assert!(check_name("kube-prometheus_2").is_ok());
        for bad in ["", "-x", "a/b", "a.b", "a b"] {
            assert!(check_name(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn frame_blocking_headers() {
        let open = probe_from_head("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n");
        assert_eq!((open.status, open.embeddable), (Some(200), Some(true)));

        let xfo = probe_from_head("HTTP/1.1 302 Found\r\nx-frame-options: deny\r\n\r\n");
        assert_eq!(xfo.embeddable, Some(false));
        assert_eq!(xfo.detail.as_deref(), Some("X-Frame-Options: deny"));

        let csp_self = "HTTP/1.1 200 OK\r\nContent-Security-Policy: default-src 'self'; frame-ancestors 'self'\r\n\r\n";
        assert_eq!(probe_from_head(csp_self).embeddable, Some(false));
        let csp_any = "HTTP/1.1 200 OK\r\nContent-Security-Policy: frame-ancestors *\r\n\r\n";
        assert_eq!(probe_from_head(csp_any).embeddable, Some(true));
    }
}
