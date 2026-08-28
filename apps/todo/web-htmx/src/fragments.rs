//! HTML renderers — the Rust twin of the Astro variant's `src/lib/fragments.ts`.
//!
//! Same DOM contract as the Solid island and the Astro htmx page: same class
//! names (libs/ui/todo-theme), same swap anchor. Every mutation endpoint
//! responds with the full list fragment; the client swaps `#todo-list`
//! wholesale (`hx-swap="outerHTML"`) — boring, stateless, always consistent.

use std::fmt::Write;

use crate::api::{Priority, Todo, PRIORITIES};

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

pub fn render_todo_item(todo: &Todo) -> String {
    let title = escape_html(&todo.title);
    let done = if todo.completed {
        " todo-item__title--done"
    } else {
        ""
    };
    let checked = if todo.completed { " checked" } else { "" };
    let priority = todo.priority.as_str();
    let id = escape_html(&todo.id);
    format!(
        r#"<li class="todo-item">
  <input class="todo-checkbox" type="checkbox" aria-label="toggle {title}"{checked}
    hx-post="/partials/todos/{id}/toggle" {SWAP} />
  <span class="todo-item__title{done}">{title}</span>
  <span class="badge badge--{priority}">{priority}</span>
  <button class="btn btn--danger" type="button" aria-label="delete {title}"
    hx-delete="/partials/todos/{id}" {SWAP}>Delete</button>
</li>"#
    )
}

pub fn render_todo_list(todos: &[Todo]) -> String {
    let items = todos
        .iter()
        .map(render_todo_item)
        .collect::<Vec<_>>()
        .join("\n");
    format!("<ul class=\"todo-list\" id=\"todo-list\">\n{items}\n</ul>")
}

/// Full page shell: theme + htmx (both served by this binary), the create
/// form, and the server-rendered initial list. Mirrors htmx.astro.
pub fn render_page(list_html: Option<&str>) -> String {
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
      </header>

      <form
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
      </form>

      {body}
    </main>
  </body>
</html>
"##
    )
}

// The htmx wire format IS these fragments: escaping and the swap contract
// (#todo-list id, hx-target) are load-bearing. Mirrors fragments.test.ts.
#[cfg(test)]
mod tests {
    use super::*;

    fn todo(title: &str, completed: bool) -> Todo {
        Todo {
            id: "00000000-0000-0000-0000-000000000001".into(),
            title: title.into(),
            completed,
            priority: Priority::Medium,
        }
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
        let html = render_todo_item(&todo(r#"<script>"x"</script>"#, false));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;&quot;x&quot;&lt;/script&gt;"));
    }

    #[test]
    fn reflects_completion_state() {
        assert!(render_todo_item(&todo("buy milk", true)).contains("todo-item__title--done"));
        assert!(!render_todo_item(&todo("buy milk", false)).contains("todo-item__title--done"));
    }

    #[test]
    fn targets_the_list_swap_on_every_mutation_control() {
        let html = render_todo_item(&todo("buy milk", false));
        assert!(html
            .contains(r#"hx-post="/partials/todos/00000000-0000-0000-0000-000000000001/toggle""#));
        assert!(
            html.contains(r#"hx-delete="/partials/todos/00000000-0000-0000-0000-000000000001""#)
        );
        assert_eq!(html.matches(r##"hx-target="#todo-list""##).count(), 2);
    }

    #[test]
    fn renders_the_swap_anchor_with_all_items() {
        let html = render_todo_list(&[todo("buy milk", false), todo("other", false)]);
        assert!(html.contains(r#"id="todo-list""#));
        assert_eq!(html.matches(r#"<li class="todo-item">"#).count(), 2);
    }

    #[test]
    fn page_wires_form_to_the_list_swap() {
        let html = render_page(Some("<ul id=\"todo-list\"></ul>"));
        assert!(html.contains(r#"hx-post="/partials/todos""#));
        assert!(html.contains(r##"hx-target="#todo-list""##));
        assert!(html.contains(r#"<option value="medium" selected>"#));
        assert!(render_page(None).contains("todo-error"));
    }
}
