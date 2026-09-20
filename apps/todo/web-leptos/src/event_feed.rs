//! Live todo event feed — the Rust twin of `apps/todo/web/src/event-feed.tsx`.
//!
//! Renders the two realtime transports against the same broadcast bus in
//! todo-api: SSE (named events per lifecycle kind) and WebSocket (the same
//! events as text frames, plus a bidirectional echo — type a message, the
//! server echoes it back).
//!
//! The connection itself is owned by [`crate::app`]; this component only
//! renders what the handlers pushed into its signals and writes back through
//! [`crate::app::send_over_socket`].

use leptos::prelude::*;

use crate::app::send_over_socket;

/// Newest 20, same window as the Solid component.
pub const MAX_ITEMS: usize = 20;

/// Transport badge: `badge--sse` / `badge--ws` in the shared theme.
pub type Transport = &'static str;

/// `feed-status--{connecting,open,closed}` in the shared theme.
pub type Status = &'static str;

pub const CONNECTING: Status = "connecting";
pub const OPEN: Status = "open";
pub const CLOSED: Status = "closed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedItem {
    pub id: u32,
    pub transport: Transport,
    pub label: String,
    /// `toLocaleTimeString()` of arrival.
    pub at: String,
}

#[component]
pub fn EventFeed(
    items: RwSignal<Vec<FeedItem>>,
    sse_status: RwSignal<Status>,
    ws_status: RwSignal<Status>,
) -> impl IntoView {
    let message = RwSignal::new(String::new());

    let send = move |event: web_sys::SubmitEvent| {
        event.prevent_default();
        let trimmed = message.get_untracked().trim().to_owned();
        if trimmed.is_empty() || !send_over_socket(&trimmed) {
            return;
        }
        message.set(String::new());
    };

    view! {
        <section class="event-feed" aria-label="live events">
            <header class="event-feed__header">
                <h2 class="event-feed__title">"Live events"</h2>
                <span class=move || format!("feed-status feed-status--{}", sse_status.get())>
                    "SSE " {move || sse_status.get()}
                </span>
                <span class=move || format!("feed-status feed-status--{}", ws_status.get())>
                    "WS " {move || ws_status.get()}
                </span>
            </header>

            <form class="event-feed__form" on:submit=send>
                <input
                    class="todo-input"
                    type="text"
                    placeholder="send over WebSocket (echoed back)"
                    aria-label="websocket message"
                    prop:value=move || message.get()
                    on:input=move |ev| message.set(event_target_value(&ev))
                />
                <button class="btn btn--primary" type="submit">
                    "Send"
                </button>
            </form>

            <Show
                when=move || !items.with(Vec::is_empty)
                fallback=|| {
                    view! {
                        <p class="todo-status">
                            "No events yet — add, complete, or delete a todo."
                        </p>
                    }
                }
            >
                <ul class="event-feed__list">
                    <For each=move || items.get() key=|item| item.id let:item>
                        <li class="event-feed__item">
                            <span class=format!(
                                "badge badge--{}",
                                item.transport,
                            )>{item.transport}</span>
                            <span class="event-feed__label">{item.label.clone()}</span>
                            <span class="event-feed__time">{item.at.clone()}</span>
                        </li>
                    </For>
                </ul>
            </Show>
        </section>
    }
}
