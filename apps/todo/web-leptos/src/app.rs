//! The one route — the Rust twin of `apps/todo/web/src/todo-app.tsx` (`/`).
//!
//! Parity scope is the Solid SPA's `/` route ONLY: TanStack Query + signals,
//! feature flags, identity switching, database-sourced realtime. The `/xstate`
//! and `/effect` comparison routes are deliberately absent, and so is the
//! router that serves them — see README.md.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::dto::{CreateTodo, PRIORITIES, Todo, TodoEvent, TodoPriority};
use crate::event_feed::{CLOSED, CONNECTING, EventFeed, FeedItem, MAX_ITEMS, OPEN, Status};
use crate::flags::{FLAG_APP, FLAG_MAX_ITEMS, FLAG_REALTIME, FLAG_WRITE, FlagsResponse, UNLIMITED};
use crate::identity;
use crate::realtime::{Connection, Handlers, apply_event, upsert_todo};

thread_local! {
    /// The live SSE + WebSocket pair, module-scoped exactly as in
    /// `realtime.ts`: one connection shared by the list and the feed, replaced
    /// when the identity changes and dropped (closed) when realtime is off.
    static CONNECTION: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

/// Write a text frame to the shared socket; `false` when it is not open.
pub fn send_over_socket(text: &str) -> bool {
    CONNECTION.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_some_and(|connection| connection.send(text))
    })
}

/// What the todo list is doing. Mirrors the three `todosQuery` states the Solid
/// app renders; "not started yet" is `Pending` there too, because a disabled
/// TanStack query with no data reports `isPending`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Load {
    Pending,
    Ready,
    Error,
}

