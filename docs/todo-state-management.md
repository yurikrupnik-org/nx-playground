# State management, three ways — `/`, `/xstate`, `/effect`

`todo-web` serves the same todo loop three times, one route per state-management
approach. The vertical already compares *rendering* stacks by shipped JS
(`stack_profiles`, rendered by web-astro); this compares *state* the same way, on
the same measured basis.

Every route does the same work: load the list, add, toggle, delete, and stay in
sync with database change events (see [`realtime-todo.md`](./realtime-todo.md)).
They share the HTTP client (`lib/todo-api.ts`), the pure merge rules
(`lib/realtime.ts`) and the SSE subscription, so the only variable is who holds
the state.

| Route | Implementation | Files |
|---|---|---|
| `/` | TanStack Query cache + signals. Also carries feature flags and identity switching, so it is the real app rather than a demo. | `todo-app.tsx` |
| `/xstate` | One XState v5 machine: explicit `loading / failed / ready.{idle,saving,resyncing}`. | `routes/todo-machine.ts`, `routes/xstate-route.tsx` |
| `/effect` | Effect `SubscriptionRef` for state, `Effect` for operations, `Stream` for subscription. | `routes/todo-effect.ts`, `routes/effect-route.tsx` |

## Measured cost

Production build (`bun run build`), transitive static closure per entry, gzipped:

| Route | JS (gzip) | Δ vs baseline |
|---|---:|---:|
| `/` | 46.2 kB | — |
| `/xstate` | 62.2 kB | **+16.0 kB** |
| `/effect` | 103.3 kB | **+57.1 kB** |

Both alternatives are `lazy()`-loaded, so visiting `/` downloads neither. That is
deliberate and load-bearing: this vertical publishes shipped-JS numbers, and a
library used by one route must not be charged to the others. `xstate` and `effect`
land in their own chunks (`xstate-route-*.js`, `effect-route-*.js`), which the
table above reflects.

For scale: the whole baseline app is 46.2 kB gz. Effect alone is larger than that.

## What each approach actually buys

**TanStack Query (`/`)** — the async state machine you already have. Caching,
dedup, invalidation, request status. The gap is *coordination*: multi-step rules
end up spread across signals and callbacks, which is where the two bugs below
came from.

**XState (`/xstate`)** — makes illegal states unrepresentable and coordination
explicit:

- `ready.saving` serialises writes. A second click during an in-flight write is
  dropped by the state chart, not by a hand-rolled boolean.
- `ready.resyncing` turns "must refetch after reconnect" into a state you can see
  in a diagram, instead of a callback someone can forget.
- `failed` can retry, and treats a stream reconnect as a retry trigger.
- Promise actors receive an `AbortSignal` that fires when the actor stops or the
  invoking state is exited, so an abandoned load aborts its request. Wired in
  `loadTodos`; parity with Effect here, and easy to forget in both.

Costs: +16 kB gz, a second vocabulary, and `@xstate/solid` is unusable here (it
peers on `solid-js@^1.6`; this app is Solid 2), so the actor→signal bridge is
hand-written — all 10 lines of it, in `xstate-route.tsx`.

**Effect (`/effect`)** — makes *effects* first-class rather than states:

- `load` is retried with `Schedule.exponential` and a timeout, declaratively.
  Doing that by hand is the fiddliest part of the baseline route.
- `ManagedRuntime` owns fiber lifetime: `runtime.dispose()` on unmount interrupts
  the mirror fiber, the initial load, and any in-flight retry. No "component
  unmounted, request still running".
- `SubscriptionRef.changes` is a `Stream`, so the UI *subscribes* to state.

Costs: +57 kB gz, the steepest learning curve of the three, and no official Solid
binding (`@effect-atom/atom-solid` peers `solid-js >=1 <2`), so the
`Stream → signal` bridge is hand-written too.

## Two bugs this exercise surfaced

Both were found by running the routes, not by reading code — worth recording
because they are the kind of thing the machine formalism is supposed to prevent.

1. **Dropped `streamOpen`** (`routes/todo-machine.ts`). The SSE stream usually
   connects *while the first load is still in flight*. The event was only handled
   in `ready`, so it was silently discarded and the UI showed "stream off"
   forever. Fixed by handling it at the machine root, with child states
   overriding to also refetch/retry. Regression test: *"records a stream that
   connects before the first load finishes"*.
2. **Late subscribers never got `onOpen`** (`lib/realtime.ts`). The `EventSource`
   is a page-wide singleton; a view that subscribed to an *already-open* stream
   never saw `onopen` again, so it skipped its catch-up refetch — and `NOTIFY` has
   no backlog, so those changes were simply lost. This affected the baseline route
   too (the event feed mounting alongside the list). Fixed by tracking whether the
   stream is open and invoking a late subscriber's `onOpen` immediately.

Note which design caught which: the machine made bug 1 obvious once stated as
"which states handle this event?", while bug 2 lived in shared plumbing that no
state library would have covered.

## Recommendation

Default to the baseline. Reach for a machine when coordination — not data
fetching — is the hard part: write serialisation, resync obligations, wizard
flows, optimistic rollback. XState is the cheaper of the two here and targets
exactly that. Effect earns its 57 kB when you want retry/timeout/interruption
policy as a first-class, composable concern across many operations; as a pure
state container for one list it is heavy.

## A production bug this measurement exposed

All three SPAs (`todo-web`, `zerg-web`, `terran-web`) set
`resolve.conditions: ['development', 'browser']` unconditionally, which resolves
`solid-js` to `dist/dev.js` — the **development** runtime, with reactivity
warnings and debug hooks — *in production builds*. Now gated on
`command === 'serve'`. todo-web's baseline dropped from 56.8 kB to 46.2 kB gz
(**−9.6 kB, −17%**) from that one line; verified by the absence of dev-only
warning strings in all three bundles.

## Running it

```bash
just docker-up && just migrate todo     # Postgres + the NOTIFY trigger
just run todo-api
cd apps/todo/web && bun run dev         # http://localhost:3100
```

Then switch routes in the nav and write to the database directly — every route
updates live:

```bash
docker exec -i dockers-postgres-1 psql -U myuser -d todo \
  -c "INSERT INTO todos (id, title) VALUES (gen_random_uuid(), 'from psql');"
```

## Tests

| File | Covers |
|---|---|
| `src/routes/todo-machine.test.ts` | 11 tests: load/retry, write serialisation, error surfacing and clearing, changes applied mid-write, resync on reconnect, the dropped-`streamOpen` regression. Actors are stubbed via `machine.provide`, states awaited with `waitFor` — no network, no timers. |
| `src/routes/todo-effect.test.ts` | 12 tests: retry schedule (asserts attempt count), degradation that keeps visible rows, write error mapping (403/429 flag messages), `SubscriptionRef.changes` emission order. |
| `src/lib/realtime.test.ts` | 14 tests: merge rules plus the subscription contract, including the late-subscriber `onOpen` fix. |
