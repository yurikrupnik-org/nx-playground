//! Todo hypermedia frontend: axum + htmx.
//!
//! The third variant of the same UI (Solid island / Astro+htmx / this):
//! identical DOM contract, but here axum renders the HTML fragments itself —
//! no JS runtime on the server. Talks to todo-api over JSON exactly like the
//! Astro variant; ships zero app JS to the browser (htmx only, self-hosted).
//!
//! Single static binary: theme CSS and htmx.min.js are embedded at compile
//! time (`assets/htmx.min.js` is vendored from htmx.org@2.0.10 — bun installs
//! node_modules app-locally, which doesn't exist in cargo/docker builds).
//!
//! Feature flags come from todo-api (`GET /api/flags`) per identity: this app
//! never talks to Flagsmith. Identity lives in the `todo_identity` cookie and
//! is forwarded upstream on every call. A broken flag source degrades to
//! all-ON defaults, never to a broken page.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Form, Router};
use axum_helpers::create_app;
use core_config::{FromEnv, app_info};
use eyre::{Result, WrapErr};
use serde::Deserialize;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

use crate::api::{CreateTodo, FLAG_APP, FLAG_WRITE, Flags, Priority, TodoApi};
use crate::config::AppConfig;
use crate::fragments::{
    render_disabled_notice, render_disabled_page, render_page, render_todo_list,
};

mod api;
mod config;
mod fragments;

const THEME_CSS: &str = include_str!("../../../../libs/ui/todo-theme/todo.css");
const HTMX_JS: &str = include_str!("../assets/htmx.min.js");

/// Cookie carrying the Flagsmith identity (same name in the Astro variant).
const IDENTITY_COOKIE: &str = "todo_identity";
/// One year, matching the other frontends' persistence.
const IDENTITY_MAX_AGE: u32 = 31_536_000;
/// Longest accepted identity (todo-api applies the same bound).
const IDENTITY_MAX_LEN: usize = 64;
/// htmx marks its own requests; without it the identity form is a plain POST.
const HX_REQUEST: &str = "hx-request";

/// Short-circuit responses (app disabled, writes disabled) travel as `Err`;
/// axum renders either arm, so handlers can use `?` on the gates.
type Handler = Result<Response, Response>;

#[tokio::main]
async fn main() -> Result<()> {
    let config = AppConfig::from_env()?;
    let _tracing_guard = core_config::tracing::init_tracing(&config.environment, app_info!());

    let api = TodoApi::new(config.todo_api_url.clone());
    info!(todo_api_url = %config.todo_api_url, "rendering fragments over todo-api");

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(|| async { "ok" }))
        .route("/identity", post(set_identity))
        .route("/assets/todo.css", get(theme_css))
        .route("/assets/htmx.min.js", get(htmx_js))
        .route("/partials/todos", get(list_todos).post(create_todo))
        .route("/partials/todos/{id}", delete(delete_todo))
        .route("/partials/todos/{id}/toggle", post(toggle_todo))
        .with_state(api)
        .layer(TraceLayer::new_for_http());

    info!(addr = %config.server.addr(), "todo-web-htmx listening");
    create_app(app, &config.server)
        .await
        .wrap_err("server error")?;

    Ok(())
}

/// Standard fragment response (htmx swaps on 2xx only).
fn fragment(html: String, status: StatusCode) -> Response {
    (status, Html(html)).into_response()
}

/// Upstream failure: non-2xx text so htmx leaves the current DOM intact.
fn upstream_error(err: eyre::Report) -> Response {
    error!(error = %err, "todo-api call failed");
    (StatusCode::BAD_GATEWAY, "todo-api unavailable").into_response()
}

/// Identity charset shared with todo-api: `[A-Za-z0-9_.@-]{1,64}`.
fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= IDENTITY_MAX_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'@' | b'-'))
}

/// Identity from the `todo_identity` cookie. Anything malformed is treated as
/// absent — todo-api would reject it anyway, and anonymous still works.
///
/// Hand-rolled because `axum-extra` is vendored without the `cookie` feature;
/// one header, one name, no need for a cookie jar.
fn identity_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|raw| raw.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| name.trim() == IDENTITY_COOKIE)
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| valid_identity(value))
}

/// `Set-Cookie` value: persist a valid identity, clear the cookie otherwise.
fn identity_cookie(identity: &str) -> String {
    if valid_identity(identity) {
        format!("{IDENTITY_COOKIE}={identity}; Path=/; SameSite=Lax; Max-Age={IDENTITY_MAX_AGE}")
    } else {
        format!("{IDENTITY_COOKIE}=; Path=/; SameSite=Lax; Max-Age=0")
    }
}

