# State management, four ways — `/`, `/xstate`, `/effect`, `/effect-v4`

`todo-web` serves the same todo loop four times, one route per state-management
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
| `/effect` | Effect 3 (`effect-v3` alias): `SubscriptionRef` for state, `Effect` for operations, `Stream` for subscription. | `routes/todo-effect.ts`, `routes/effect-route.tsx` |
| `/effect-v4` | Effect 4: the same model, plus the REST client as a `TodoApi` service (`Context.Service` + `Layer`). | `routes/todo-effect-v4.ts`, `routes/effect-v4-route.tsx` |

## Measured cost

Production build (`bun run build`), transitive static closure per entry, gzipped
with `cat <chunks> | gzip -9 -c | wc -c`, measured on Solid `2.0.0-rc.9` /
`@solidjs/router@2.0.0-next.26`. Always compress from **stdin**: `gzip -9 -c
FILE` stores an FNAME header and inflates the result by `len(filename)+1`, which
differs per chunk and so skews a comparison between routes.

| Route | JS (gzip) | Δ vs baseline |
|---|---:|---:|
| `/` | 52.1 kB | — |
| `/xstate` | 67.8 kB | **+15.6 kB** |
| `/effect` | 109.2 kB | **+57.1 kB** |
| `/effect-v4` | 74.1 kB | **+21.9 kB** |

`/effect-v4` measured 2026-10-10 on `effect@4.0.2` with the same method (the
other rows re-measured in the same build to within 0.1 kB).

All alternatives are `lazy()`-loaded, so visiting `/` downloads none of them.
That is deliberate and load-bearing: this vertical publishes shipped-JS numbers,
and a library used by one route must not be charged to the others. `xstate` and
both Effect majors land in their own chunks (`xstate-route-*.js`,
`effect-route-*.js`, `effect-v4-route-*.js`), which the table above reflects.

For scale: the whole baseline app is 52.1 kB gz. Effect 3 alone is larger than
that; Effect 4 costs 38% of Effect 3's delta for the same program.

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

Costs: +15.6 kB gz, a second vocabulary, and `@xstate/solid` is unusable here (it
peers on `solid-js@^1.6`; this app is Solid 2), so the actor→signal bridge is
hand-written — all 10 lines of it, in `xstate-route.tsx`.

**Effect 3 (`/effect`)** — makes *effects* first-class rather than states:

- `load` is retried with `Schedule.exponential` and a timeout, declaratively.
  Doing that by hand is the fiddliest part of the baseline route.
- `ManagedRuntime` owns fiber lifetime: `runtime.dispose()` on unmount interrupts
  the mirror fiber, the initial load, and any in-flight retry. No "component
  unmounted, request still running".
- `SubscriptionRef.changes` is a `Stream`, so the UI *subscribes* to state.

Costs: +57 kB gz, the steepest learning curve here, and no official Solid
binding (`@effect-atom/atom-solid` peers `solid-js >=1 <2`), so the
`Stream → signal` bridge is hand-written too.

**Effect 4 (`/effect-v4`)** — the same program on the rewritten runtime:

- `Schedule.compose` is gone; the retry policy is
  `Effect.retry({ schedule: Schedule.exponential('200 millis'), times: 2 })`.
  `forkDaemon` → `forkDetach`/`forkChild`, `ref.changes` →
  `SubscriptionRef.changes(ref)`, `Stream.runCollect` returns an `Array`.
- `todoApi` is provided as a `TodoApi` service instead of imported. The route
  provides `TodoApiLive`; tests provide a fake `Layer`. That is what makes
  `@effect/vitest`'s `TestClock` usable: no promise hops between the program
  and the clock, so the tests check the retry delays (+200 ms, +400 ms) and the
  5 s timeout exactly, in ~1 ms of wall time. The Effect 3 suite waits through
  the real schedule (~1.4 s across three tests) and cannot test the timeout at
  all.
- Fiber lifetime goes through a `FiberSet.makeRuntimePromise` runner on a
  `Scope` that the route closes on cleanup, so every click, stream event and
  retry is interrupted on unmount (and `tryPromise`'s `AbortSignal` cancels
  the HTTP request; there is a test for that).
- **Solid 2 gotcha:** Effect 4 runs a new fiber synchronously up to its first
  async boundary. Started from the component body, the state mirror's first
  emission writes a signal inside Solid's owned scope, and Solid 2 dev *throws*
  `REACTIVE_WRITE_IN_OWNED_SCOPE`. That kills the mirror fiber and leaves the
  page on "Loading todos…". The route starts every fiber through
  `ready.then(...)`, i.e. after the body returns. Effect 3's `runPromise` does
  not hit this.

Costs: +21.9 kB gz (vs +57.1 kB for Effect 3). The learning curve is the same,
and the Solid bridge is still hand-written.

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
exactly that. Effect earns its bytes when you want retry/timeout/interruption
policy as a first-class, composable concern across many operations, and on
Effect 4 those bytes are +21.9 kB rather than +57 kB. As a pure state container
for one list it is still heavy.

## A production bug this measurement exposed

All three SPAs (`todo-web`, `zerg-web`, `terran-web`) set
`resolve.conditions: ['development', 'browser']` unconditionally, which resolves
`solid-js` to `dist/dev.js` — the **development** runtime, with reactivity
warnings and debug hooks — *in production builds*. Now gated on
`command === 'serve'`, and re-measured on the current dependency state: forcing
the condition back on takes todo-web's `/` closure from 52.1 kB to 61.3 kB gz
(**−9.2 kB, −15%** for the guard) and pulls in `web.dev-*.js` instead of
`web-*.js`; verified by the absence of dev-only warning strings (`"You appear to
have multiple instances of Solid"`) in all three production bundles.

## Running it

```bash
task docker-up && task migrate DB=todo     # Postgres + the NOTIFY trigger
task run -- todo-api
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
| `src/routes/todo-effect.test.ts` | 12 tests (Effect 3): retry schedule (asserts attempt count), degradation that keeps visible rows, write error mapping (403/429 flag messages), `SubscriptionRef.changes` emission order. Real timers. |
| `src/routes/todo-effect-v4.test.ts` | 12 tests (Effect 4, `@effect/vitest` `it.effect`): retry delays and the 5 s timeout under `TestClock`, degradation, writes against a fake `TodoApi` layer, `changes` emission order, and two `TodoApiLive` tests with stubbed `fetch` (403 message passes through; interrupting the fiber aborts the request). |
| `src/lib/realtime.test.ts` | 14 tests: merge rules plus the subscription contract, including the late-subscriber `onOpen` fix. |
