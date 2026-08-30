# todo-web — the realtime reference app

SolidJS SPA for the todo vertical. **This is the repo's designated realtime UI**:
its list is driven by Postgres change events, so it reflects writes made by any
process — another `todo-api` replica, `todo-worker`, the todo CLI, or a plain
`psql` UPDATE — not just the ones this browser made.

Full design, rationale and the extension recipe: [`docs/realtime-todo.md`](../../../docs/realtime-todo.md).

Nothing else here is special: the other web apps (`todo-astro-web`,
`todo_web_htmx`, `zerg-web`, `terran-web`) are intentionally request/response.
Pick this one when you need a working example of live-updating UI.

## Routes — one per state-management approach

The same todo loop, implemented three times, so the approaches can be compared on
equal footing (and on measured bytes). See
[`docs/todo-state-management.md`](../../../docs/todo-state-management.md).

| Route | Holds state with | JS (gzip) |
|---|---|---:|
| `/` | TanStack Query + signals (plus flags/identity — the real app) | 46.2 kB |
| `/xstate` | an XState v5 machine | +16.0 kB |
| `/effect` | Effect `SubscriptionRef` + `Stream` | +57.1 kB |

Both alternatives are `lazy()`-loaded, so `/` downloads neither. Keep it that
way: this vertical publishes shipped-JS numbers.

## Layout

| Path | Role |
|---|---|
| `src/lib/realtime.ts` | One shared `EventSource` for the page + the pure cache-merge rules (`applyTodoEvent`, `upsertTodo`, `removeTodo`). |
| `src/lib/realtime.test.ts` | Merge rules + the subscription contract (late subscribers still get their resync callback). |
| `src/todo-app.tsx` | `/` — list + mutations over the TanStack Query cache that events patch. |
| `src/routes/todo-machine.ts` | `/xstate` — the machine: load, write serialisation, resync states. |
| `src/routes/todo-effect.ts` | `/effect` — `SubscriptionRef` state + retry/timeout policy. |
| `src/components/todo-view.tsx` | Presentational list/form shared by the two demo routes. |
| `src/event-feed.tsx` | Debug log of the same stream, plus the WebSocket echo demo. |
| `src/lib/todo-api.ts` | REST client (`/api/todos`). |

## Commands

```bash
bun run dev        # vite dev server on :3100, proxies /api to todo-api (ws: true)
bun run test       # vitest
bun run typecheck  # tsc --noEmit — the actual TS gate; `vite build` does not check types
bun run build
```

Requires `todo-api` on `:8080` (`just run todo-api`) with the todo migrations
applied (`just migrate todo`) — the trigger ships as a migration, so a database
without it produces a working but silent UI.