#[component]
pub fn App() -> impl IntoView {
    let identity = RwSignal::new(identity::get());
    let flags = RwSignal::new(FlagsResponse::defaults());
    let flags_fetched = RwSignal::new(false);

    let todos = RwSignal::new(Vec::<Todo>::new());
    let load = RwSignal::new(Load::Pending);
    // Bumped to re-run the list fetch — the equivalent of
    // `queryClient.invalidateQueries({ queryKey: TODOS_KEY })`.
    let todos_epoch = RwSignal::new(0u32);
    let mutation_error = RwSignal::new(None::<String>);

    let title = RwSignal::new(String::new());
    let priority = RwSignal::new(TodoPriority::default());

    let feed = RwSignal::new(Vec::<FeedItem>::new());
    let feed_seq = RwSignal::new(0u32);
    let sse_status = RwSignal::new(CONNECTING as Status);
    let ws_status = RwSignal::new(CONNECTING as Status);

    // Rendering gates fail open on the built-in defaults, so a slow or broken
    // /api/flags shows the working UI instead of an empty page.
    let app_enabled = move || flags.with(|f| f.is_enabled(FLAG_APP));
    let write_enabled = move || flags.with(|f| f.is_enabled(FLAG_WRITE));
    let max_items = move || flags.with(|f| f.int_value(FLAG_MAX_ITEMS, UNLIMITED));
    let flag_source = move || flags.with(|f| f.source.clone());
    // Connections are the exception: opening a stream todo-api may answer 403
    // to, only to close it a tick later, is worse than waiting one round trip.
    let realtime_enabled = move || flags_fetched.get() && flags.with(|f| f.is_enabled(FLAG_REALTIME));

    // --- flags -------------------------------------------------------------
    // todo-api is the single flag evaluation point; `flags::fetch` never fails,
    // falling back to the everything-on defaults so a Flagsmith outage or an
    // unreachable /api/flags cannot blank the page.
    Effect::new(move |_| {
        let ident = identity.get();
        flags_fetched.set(false);
        spawn_local(async move {
            flags.set(crate::flags::fetch(&ident).await);
            flags_fetched.set(true);
        });
    });

    // --- todos -------------------------------------------------------------
    // Held until flags resolve: a `todo_app_web`-disabled SPA must not fetch.
    Effect::new(move |_| {
        // Every dependency is read before the guard, or a later flag flip
        // would not re-trigger the load.
        let _ = todos_epoch.get();
        let ident = identity.get();
        let ready = flags_fetched.get();
        let enabled = app_enabled();
        if !(ready && enabled) {
            return;
        }
        spawn_local(async move {
            match api::list(&ident).await {
                Ok(list) => {
                    todos.set(list);
                    load.set(Load::Ready);
                }
                Err(_) => load.set(Load::Error),
            }
        });
    });

    // --- realtime ----------------------------------------------------------
    // The list is driven by database change events, so it also reflects writes
    // made by anything else touching the table (another replica, todo-worker,
    // the CLI, psql). Mutations below patch the same list from their own
    // response, so a local edit still lands instantly if the stream is down.
    //
    // Re-runs when `todo_realtime` flips or the identity changes; dropping the
    // old `Connection` closes both transports, the equivalent of the Solid
    // effect's cleanup.
    Effect::new(move |_| {
        let enabled = realtime_enabled();
        let ident = identity.get();
        CONNECTION.with(|cell| cell.replace(None));
        sse_status.set(CONNECTING);
        ws_status.set(CONNECTING);
        if !enabled {
            return;
        }

        let push = move |transport: &'static str, label: String| {
            let id = feed_seq.get_untracked();
            feed_seq.set(id + 1);
            let at = locale_time();
            feed.update(|items| {
                items.insert(
                    0,
                    FeedItem {
                        id,
                        transport,
                        label,
                        at,
                    },
                );
                items.truncate(MAX_ITEMS);
            });
        };

        let handlers = Handlers {
            on_event: Rc::new(move |event: TodoEvent, from_sse: bool| {
                let label = format!(
                    "{}: {}",
                    event.kind.as_str(),
                    event
                        .todo
                        .as_ref()
                        .map_or_else(|| event.todo_id.to_string(), |todo| todo.title.clone()),
                );
                todos.update(|list| apply_event(list, event));
                push(if from_sse { "sse" } else { "ws" }, label);
            }),
            on_sse_open: Rc::new(move || {
                sse_status.set(OPEN);
                // NOTIFY has no backlog: refetch after a reconnect to pick up
                // whatever was committed while the stream was down.
                todos_epoch.update(|epoch| *epoch += 1);
            }),
            on_sse_error: Rc::new(move || sse_status.set(CLOSED)),
            on_ws_open: Rc::new(move || ws_status.set(OPEN)),
            on_ws_close: Rc::new(move || ws_status.set(CLOSED)),
            on_ws_text: Rc::new(move |text: String| push("ws", text)),
        };

        CONNECTION.with(|cell| cell.replace(Connection::open(&ident, handlers)));
    });

    // --- mutations ---------------------------------------------------------
    // todo-api answers 403 (todo_write off) and 429 (todo_max_items reached);
    // surface those instead of letting a click do nothing.
    let add = move |input: CreateTodo| {
        mutation_error.set(None);
        let ident = identity.get_untracked();
        spawn_local(async move {
            match api::create(&ident, &input).await {
                Ok(todo) => todos.update(|list| upsert_todo(list, todo)),
                Err(message) => mutation_error.set(Some(message)),
            }
        });
    };

    let toggle = move |todo: Todo| {
        mutation_error.set(None);
        let ident = identity.get_untracked();
        let id = todo.id.to_string();
        let completed = todo.completed;
        spawn_local(async move {
            let result = if completed {
                api::uncomplete(&ident, &id).await
            } else {
                api::complete(&ident, &id).await
            };
            match result {
                Ok(todo) => todos.update(|list| upsert_todo(list, todo)),
                Err(message) => mutation_error.set(Some(message)),
            }
        });
    };

    let remove = move |todo_id: uuid::Uuid| {
        mutation_error.set(None);
        let ident = identity.get_untracked();
        let id = todo_id.to_string();
        spawn_local(async move {
            match api::remove(&ident, &id).await {
                Ok(()) => todos.update(|list| crate::realtime::remove_todo(list, todo_id)),
                Err(message) => mutation_error.set(Some(message)),
            }
        });
    };

    let switch_identity = move |next: String| {
        identity.set(next);
        // Both queries are identity-scoped; re-running them is what
        // `invalidateQueries` does on the Solid side. The flags effect reruns
        // on `identity` alone, so only the list needs an explicit bump.
        todos_epoch.update(|epoch| *epoch += 1);
    };

    let submit = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        let trimmed = title.get_untracked().trim().to_owned();
        if trimmed.is_empty() {
            return;
        }
        add(CreateTodo {
            title: trimmed,
            description: String::new(),
            priority: priority.get_untracked(),
        });
        title.set(String::new());
    };

    let status_strip = move || {
        view! {
            <p class="todo-status">
                "Identity " <span class="badge badge--low">{move || identity.get()}</span>
                " flags "
                <span class=move || {
                    let variant = if flag_source() == "remote" { "medium" } else { "high" };
                    format!("badge badge--{variant}")
                }>{flag_source}</span>
                <Show when=move || { max_items() >= 0 }>
                    " cap " <span class="badge badge--high">{max_items}</span>
                </Show>
            </p>
        }
    };

    view! {
        <Show
            when=app_enabled
            fallback=move || {
                view! {
                    <main class="todo-app">
                        <header class="todo-app__header">
                            <h1 class="todo-app__title">"Unavailable"</h1>
                            <span class="todo-app__subtitle">"Leptos"</span>
                        </header>
                        <p class="todo-error" role="alert">
                            "todo-web is disabled by feature flag " <strong>"todo_app_web"</strong>
                            " for identity " <strong>{move || identity.get()}</strong> "."
                        </p>
                        {status_strip()}
                        <IdentitySwitcher identity=identity on_switch=switch_identity />
                    </main>
                }
            }
        >
            <main class="todo-app">
                <header class="todo-app__header">
                    <h1 class="todo-app__title">"Todos"</h1>
                    <span class="todo-app__subtitle">"Leptos"</span>
                </header>

                {status_strip()}
                <IdentitySwitcher identity=identity on_switch=switch_identity />

                <Show when=write_enabled>
                    <form class="todo-form" on:submit=submit>
                        <input
                            class="todo-input"
                            type="text"
                            placeholder="new todo title"
                            aria-label="new todo title"
                            prop:value=move || title.get()
                            on:input=move |ev| title.set(event_target_value(&ev))
                        />
                        <select
                            class="todo-select"
                            aria-label="priority"
                            prop:value=move || priority.get().as_str()
                            on:change=move |ev| {
                                if let Some(next) = TodoPriority::parse(&event_target_value(&ev)) {
                                    priority.set(next);
                                }
                            }
                        >
                            {PRIORITIES
                                .into_iter()
                                .map(|value| {
                                    view! { <option value=value.as_str()>{value.as_str()}</option> }
                                })
                                .collect_view()}
                        </select>
                        <button class="btn btn--primary" type="submit">
                            "Add"
                        </button>
                    </form>
                </Show>
                <Show when=move || !write_enabled()>
                    <p class="todo-status">
                        "Read-only: writes are disabled by feature flag "
                        <strong>"todo_write"</strong> "."
                    </p>
                </Show>

                <Show when=move || load.get() == Load::Pending>
                    <p class="todo-status">"Loading todos…"</p>
                </Show>
                <Show when=move || load.get() == Load::Error>
                    <p class="todo-error" role="alert">
                        "Failed to load todos."
                    </p>
                </Show>
                <Show when=move || mutation_error.with(Option::is_some)>
                    <p class="todo-error" role="alert">
                        {move || mutation_error.get().unwrap_or_default()}
                    </p>
                </Show>

                <ul class="todo-list">
                    <For
                        each=move || todos.get()
                        // Keyed on the row's identity AND its version: an
                        // updated todo is a new key, so the row re-renders —
                        // the behaviour Solid's `<For>` gets from the array
                        // element being replaced.
                        key=|todo| (todo.id, todo.updated_at, todo.completed)
                        let:todo
                    >
                        {
                            // Every attribute value the view macro sees becomes
                            // its own closure, so the row's strings are taken
                            // off `todo` once, up front.
                            let toggle_label = format!("toggle {}", todo.title);
                            let delete_label = format!("delete {}", todo.title);
                            let title_class = if todo.completed {
                                "todo-item__title todo-item__title--done"
                            } else {
                                "todo-item__title"
                            };
                            let priority = todo.priority.as_str();
                            let priority_class = format!("badge badge--{priority}");
                            let title_text = todo.title.clone();
                            let completed = todo.completed;
                            let removed = todo.id;
                            view! {
                                <li class="todo-item">
                                    <Show when=write_enabled>
                                        <input
                                            class="todo-checkbox"
                                            type="checkbox"
                                            aria-label=toggle_label.clone()
                                            prop:checked=completed
                                            on:change={
                                                let todo = todo.clone();
                                                move |_| toggle(todo.clone())
                                            }
                                        />
                                    </Show>
                                    <span class=title_class>{title_text}</span>
                                    <span class=priority_class>{priority}</span>
                                    <Show when=write_enabled>
                                        <button
                                            class="btn btn--danger"
                                            type="button"
                                            aria-label=delete_label.clone()
                                            on:click=move |_| remove(removed)
                                        >
                                            "Delete"
                                        </button>
                                    </Show>
                                </li>
                            }
                        }
                    </For>
                </ul>

                <Show when=realtime_enabled>
                    <EventFeed items=feed sse_status=sse_status ws_status=ws_status />
                </Show>
            </main>
        </Show>
    }
}

