//! HTML renderers — the Rust twin of the Astro variant's `src/lib/fragments.ts`.
//!
//! Same DOM contract as the Solid island and the Astro htmx page: same class
//! names (libs/ui/todo-theme), same swap anchor. Every mutation endpoint
//! responds with the full list fragment; the client swaps `#todo-list`
//! wholesale (`hx-swap="outerHTML"`) — boring, stateless, always consistent.
//!
//! Feature flags reach the DOM here: `todo_write` decides whether the mutation
//! controls exist at all (the server refuses them regardless), and the status
//! strip renders the resolved set so a demo can watch flags flip per identity.

use std::fmt::Write;

use crate::api::{
    FLAG_MAX_ITEMS, FLAG_WRITE, Flags, KNOWN_FLAGS, PRIORITIES, Priority, Todo, UNLIMITED,
};

/// Minimal HTML entity escaping for text and attribute values.
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Shared htmx attributes: every mutation re-renders the whole list.
const SWAP: &str = r##"hx-target="#todo-list" hx-swap="outerHTML""##;

/// One row. Without `todo_write` the row is read-only: no checkbox, no delete
/// button — the controls are absent, not merely disabled.
pub fn render_todo_item(todo: &Todo, write_enabled: bool) -> String {
    let title = escape_html(&todo.title);
    let done = if todo.completed {
        " todo-item__title--done"
    } else {
        ""
    };
    let priority = todo.priority.as_str();
    let id = escape_html(&todo.id);
    let (checkbox, delete) = if write_enabled {
        let checked = if todo.completed { " checked" } else { "" };
        (
            format!(
                r#"<input class="todo-checkbox" type="checkbox" aria-label="toggle {title}"{checked}
    hx-post="/partials/todos/{id}/toggle" {SWAP} />
  "#
            ),
            format!(
                r#"
  <button class="btn btn--danger" type="button" aria-label="delete {title}"
    hx-delete="/partials/todos/{id}" {SWAP}>Delete</button>"#
            ),
        )
    } else {
        (String::new(), String::new())
    };
    format!(
        r#"<li class="todo-item">
  {checkbox}<span class="todo-item__title{done}">{title}</span>
  <span class="badge badge--{priority}">{priority}</span>{delete}
</li>"#
    )
}

pub fn render_todo_list(todos: &[Todo], write_enabled: bool) -> String {
    let items = todos
        .iter()
        .map(|todo| render_todo_item(todo, write_enabled))
        .collect::<Vec<_>>()
        .join("\n");
    format!("<ul class=\"todo-list\" id=\"todo-list\">\n{items}\n</ul>")
}

/// Human label for the identity shown in the header and prefilled in the form.
fn identity_label(identity: Option<&str>) -> String {
    identity.map_or_else(|| "anonymous".to_owned(), escape_html)
}

/// Identity switcher. `hx-post` for htmx, `action`/`method` so the form also
/// works with JS disabled (the handler answers 303 then).
fn render_identity_form(identity: Option<&str>) -> String {
    let value = identity.map(escape_html).unwrap_or_default();
    format!(
        r#"<form class="todo-form" id="identity-form" action="/identity" method="post"
        hx-post="/identity" hx-swap="none">
        <input class="todo-input" type="text" name="identity" value="{value}"
          placeholder="identity for flag targeting" aria-label="identity" />
        <button class="btn btn--primary" type="submit">Switch user</button>
      </form>"#
    )
}

/// Resolved flag set, one badge each (green on / red off) plus the source.
fn render_flag_strip(flags: &Flags) -> String {
    let mut badges = String::new();
    for flag in KNOWN_FLAGS {
        let enabled = flags.enabled(flag);
        let (tone, state) = if enabled {
            ("low", "on")
        } else {
            ("high", "off")
        };
        let state = match flags.int(flag) {
            Some(UNLIMITED) if flag == FLAG_MAX_ITEMS => "unlimited".to_owned(),
            Some(value) => value.to_string(),
            None => state.to_owned(),
        };
        let _ = write!(
            badges,
            r#"
        <span class="badge badge--{tone}">{flag} {state}</span>"#
        );
    }
    let source = escape_html(flags.source());
    format!(
        r#"<section class="todo-status" id="flag-strip" aria-label="feature flags">{badges}
        <span class="badge badge--sse">source {source}</span>
      </section>"#
    )
}