/// Per-request preamble: identity + resolved flags, or the app kill switch.
/// `Err` short-circuits with the caller's flavour of 503.
async fn gate(
    api: &TodoApi,
    headers: &HeaderMap,
    page: bool,
) -> Result<(Option<String>, Flags), Response> {
    let identity = identity_from_headers(headers);
    let flags = api.flags_or_defaults(identity.as_deref()).await;
    if flags.enabled(FLAG_APP) {
        return Ok((identity, flags));
    }
    let body = if page {
        render_disabled_page(FLAG_APP, identity.as_deref())
    } else {
        render_disabled_notice(FLAG_APP)
    };
    Err(fragment(body, StatusCode::SERVICE_UNAVAILABLE))
}

/// Mutations need `todo_write`: `Some(403)` when off, and the caller returns it
/// without ever touching the upstream API.
fn write_denied(flags: &Flags) -> Option<Response> {
    if flags.enabled(FLAG_WRITE) {
        return None;
    }
    Some(fragment(
        render_disabled_notice(FLAG_WRITE),
        StatusCode::FORBIDDEN,
    ))
}

/// Full page; initial list server-rendered, load failure degrades to an alert.
async fn index(State(api): State<TodoApi>, headers: HeaderMap) -> Handler {
    let (identity, flags) = gate(&api, &headers, true).await?;
    let list_html = match api.list(identity.as_deref()).await {
        Ok(todos) => Some(render_todo_list(&todos, flags.enabled(FLAG_WRITE))),
        Err(err) => {
            error!(error = %err, "initial list failed");
            None
        }
    };
    Ok(Html(render_page(
        &flags,
        identity.as_deref(),
        list_html.as_deref(),
    ))
    .into_response())
}

/// The identity switcher target. htmx gets `HX-Refresh` (the whole page depends
/// on the flags, so a full reload is the honest swap); a JS-less form post gets
/// a 303 back to `/`.
async fn set_identity(headers: HeaderMap, Form(form): Form<IdentityForm>) -> Response {
    let cookie = identity_cookie(form.identity.trim());
    if headers.contains_key(HX_REQUEST) {
        return (
            StatusCode::NO_CONTENT,
            [
                (header::SET_COOKIE.as_str(), cookie.as_str()),
                ("hx-refresh", "true"),
            ],
        )
            .into_response();
    }
    (
        [(header::SET_COOKIE.as_str(), cookie.as_str())],
        Redirect::to("/"),
    )
        .into_response()
}

/// The identity switcher body (`<input name="identity">`).
#[derive(Debug, Deserialize)]
struct IdentityForm {
    #[serde(default)]
    identity: String,
}

async fn theme_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        THEME_CSS,
    )
}

async fn htmx_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        HTMX_JS,
    )
}

async fn render_list(api: &TodoApi, identity: Option<&str>, flags: &Flags) -> Response {
    match api.list(identity).await {
        Ok(todos) => fragment(
            render_todo_list(&todos, flags.enabled(FLAG_WRITE)),
            StatusCode::OK,
        ),
        Err(err) => upstream_error(err),
    }
}

async fn list_todos(State(api): State<TodoApi>, headers: HeaderMap) -> Handler {
    let (identity, flags) = gate(&api, &headers, false).await?;
    Ok(render_list(&api, identity.as_deref(), &flags).await)
}

/// The form-encoded body htmx submits (title, priority).
#[derive(Debug, Deserialize)]
struct CreateForm {
    #[serde(default)]
    title: String,
    #[serde(default)]
    priority: Option<String>,
}

