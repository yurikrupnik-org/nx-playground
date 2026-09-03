//! Realtime todo event fan-out: SSE and WebSocket transports.
//!
//! The bus is fed by [`domain_todo::db_events`], which listens for Postgres
//! `NOTIFY` from the `todos_notify` trigger. Every committed change reaches every
//! connected browser no matter which process wrote it — another API replica, the
//! todo-worker, the CLI, or a hand-run `psql` UPDATE.
//!
//! Two transports stream the same events:
//!
//! - `GET /api/events/sse` — Server-Sent Events. Each todo event becomes a
//!   named SSE event (`created`, `updated`, `completed`, `uncompleted`,
//!   `deleted`) whose data is the JSON-serialized `TodoEvent` (the ts-rs type
//!   in `@domain/todo`). Keep-alive comments hold idle connections open.
//! - `GET /api/events/ws` — WebSocket. Pushes the same JSON events as text
//!   frames and echoes client text frames back (`echo: <text>`) to
//!   demonstrate the bidirectional channel.
//!
//! Slow consumers that fall behind the channel capacity receive a `lagged`
//! notice with the number of dropped events instead of blocking the listener.

use std::convert::Infallible;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use domain_todo::TodoEvent;
use futures::stream::Stream;
use futures::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tracing::{debug, warn};

/// Events buffered per subscriber before a slow consumer starts lagging.
const CHANNEL_CAPACITY: usize = 256;

/// Create the shared event bus. The returned sender is moved into the database
/// listener (producer) and into [`router`] (fan-out).
pub fn channel() -> broadcast::Sender<TodoEvent> {
    broadcast::channel(CHANNEL_CAPACITY).0
}

/// `/sse` + `/ws` routes over the shared event bus.
pub fn router(tx: broadcast::Sender<TodoEvent>) -> Router {
    Router::new()
        .route("/sse", get(sse_handler))
        .route("/ws", get(ws_handler))
        .with_state(tx)
}

/// SSE stream of todo events, one named event per lifecycle transition.
///
/// Opens with a `: connected` comment so response headers flush immediately
/// through buffering proxies (e.g. the Vite dev proxy) and `EventSource`
/// fires `open` without waiting for the first event or keep-alive.
async fn sse_handler(
    State(tx): State<broadcast::Sender<TodoEvent>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let connected = futures::stream::once(async { Ok(Event::default().comment("connected")) });
    let events = BroadcastStream::new(tx.subscribe()).filter_map(|item| async move {
        match item {
            Ok(event) => match Event::default()
                .event(event.kind.subject_suffix())
                .json_data(&event)
            {
                Ok(sse_event) => Some(Ok(sse_event)),
                Err(e) => {
                    warn!(error = %e, "failed to serialize todo event for SSE");
                    None
                }
            },
            Err(BroadcastStreamRecvError::Lagged(skipped)) => Some(Ok(Event::default()
                .event("lagged")
                .data(skipped.to_string()))),
        }
    });
    Sse::new(connected.chain(events)).keep_alive(KeepAlive::default())
}

/// Upgrade to a WebSocket that pushes todo events and echoes client text.
async fn ws_handler(
    State(tx): State<broadcast::Sender<TodoEvent>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, tx.subscribe()))
}

async fn handle_socket(socket: WebSocket, mut rx: broadcast::Receiver<TodoEvent>) {
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(event) => {
                    let Ok(json) = serde_json::to_string(&event) else {
                        warn!("failed to serialize todo event for WebSocket");
                        continue;
                    };
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    debug!(skipped, "WebSocket subscriber lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            msg = receiver.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    if sender
                        .send(Message::Text(format!("echo: {text}").into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                // Ping/pong are answered by axum; ignore binary frames.
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    debug!(error = %e, "WebSocket receive error");
                    break;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn deleted_event() -> TodoEvent {
        TodoEvent::deleted(Uuid::now_v7())
    }

    #[tokio::test]
    async fn sse_endpoint_streams_published_events() {
        let tx = channel();
        let app = router(tx.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/sse")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        // The handler subscribed while producing the response; publish now.
        let event = deleted_event();
        tx.send(event.clone()).expect("subscriber exists");

        // First the connect comment, then the published event (frames may
        // coalesce, so accumulate).
        let mut body = response.into_body().into_data_stream();
        let mut text = String::new();
        while !text.contains("event: deleted\n") {
            let frame = body
                .next()
                .await
                .expect("stream yields a frame")
                .expect("frame ok");
            text.push_str(std::str::from_utf8(&frame).expect("utf8"));
        }

        assert!(text.starts_with(": connected\n"), "got: {text}");
        assert!(text.contains(&event.todo_id.to_string()), "got: {text}");
    }
}
