import type { TodoEvent } from '@domain/todo';
import { createSignal, For, onCleanup, Show } from 'solid-js';

import { subscribeToTodoEvents, todoEventsWebSocketUrl } from './lib/realtime';

const MAX_ITEMS = 20;

type Transport = 'sse' | 'ws';
type Status = 'connecting' | 'open' | 'closed';

interface FeedItem {
  id: string;
  transport: Transport;
  label: string;
  at: string;
}

/**
 * Live todo event feed demonstrating two realtime transports against the
 * same broadcast bus in todo-api:
 *
 * - SSE via `EventSource('/api/events/sse')` — named events per lifecycle kind.
 * - WebSocket via `/api/events/ws` — same events as text frames, plus a
 *   bidirectional echo (type a message, the server echoes it back).
 *
 * Both go through the Vite dev proxy (`ws: true` for the socket).
 */
export function EventFeed() {
  const [items, setItems] = createSignal<FeedItem[]>([]);
  const [sseStatus, setSseStatus] = createSignal<Status>('connecting');
  const [wsStatus, setWsStatus] = createSignal<Status>('connecting');
  const [message, setMessage] = createSignal('');

  let itemId = 0;

  const push = (transport: Transport, label: string) => {
    const item: FeedItem = {
      id: `${transport}-${itemId++}`,
      transport,
      label,
      at: new Date().toLocaleTimeString(),
    };
    setItems((prev) => [item, ...prev].slice(0, MAX_ITEMS));
  };

  // Connections live for the component's lifetime (body runs once in Solid;
  // `onMount` was removed in 2.0 and no DOM access is needed here).
  // --- SSE: the page-wide subscription, shared with the list view ---
  const unsubscribe = subscribeToTodoEvents(
    (event) =>
      push('sse', `${event.kind}: ${event.todo?.title ?? event.todo_id}`),
    {
      onOpen: () => setSseStatus('open'),
      onError: () => setSseStatus('closed'),
    },
  );

  // --- WebSocket: todo events + echo replies as text frames ---
  const socket = new WebSocket(todoEventsWebSocketUrl());
  socket.onopen = () => setWsStatus('open');
  socket.onclose = () => setWsStatus('closed');
  socket.onmessage = (event) => {
    const data = String(event.data);
    if (data.startsWith('echo: ')) {
      push('ws', data);
      return;
    }
    const parsed = JSON.parse(data) as TodoEvent;
    push('ws', `${parsed.kind}: ${parsed.todo?.title ?? parsed.todo_id}`);
  };

  onCleanup(() => {
    unsubscribe();
    socket.close();
  });

  const sendMessage = (event: Event) => {
    event.preventDefault();
    const trimmed = message().trim();
    if (!trimmed || socket.readyState !== WebSocket.OPEN) return;
    socket.send(trimmed);
    setMessage('');
  };

  return (
    <section class="event-feed" aria-label="live events">
      <header class="event-feed__header">
        <h2 class="event-feed__title">Live events</h2>
        <span class={`feed-status feed-status--${sseStatus()}`}>
          SSE {sseStatus()}
        </span>
        <span class={`feed-status feed-status--${wsStatus()}`}>
          WS {wsStatus()}
        </span>
      </header>

      <form class="event-feed__form" onSubmit={sendMessage}>
        <input
          class="todo-input"
          type="text"
          placeholder="send over WebSocket (echoed back)"
          aria-label="websocket message"
          value={message()}
          onInput={(event) => setMessage(event.currentTarget.value)}
        />
        <button class="btn btn--primary" type="submit">
          Send
        </button>
      </form>

      <Show
        when={items().length > 0}
        fallback={
          <p class="todo-status">
            No events yet — add, complete, or delete a todo.
          </p>
        }
      >
        <ul class="event-feed__list">
          <For each={items()}>
            {(item) => (
              <li class="event-feed__item">
                <span class={`badge badge--${item.transport}`}>
                  {item.transport}
                </span>
                <span class="event-feed__label">{item.label}</span>
                <span class="event-feed__time">{item.at}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}