/// Full page shell: theme + htmx (both served by this binary), the identity
/// switcher, the resolved flags, the create form (only when `todo_write` is on)
/// and the server-rendered initial list. Mirrors htmx.astro.
pub fn render_page(flags: &Flags, identity: Option<&str>, list_html: Option<&str>) -> String {
    let write_enabled = flags.enabled(FLAG_WRITE);
    let create_form = if write_enabled {
        let mut options = String::new();
        for priority in PRIORITIES {
            let value = priority.as_str();
            let selected = if priority == Priority::Medium {
                " selected"
            } else {
                ""
            };
            let _ = write!(
                options,
                r#"<option value="{value}"{selected}>{value}</option>"#
            );
        }
        format!(
            r##"<form
        class="todo-form"
        hx-post="/partials/todos"
        hx-target="#todo-list"
        hx-swap="outerHTML"
        hx-on::after-request="if (event.detail.successful) this.reset()"
      >
        <input
          class="todo-input"
          type="text"
          name="title"
          placeholder="new todo title"
          aria-label="new todo title"
          required
        />
        <select class="todo-select" name="priority" aria-label="priority">{options}</select>
        <button class="btn btn--primary" type="submit">Add</button>
      </form>"##
        )
    } else {
        r#"<p class="todo-status" id="write-disabled">Read-only: <code>todo_write</code> is off.</p>"#
            .to_owned()
    };
    let identity_label = identity_label(identity);
    let identity_form = render_identity_form(identity);
    let flag_strip = render_flag_strip(flags);
    let body =
        list_html.unwrap_or(r#"<p class="todo-error" role="alert">Failed to load todos.</p>"#);
    format!(
        r##"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Todos — axum + htmx</title>
    <link rel="stylesheet" href="/assets/todo.css" />
    <script src="/assets/htmx.min.js" defer></script>
  </head>
  <body>
    <main class="todo-app">
      <header class="todo-app__header">
        <h1 class="todo-app__title">Todos</h1>
        <span class="todo-app__subtitle">axum + htmx</span>
        <span class="todo-app__subtitle" id="identity-label">identity: {identity_label}</span>
      </header>

      {identity_form}

      {flag_strip}

      {create_form}

      {body}
    </main>
  </body>
</html>
"##
    )
}

/// Full page for the 503 kill-switch path: the app itself is flagged off, so
/// there is nothing to render but the reason and the identity switcher (so the
/// visitor can flip back to an identity that has the app).
pub fn render_disabled_page(flag: &str, identity: Option<&str>) -> String {
    let flag = escape_html(flag);
    let identity_label = identity_label(identity);
    let identity_form = render_identity_form(identity);
    format!(
        r##"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Todos — unavailable</title>
    <link rel="stylesheet" href="/assets/todo.css" />
    <script src="/assets/htmx.min.js" defer></script>
  </head>
  <body>
    <main class="todo-app">
      <header class="todo-app__header">
        <h1 class="todo-app__title">Todos</h1>
        <span class="todo-app__subtitle">axum + htmx</span>
        <span class="todo-app__subtitle" id="identity-label">identity: {identity_label}</span>
      </header>

      <p class="todo-error" role="alert" id="flag-disabled">
        This app is disabled by feature flag <code>{flag}</code>.
      </p>

      {identity_form}
    </main>
  </body>
</html>
"##
    )
}

/// Short non-2xx fragment for the partials. htmx does not swap on non-2xx, so
/// this body is only ever seen in devtools — keep it terse but explicit.
pub fn render_disabled_notice(flag: &str) -> String {
    let flag = escape_html(flag);
    format!(
        r#"<p class="todo-error" role="alert">Disabled by feature flag <code>{flag}</code>.</p>"#
    )
}

// The htmx wire format IS these fragments: escaping and the swap contract
// (#todo-list id, hx-target) are load-bearing. Mirrors fragments.test.ts.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{FLAG_APP, SOURCE_DEFAULTS};

    fn todo(title: &str, completed: bool) -> Todo {
        Todo {
            id: "00000000-0000-0000-0000-000000000001".into(),
            title: title.into(),
            completed,
            priority: Priority::Medium,
        }
    }

    fn flags(json: &str) -> Flags {
        serde_json::from_str(json).expect("flag payload parses")
    }

    #[test]
    fn escapes_markup_significant_characters() {
        assert_eq!(
            escape_html(r#"<img src=x onerror="pwn('&')">"#),
            "&lt;img src=x onerror=&quot;pwn(&#39;&amp;&#39;)&quot;&gt;"
        );
    }

    #[test]
    fn escapes_user_titles_in_text_and_attribute_positions() {
        let html = render_todo_item(&todo(r#"<script>"x"</script>"#, false), true);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;&quot;x&quot;&lt;/script&gt;"));
    }

    #[test]
    fn reflects_completion_state() {
        assert!(render_todo_item(&todo("buy milk", true), true).contains("todo-item__title--done"));
        assert!(
            !render_todo_item(&todo("buy milk", false), true).contains("todo-item__title--done")
        );
    }

    #[test]
    fn targets_the_list_swap_on_every_mutation_control() {
        let html = render_todo_item(&todo("buy milk", false), true);
        assert!(
            html.contains(
                r#"hx-post="/partials/todos/00000000-0000-0000-0000-000000000001/toggle""#
            )
        );
        assert!(
            html.contains(r#"hx-delete="/partials/todos/00000000-0000-0000-0000-000000000001""#)
        );
        assert_eq!(html.matches(r##"hx-target="#todo-list""##).count(), 2);
    }

    #[test]
    fn renders_the_swap_anchor_with_all_items() {
        let html = render_todo_list(&[todo("buy milk", false), todo("other", false)], true);
        assert!(html.contains(r#"id="todo-list""#));
        assert_eq!(html.matches(r#"<li class="todo-item">"#).count(), 2);
    }

    #[test]
    fn page_wires_form_to_the_list_swap() {
        let html = render_page(&Flags::defaults(), None, Some("<ul id=\"todo-list\"></ul>"));
        assert!(html.contains(r#"hx-post="/partials/todos""#));
        assert!(html.contains(r##"hx-target="#todo-list""##));
        assert!(html.contains(r#"<option value="medium" selected>"#));
        assert!(render_page(&Flags::defaults(), None, None).contains("todo-error"));
    }

    #[test]
    fn read_only_list_drops_the_mutation_controls_but_keeps_the_items() {
        let html = render_todo_list(&[todo("buy milk", false), todo("walk dog", true)], false);
        assert!(!html.contains("todo-checkbox"));
        assert!(!html.contains("btn--danger"));
        assert!(!html.contains("hx-delete"));
        assert!(!html.contains("hx-post"));
        assert!(html.contains(">buy milk<"));
        assert!(html.contains(">walk dog<"));
        assert!(html.contains("todo-item__title--done"));
        assert_eq!(html.matches(r#"<li class="todo-item">"#).count(), 2);
        assert!(html.contains(r#"id="todo-list""#));
    }

    #[test]
    fn read_only_page_hides_the_add_form() {
        let flags = flags(r#"{"source":"remote","flags":{"todo_write":{"enabled":false}}}"#);
        let html = render_page(&flags, Some("yuri"), Some("<ul id=\"todo-list\"></ul>"));
        assert!(!html.contains(r#"hx-post="/partials/todos""#));
        assert!(!html.contains(r#"name="title""#));
        assert!(html.contains(r#"id="write-disabled""#));
        // The identity switcher survives: it is how you flip the flag back.
        assert!(html.contains(r#"hx-post="/identity""#));
    }

    #[test]
    fn disabled_page_names_the_flag_and_keeps_the_switcher() {
        let html = render_disabled_page(FLAG_APP, Some("yuri"));
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains(r#"id="flag-disabled""#));
        assert!(html.contains("<code>todo_app_htmx</code>"));
        assert!(html.contains(r#"hx-post="/identity""#));
        assert!(html.contains("identity: yuri"));
        assert!(!html.contains(r#"id="todo-list""#));
    }

    #[test]
    fn disabled_notice_is_a_short_fragment() {
        let html = render_disabled_notice(FLAG_WRITE);
        assert!(html.starts_with(r#"<p class="todo-error" role="alert">"#));
        assert!(html.contains("<code>todo_write</code>"));
    }

    #[test]
    fn identity_form_shows_the_current_identity_escaped() {
        let html = render_page(&Flags::defaults(), Some(r#"a"><b"#), None);
        assert!(html.contains(r#"value="a&quot;&gt;&lt;b""#));
        assert!(html.contains("identity: a&quot;&gt;&lt;b"));
        assert!(!html.contains(r#"value="a"><b""#));
        // No-JS fallback path is present alongside hx-post.
        assert!(html.contains(r#"action="/identity" method="post""#));
    }

    #[test]
    fn identity_form_is_empty_and_labelled_anonymous_without_identity() {
        let html = render_page(&Flags::defaults(), None, None);
        assert!(html.contains(r#"name="identity" value=""#));
        assert!(html.contains("identity: anonymous"));
    }

    #[test]
    fn flag_strip_reports_every_flag_and_the_source() {
        let remote = flags(
            r#"{"identity":"yuri","source":"remote","flags":{
                 "todo_app_web":{"enabled":true,"value":null},
                 "todo_app_htmx":{"enabled":true,"value":null},
                 "todo_app_astro":{"enabled":true,"value":null},
                 "todo_realtime":{"enabled":false,"value":null},
                 "todo_write":{"enabled":true,"value":null},
                 "todo_max_items":{"enabled":true,"value":"3"}}}"#,
        );
        let html = render_flag_strip(&remote);
        for flag in KNOWN_FLAGS {
            assert!(html.contains(flag), "{flag} missing from the strip");
        }
        assert!(html.contains("todo_realtime off"));
        assert!(html.contains("badge--high"));
        assert!(html.contains("todo_write on"));
        assert!(html.contains("todo_max_items 3"));
        assert!(html.contains("source remote"));
        assert!(!html.contains(SOURCE_DEFAULTS));
    }

    #[test]
    fn flag_strip_reports_the_degraded_source() {
        let html = render_flag_strip(&Flags::defaults());
        assert!(html.contains("source defaults"));
        assert!(html.contains("todo_max_items unlimited"));
        assert!(!html.contains("badge--high"));
        assert!(html.contains("todo_app_htmx on"));
    }
}
