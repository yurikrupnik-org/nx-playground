//! The TCP ports listening on this machine, from `lsof`.
//!
//! `lsof` is the one tool that names the owning process on both macOS and
//! Linux. Unprivileged, it sees only the current user's processes — which is
//! where port-forwards, dev servers and Docker's published ports live.

use std::collections::BTreeMap;
use std::process::Command;

use eyre::{Result, WrapErr, bail};
use serde::Serialize;

use super::Ui;

/// One process listening on one port, on every address it bound.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsedPort {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    pub user: String,
    /// `*` (every interface), `127.0.0.1`, `[::1]`, ...
    pub addresses: Vec<String>,
    /// Registered UIs a browser reaches through this listener.
    pub uis: Vec<String>,
}

/// Every listening TCP port, sorted by port then pid, joined with the UIs
/// that point at it.
pub fn used(uis: &[Ui]) -> Result<Vec<UsedPort>> {
    // `+c 0`: full command names instead of lsof's 9-character default.
    let out = Command::new("lsof")
        .args(["+c", "0", "-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcLn"])
        .output()
        .wrap_err("running lsof (needed to list used ports)")?;
    // lsof exits 1 when nothing matches, which is not an error here.
    if !out.status.success() && !out.stderr.is_empty() {
        bail!(
            "lsof failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut ports = parse_lsof(&String::from_utf8_lossy(&out.stdout));
    for p in &mut ports {
        p.uis = uis
            .iter()
            .filter(|ui| {
                ui.local && ui.port == p.port && p.addresses.iter().any(|a| serves_loopback(a))
            })
            .map(|ui| ui.name.clone())
            .collect();
    }
    Ok(ports)
}

/// Whether a listener bound to `address` answers `localhost`.
fn serves_loopback(address: &str) -> bool {
    address == "*" || address == "[::1]" || address.starts_with("127.")
}

/// `lsof -F pcLn`: a `p<pid>` record, then its `c<command>` and `L<user>`,
/// then per descriptor `f<fd>` and `n<address>:<port>`.
fn parse_lsof(raw: &str) -> Vec<UsedPort> {
    let mut by_key: BTreeMap<(u16, u32), UsedPort> = BTreeMap::new();
    let (mut pid, mut command, mut user) = (0u32, String::new(), String::new());
    for line in raw.lines() {
        let Some(field) = line.chars().next() else {
            continue;
        };
        let value = &line[field.len_utf8()..];
        match field {
            'p' => {
                pid = value.parse().unwrap_or(0);
                command.clear();
                user.clear();
            }
            'c' => command = value.to_string(),
            'L' => user = value.to_string(),
            'n' => {
                let Some((address, port)) = value.rsplit_once(':') else {
                    continue;
                };
                let Ok(port) = port.parse::<u16>() else {
                    continue;
                };
                let entry = by_key.entry((port, pid)).or_insert_with(|| UsedPort {
                    port,
                    pid,
                    command: command.clone(),
                    user: user.clone(),
                    addresses: Vec::new(),
                    uis: Vec::new(),
                });
                if !entry.addresses.iter().any(|a| a == address) {
                    entry.addresses.push(address.to_string());
                }
            }
            _ => {}
        }
    }
    by_key.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_descriptors_per_process_and_port() {
        let raw = "p682\ncControlCenter\nLme\nf9\nn*:7000\nf10\nn*:7000\nf11\nn*:5000\n\
                   p93\ncbutler\nLme\nf9\nn127.0.0.1:7878\nf10\nn[::1]:7878\n";
        let ports = parse_lsof(raw);
        let summary: Vec<_> = ports
            .iter()
            .map(|p| (p.port, p.pid, p.command.as_str(), p.addresses.join(",")))
            .collect();
        assert_eq!(
            summary,
            [
                (5000, 682, "ControlCenter", "*".to_string()),
                (7000, 682, "ControlCenter", "*".to_string()),
                (7878, 93, "butler", "127.0.0.1,[::1]".to_string()),
            ]
        );
    }

    #[test]
    fn only_loopback_reachable_listeners_serve_localhost() {
        assert!(serves_loopback("*"));
        assert!(serves_loopback("127.0.0.1"));
        assert!(serves_loopback("[::1]"));
        assert!(!serves_loopback("192.168.1.5"));
    }
}