async fn create_todo(
    State(api): State<TodoApi>,
    headers: HeaderMap,
    Form(form): Form<CreateForm>,
) -> Handler {
    let (identity, flags) = gate(&api, &headers, false).await?;
    if let Some(denied) = write_denied(&flags) {
        return Ok(denied);
    }

    let title = form.title.trim();
    let priority = match form.priority.as_deref() {
        None => Some(Priority::Medium),
        Some(value) => Priority::parse(value),
    };
    let (Some(priority), false) = (priority, title.is_empty()) else {
        return Ok(fragment(
            "invalid todo".into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    };

    let input = CreateTodo {
        title: title.to_owned(),
        description: String::new(),
        priority,
    };
    if let Err(err) = api.create(&input, identity.as_deref()).await {
        return Ok(upstream_error(err));
    }
    Ok(render_list(&api, identity.as_deref(), &flags).await)
}

/// Flip completion; current state is read from the API so the toggle is
/// idempotent per rendered state.
async fn toggle_todo(
    State(api): State<TodoApi>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Handler {
    let (identity, flags) = gate(&api, &headers, false).await?;
    if let Some(denied) = write_denied(&flags) {
        return Ok(denied);
    }

    let todo = match api.get(&id, identity.as_deref()).await {
        Ok(Some(todo)) => todo,
        Ok(None) => return Ok(fragment("unknown todo".into(), StatusCode::NOT_FOUND)),
        Err(err) => return Ok(upstream_error(err)),
    };
    if let Err(err) = api.toggle(&id, todo.completed, identity.as_deref()).await {
        return Ok(upstream_error(err));
    }
    Ok(render_list(&api, identity.as_deref(), &flags).await)
}

async fn delete_todo(
    State(api): State<TodoApi>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Handler {
    let (identity, flags) = gate(&api, &headers, false).await?;
    if let Some(denied) = write_denied(&flags) {
        return Ok(denied);
    }

    if let Err(err) = api.remove(&id, identity.as_deref()).await {
        return Ok(upstream_error(err));
    }
    Ok(render_list(&api, identity.as_deref(), &flags).await)
}

// Identity plumbing is the one piece of request parsing this binary owns: a
// malformed cookie must read as anonymous, never as a rejected request.
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(cookies: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for cookie in cookies {
            map.append(
                header::COOKIE,
                HeaderValue::from_str(cookie).expect("test cookie is a valid header value"),
            );
        }
        map
    }

    #[test]
    fn accepts_the_documented_identity_charset() {
        assert!(valid_identity("yuri"));
        assert!(valid_identity("anon-0123abcd"));
        assert!(valid_identity("first.last@example.com"));
        assert!(valid_identity("A_b-9.@"));
        assert!(valid_identity(&"x".repeat(IDENTITY_MAX_LEN)));
    }

    #[test]
    fn rejects_empty_oversized_and_out_of_charset_identities() {
        assert!(!valid_identity(""));
        assert!(!valid_identity(&"x".repeat(IDENTITY_MAX_LEN + 1)));
        assert!(!valid_identity("has space"));
        assert!(!valid_identity("semi;colon"));
        assert!(!valid_identity("quote\"quote"));
        assert!(!valid_identity("naïve"));
        assert!(!valid_identity("slash/slash"));
    }

    #[test]
    fn reads_the_identity_cookie() {
        assert_eq!(
            identity_from_headers(&headers(&["todo_identity=yuri"])).as_deref(),
            Some("yuri")
        );
    }

    #[test]
    fn picks_the_identity_out_of_a_multi_cookie_header() {
        let map = headers(&["theme=dark; todo_identity=yuri ; other=1"]);
        assert_eq!(identity_from_headers(&map).as_deref(), Some("yuri"));
    }

    #[test]
    fn reads_across_repeated_cookie_headers() {
        let map = headers(&["theme=dark", "todo_identity=other.user"]);
        assert_eq!(identity_from_headers(&map).as_deref(), Some("other.user"));
    }

    #[test]
    fn treats_a_malformed_identity_as_absent() {
        assert_eq!(
            identity_from_headers(&headers(&["todo_identity=has space"])),
            None
        );
        assert_eq!(identity_from_headers(&headers(&["todo_identity="])), None);
        let long = format!("todo_identity={}", "x".repeat(IDENTITY_MAX_LEN + 1));
        assert_eq!(identity_from_headers(&headers(&[long.as_str()])), None);
    }

    #[test]
    fn missing_cookie_is_anonymous() {
        assert_eq!(identity_from_headers(&HeaderMap::new()), None);
        assert_eq!(identity_from_headers(&headers(&["theme=dark"])), None);
        assert_eq!(
            identity_from_headers(&headers(&["not_todo_identity=yuri"])),
            None
        );
    }

    #[test]
    fn cookie_persists_valid_identities_and_clears_invalid_ones() {
        assert_eq!(
            identity_cookie("yuri"),
            "todo_identity=yuri; Path=/; SameSite=Lax; Max-Age=31536000"
        );
        assert_eq!(
            identity_cookie(""),
            "todo_identity=; Path=/; SameSite=Lax; Max-Age=0"
        );
        assert_eq!(
            identity_cookie("bad value"),
            "todo_identity=; Path=/; SameSite=Lax; Max-Age=0"
        );
    }

    /// A cookie set by this app must read back identically.
    #[test]
    fn set_cookie_round_trips_through_the_reader() {
        let cookie = identity_cookie("first.last@example.com");
        let value = cookie.split(';').next().expect("cookie has a name=value");
        assert_eq!(
            identity_from_headers(&headers(&[value])).as_deref(),
            Some("first.last@example.com")
        );
    }
}