/// Switch the Flagsmith identity the whole app is evaluated as. Present in both
/// the normal and the flagged-off view, so a demo can always switch back.
#[component]
fn IdentitySwitcher(
    identity: RwSignal<String>,
    on_switch: impl Fn(String) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let draft = RwSignal::new(identity.get_untracked());
    let rejected = RwSignal::new(false);

    // Writable derived: resets to the applied identity whenever it changes.
    Effect::new(move |_| draft.set(identity.get()));

    let submit = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        match identity::set(&draft.get_untracked()) {
            Some(stored) => {
                rejected.set(false);
                on_switch(stored);
            }
            None => rejected.set(true),
        }
    };

    view! {
        <form class="todo-form" on:submit=submit>
            <input
                class="todo-input"
                type="text"
                aria-label="identity"
                placeholder="identity"
                prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))
            />
            <button class="btn btn--primary" type="submit">
                "Switch user"
            </button>
            <Show when=move || rejected.get()>
                <span class="todo-error" role="alert">
                    "1–64 chars of A–Z a–z 0–9 _ . @ -"
                </span>
            </Show>
        </form>
    }
}

/// `new Date().toLocaleTimeString()` — the browser's own formatting, resolved
/// against `navigator.language` so the feed reads the same as the Solid app's.
fn locale_time() -> String {
    let locale = web_sys::window()
        .and_then(|w| w.navigator().language())
        .unwrap_or_else(|| "en-US".to_owned());
    js_sys::Date::new_0().to_locale_time_string(&locale).into()
}
