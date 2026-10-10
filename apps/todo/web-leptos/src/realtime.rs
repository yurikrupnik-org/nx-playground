//! Realtime todo changes, sourced from the database — the Rust twin of
//! `apps/todo/web/src/lib/realtime.ts`.
//!
//! The list is driven by a Postgres `NOTIFY` trigger (`todos_notify`) →
//! `domain_todo::db_events` → SSE/WebSocket, so it reflects writes made by
//! anything else touching the table (another replica, todo-worker, the CLI,
//! `psql`) — not an in-process tee. See `docs/realtime-todo.md`.
//!
//! Two transports against the same broadcast bus, exactly as the Solid app
//! demonstrates them: SSE (`/api/events/sse`, one named event per lifecycle
//! kind) and WebSocket (`/api/events/ws`, the same events as text frames plus a
//! bidirectional echo).

use std::rc::Rc;

use uuid::Uuid;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{EventSource, MessageEvent, WebSocket};

use crate::dto::{TODO_EVENT_KINDS, Todo, TodoEvent, TodoEventKind};

/// Insert or replace `todo` by id, keeping the list ordered newest-first (the
/// API's `created_at DESC`).
///
/// Idempotent: replaying the same todo must not duplicate a row, which matters
/// because a reconnect refetch can overlap with in-flight events.
pub fn upsert_todo(todos: &mut Vec<Todo>, todo: Todo) {
    match todos.iter().position(|existing| existing.id == todo.id) {
        Some(index) => todos[index] = todo,
        None => {
            todos.push(todo);
            todos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        }
    }
}

/// Drop the row with `id`, if present.
pub fn remove_todo(todos: &mut Vec<Todo>, id: Uuid) {
    todos.retain(|todo| todo.id != id);
}

/// Apply one change event to the cached list.
///
/// `Deleted` drops the row; every other kind upserts the event's snapshot.
/// A non-delete event that somehow carries no snapshot is ignored rather than
/// rendering a hole.
pub fn apply_event(todos: &mut Vec<Todo>, event: TodoEvent) {
    if event.kind == TodoEventKind::Deleted {
        remove_todo(todos, event.todo_id);
        return;
    }
    if let Some(todo) = event.todo {
        upsert_todo(todos, todo);
    }
}

/// `EventSource` cannot set headers, so the identity travels as a query param —
/// the third resolution order todo-api accepts.
fn sse_url(identity: &str) -> String {
    format!("/api/events/sse?identity={}", encode(identity))
}

/// Same identity plumbing for the WebSocket transport.
fn ws_url(identity: &str) -> String {
    let location = web_sys::window().expect("window").location();
    let protocol = match location.protocol().as_deref() {
        Ok("https:") => "wss:",
        _ => "ws:",
    };
    let host = location.host().unwrap_or_default();
    format!(
        "{protocol}//{host}/api/events/ws?identity={}",
        encode(identity)
    )
}

/// `encodeURIComponent` for the identity grammar (`[A-Za-z0-9_.@-]`): only `@`
/// is not already URL-safe, so the whole of `encodeURIComponent` is one branch.
fn encode(identity: &str) -> String {
    identity.replace('@', "%40")
}

/// Everything the page wants to hear from the two streams.
pub struct Handlers {
    /// A decoded change event, from either transport. `bool` is `true` for SSE.
    pub on_event: Rc<dyn Fn(TodoEvent, bool)>,
    /// SSE opened. Carries the resync obligation: `NOTIFY` has no backlog, so
    /// whatever committed while the stream was down must be refetched.
    pub on_sse_open: Rc<dyn Fn()>,
    pub on_sse_error: Rc<dyn Fn()>,
    pub on_ws_open: Rc<dyn Fn()>,
    pub on_ws_close: Rc<dyn Fn()>,
    /// A WebSocket text frame that is not a change event — the `echo: ` reply.
    pub on_ws_text: Rc<dyn Fn(String)>,
}

/// One SSE stream plus one WebSocket, owned together.
///
/// Dropping closes both and frees the JS closures — which is how an identity
/// switch reconnects as the new user, since flags are evaluated per identity.
/// (The Solid app gets the same lifetime from its effect's cleanup.)
pub struct Connection {
    source: EventSource,
    socket: WebSocket,
    _sse_open: Closure<dyn FnMut()>,
    _sse_error: Closure<dyn FnMut()>,
    _sse_message: Closure<dyn FnMut(MessageEvent)>,
    _ws_open: Closure<dyn FnMut()>,
    _ws_close: Closure<dyn FnMut()>,
    _ws_message: Closure<dyn FnMut(MessageEvent)>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.source.close();
        let _ = self.socket.close();
    }
}

impl Connection {
    /// Send a text frame; `false` when the socket is not open (the Solid app
    /// checks `readyState !== WebSocket.OPEN` for the same reason).
    pub fn send(&self, text: &str) -> bool {
        self.socket.ready_state() == WebSocket::OPEN && self.socket.send_with_str(text).is_ok()
    }

    pub fn open(identity: &str, handlers: Handlers) -> Option<Self> {
        let source = EventSource::new(&sse_url(identity)).ok()?;
        let socket = WebSocket::new(&ws_url(identity)).ok()?;

        let on_open = handlers.on_sse_open.clone();
        let sse_open = Closure::<dyn FnMut()>::new(move || on_open());
        source.set_onopen(Some(sse_open.as_ref().unchecked_ref()));

        let on_error = handlers.on_sse_error.clone();
        let sse_error = Closure::<dyn FnMut()>::new(move || on_error());
        source.set_onerror(Some(sse_error.as_ref().unchecked_ref()));

        // One closure, registered for all five named events: the payload is the
        // same shape and a malformed frame must not tear the stream down.
        let on_event = handlers.on_event.clone();
        let sse_message = Closure::<dyn FnMut(MessageEvent)>::new(move |message: MessageEvent| {
            let Some(text) = message.data().as_string() else {
                return;
            };
            if let Ok(event) = serde_json::from_str::<TodoEvent>(&text) {
                on_event(event, true);
            }
        });
        for kind in TODO_EVENT_KINDS {
            source
                .add_event_listener_with_callback(kind, sse_message.as_ref().unchecked_ref())
                .ok()?;
        }

        let on_ws_open = handlers.on_ws_open.clone();
        let ws_open = Closure::<dyn FnMut()>::new(move || on_ws_open());
        socket.set_onopen(Some(ws_open.as_ref().unchecked_ref()));

        let on_ws_close = handlers.on_ws_close.clone();
        let ws_close = Closure::<dyn FnMut()>::new(move || on_ws_close());
        socket.set_onclose(Some(ws_close.as_ref().unchecked_ref()));

        let on_event = handlers.on_event.clone();
        let on_ws_text = handlers.on_ws_text.clone();
        let ws_message = Closure::<dyn FnMut(MessageEvent)>::new(move |message: MessageEvent| {
            let Some(text) = message.data().as_string() else {
                return;
            };
            if text.starts_with("echo: ") {
                on_ws_text(text);
                return;
            }
            if let Ok(event) = serde_json::from_str::<TodoEvent>(&text) {
                on_event(event, false);
            }
        });
        socket.set_onmessage(Some(ws_message.as_ref().unchecked_ref()));

        Some(Self {
            source,
            socket,
            _sse_open: sse_open,
            _sse_error: sse_error,
            _sse_message: sse_message,
            _ws_open: ws_open,
            _ws_close: ws_close,
            _ws_message: ws_message,
        })
    }
}
