# todo-web-leptos — the Leptos/WASM arm of the delivery-surface comparison

A Leptos 0.8 **CSR** single-page app that reimplements the `/` route of the
SolidJS SPA in [`apps/todo/web`](../web), byte-for-byte comparable because it
does byte-for-byte the same work: same endpoints, same feature flags, same
realtime transports, same DOM and the same stylesheet
([`libs/ui/todo-theme/todo.css`](../../../libs/ui/todo-theme/todo.css)).

The point of the crate is the measurement. A byte comparison against a
*different* feature set is worthless, so the parity checklist below is the
claim this app has to defend, and everything it deliberately does **not** do is
listed with it.

```text
trunk build --release        # -> dist/
bun nx build todo-web-leptos # the same thing, cached, from the repo root
trunk serve                  # :3410, proxying /api to 127.0.0.1:8080
```

## Parity checklist — the `/` route of `apps/todo/web`

Every row is implemented and was exercised against a mock todo-api in a real
browser (add / toggle / delete / SSE / WS / flag gates / identity switch).

### Identity — `src/identity.rs` ≙ `web/src/lib/identity.ts`

- [x] `localStorage['todo_identity']`, with an in-memory fallback when storage
      throws or is absent.
- [x] Grammar `^[A-Za-z0-9_.@-]{1,64}$`; an invalid value is rejected and
      **not** written.
- [x] First visit mints `anon-<8 lowercase hex>` from `crypto.getRandomValues`.
- [x] A stored value wins over the in-memory one (another tab's switch is
      picked up).
- [x] Every request carries `X-Todo-Identity` + `X-Todo-App`. `X-Todo-App` is
      **`web`**, not a fourth value — todo-api's flag catalogue has one flag per
      surface (`todo_app_web`/`_htmx`/`_astro`) and this arm must be evaluated
      under exactly the same server-side rules as the Solid app.
- [x] Identity switcher form in *both* the normal and the flagged-off view,
      with the `1–64 chars of A–Z a–z 0–9 _ . @ -` error, the draft resetting to
      the applied identity, and a switch re-evaluating flags **and** the list.

### Feature flags — `src/flags.rs` ≙ `web/src/lib/flags.ts`

- [x] `GET /api/flags`, todo-api as the single evaluation point (the SPA never
      talks to Flagsmith).
- [x] Never fails: network error, non-2xx or unparseable body all degrade to
      the fail-open catalogue defaults with `source = "defaults"`.
- [x] Six-flag catalogue backfilled over the response; unknown extra flags kept;
      a flag missing `enabled` counts as ON.
- [x] `todo_max_items` read as either a number or a string
      (`feature_state_value` is typed by whoever filled the dashboard).
- [x] Gates: `todo_app_web` (whole app), `todo_write` (add form, checkboxes,
      delete buttons), `todo_realtime` (streams + feed), `todo_max_items`
      (the `cap` badge, shown only when ≥ 0).
- [x] The realtime gate additionally waits for the first flag response —
      opening a stream todo-api may 403 is worse than one round trip — while the
      rendering gates fail open immediately.

### Data — `src/api.rs` ≙ `web/src/lib/todo-api.ts`

- [x] Same-origin `/api/todos`, so the nginx `/api` proxy in the web image works
      unchanged and no origin is compiled into the wasm.
- [x] `GET /api/todos?limit=100000`, held until flags resolve and `todo_app_web`
      is on.
- [x] `POST /api/todos`, `POST /{id}/complete`, `POST /{id}/uncomplete`,
      `DELETE /{id}`; `PUT /{id}` mirrored but unused, exactly as in the Solid
      module.
- [x] 403 → `writes are disabled by feature flag (todo_write)`, 429 →
      `todo limit reached (todo_max_items)`, everything else the per-call
      fallback message. Rendered verbatim in a `.todo-error[role=alert]`.
- [x] Mutations patch the in-memory list from their own response, so a local
      edit lands instantly even while the stream is down.

### Realtime — `src/realtime.rs` + `src/event_feed.rs` ≙ `web/src/lib/realtime.ts` + `web/src/event-feed.tsx`

- [x] SSE `/api/events/sse?identity=…` (headers are impossible on
      `EventSource`, so identity travels as the query param todo-api also
      accepts), one listener for all five named events.
- [x] WebSocket `/api/events/ws?identity=…` with `wss:` on HTTPS, plus the
      bidirectional `echo:` round trip.
- [x] Merge rules: `deleted` removes by id, every other kind upserts the
      snapshot; upsert is idempotent and keeps `created_at DESC`.
- [x] SSE open ⇒ refetch the list (`NOTIFY` has no backlog, so a reconnect owes
      a catch-up).
- [x] A malformed frame is dropped, never tearing the stream down.
- [x] Feed: newest 20, `sse`/`ws` transport badge, `toLocaleTimeString()`
      arrival time, `SSE|WS connecting|open|closed` status, and the
      `No events yet — …` empty state.
- [x] The connection pair is rebuilt when the identity changes and closed when
      `todo_realtime` goes off (Leptos: dropping `Connection`; Solid: the
      effect's cleanup).

### Rendered state

- [x] Identical class names and DOM shape against the shared theme —
      `.todo-app`, `.todo-status`, `.todo-form`, `.todo-list`, `.todo-item`,
      `.badge--{low,medium,high}`, `.event-feed*`, `.feed-status--*`.
- [x] `Loading todos…`, `Failed to load todos.`, the read-only notice, the
      status strip (identity badge, `remote`/`defaults` source badge, cap badge)
      and the `Unavailable` page all with the Solid app's copy.
- [x] `aria-label`s preserved (`identity`, `new todo title`, `priority`,
      `toggle <title>`, `delete <title>`, `websocket message`, `live events`).
- [x] Mount point is **`<div id="root">`** — `scripts/wrk/web-servers.sh`
      asserts the SPA fallback by grepping the served HTML for exactly that.

### Deliberately NOT implemented

| Not here | Why |
|---|---|
| `/xstate` and `/effect` routes | They are the *other* two arms of the state-management comparison, `lazy()`-loaded so `/` never downloads them. Reimplementing them would measure something the Solid baseline does not ship. |
| A router | One route. The Solid number this is compared against is the `/` entry's closure only, so a router here would be bytes the baseline does not carry. |
| `AppNav` and `web/src/index.css` | Both exist solely to switch between those three routes. |
| `apps/todo/web`'s vitest suite | The pure merge rules it defends (`upsert`/`remove`/`applyTodoEvent`) are ported, but this crate is a measurement arm, not a second home for the todo contract. |

## Measured bytes

`trunk build --release`, wasm-opt `-Oz`, release profile mirroring the root
workspace (`opt-level = 'z'`, `lto`, `codegen-units = 1`, `panic = 'abort'`,
`strip`):

| File | raw | gzip -9 | brotli -q11 |
|---|---:|---:|---:|
| `todo_web_leptos-<hash>.js` (wasm-bindgen glue) | 41 987 | 7 479 | 6 442 |
| `todo_web_leptos-<hash>_bg.wasm` | 341 429 | 135 036 | 111 099 |
| **shipped code** (js + wasm) | **383 416** | **142 515** | **117 541** |
| `todo-<hash>.css` (shared theme) | 4 732 | 1 533 | 1 278 |
| `index.html` | 2 145 | 1 219 | 939 |
| **first render** (everything above) | **390 293** | **145 267** | **119 758** |

Measure gzip as `gzip -9 -c < FILE`, never `gzip -9 -c FILE`: the named form
writes the filename into the gzip FNAME header, inflating every figure by
`len(name) + 1` bytes — 41 B on the wasm here — and no static server ever sends
that field. `just bench-assets` reports these same two closures; quote a row
from it rather than recomputing.

For scale, wasm-opt matters: the raw cargo artifact is 1 450 691 B before
`-Oz`, so 76 % of the module is optimiser-removable.

### Against the Solid arm

Both built from this tree on the same day, same compressors at the same
levels, same per-file basis. The Solid `/` closure is
`assets/index-<hash>.js` + `assets/web-<hash>.js`; its lazy chunks
(`xstate-route`, `effect-route`, `todo-view`, the router's `decode` and
`serverForms` — 277 kB raw together) are excluded because `/` never downloads
them.

A byte count without a compressor level is not a fact, so every row names one:

| | Solid (`/`) | Leptos | ratio |
|---|---:|---:|---:|
| shipped code, raw | 152 249 | 383 416 | 2.52× |
| shipped code, `gzip -9` | 52 477 | 142 515 | **2.72×** |
| shipped code, `brotli -q11` | 47 315 | 117 541 | 2.48× |
| first render, raw | 156 591 | 390 293 | 2.49× |
| first render, `gzip -9` | 53 985 | 145 267 | **2.69×** |
| first render, `brotli -q11` | 48 526 | 119 758 | 2.47× |

These are best-case compression. What the shared web image actually serves is
gzip level 6 — see below.

135 kB gz of the Leptos side is the wasm module, which carries its own
allocator, panic machinery, `serde`/`serde_json`, `chrono`'s RFC-3339 parser,
`uuid` and the whole reactive runtime — none of which the Solid build ships,
because the browser already provides `JSON`, `Date` and a GC. That, not
framework overhead, is the bulk of the 2.7×.

Two caveats on the numbers, both against Leptos and left uncorrected because
they are real bytes over the wire:

- Vite minifies CSS; trunk copies it verbatim. The Leptos stylesheet is 4 732 B
  to Solid's 3 754 B even though it is the *smaller* stylesheet (shared theme
  only, no `.app-nav` rules).
- Solid's two chunks gzip to 52 128 B when concatenated first, 52 477 B as two
  separate responses. The table uses the per-file sum, because that is what two
  HTTP responses actually cost — and it is the same basis as the Leptos rows.

`docs/todo-state-management.md` previously recorded 46.2 kB gz for `/`; that
was measured on `solid-js@2.0.0-rc.4`, and the RC pairing had to move to rc.9
to build at all, which costs +5.9 kB gz on the baseline.

### Serving a 341 kB wasm

Adding this arm exposed a real gap in the shared web image: the nginx stage
served `.wasm` with neither `Cache-Control` nor gzip (its asset regex and
`gzip_types` predate wasm), and caddy gzipped it but never marked it
immutable. No JS bundle in this repo was large enough for that to matter.
Both are fixed — `application/wasm` is in nginx's `gzip_types` and `wasm` is
in its immutable location, and caddy's `@immutable` matcher covers it.
`just test-web-servers apps/todo/web-leptos/dist` is green across all three
stages (27 checks: health, SPA fallback, js + wasm content-type, long-lived
cache-control, gzip).

On the wire the wasm is **135 626 B**, not the 135 036 B in the table above:
nginx compresses at gzip level 6, and `gzip -6 -c < …_bg.wasm` reproduces
135 626 exactly. Level 9 would save a further 590 B — 0.4 %, for CPU on every
cache miss.

## Layout and wiring notes

- **Standalone workspace.** The crate is in the root `Cargo.toml`
  `[workspace] exclude` list, so it has its own `[workspace]` table and its own
  `Cargo.lock`, and it cannot use `{ workspace = true }` dependencies. It only
  ever builds for `wasm32-unknown-unknown` (pinned in `.cargo/config.toml`),
  while every host gate (`just lint-rust`, `just test-rust`) is a `--workspace`
  host-target run.
- **No `rust` nx tag.** `tools/nx/plugin.ts` withholds the cargo targets and
  the `rust` tag from any `[workspace] exclude`d directory: `cargo … --package`
  from the repo root cannot resolve a non-member, and the tag would hand this
  node to `just check-rust-affected`. The `scope:todo` tag stays.
- **Dist layout.** Trunk puts *everything at the dist root* — `index.html`,
  `<crate>-<hash>.js`, `<crate>_bg-<hash>.wasm`, `<theme>-<hash>.css` — with
  root-relative `/name-hash.ext` hrefs, and **no `assets/` subdirectory**, which
  is where it differs from `vite build`. The bench harness
  (`scripts/bench/*`, `scripts/wrk/web-servers.sh`) resolves assets for both
  layouts; changing `dist`/`public_url` here breaks it.
- **`--enable-bulk-memory`.** `index.html` passes it (plus
  `--enable-bulk-memory-opt`, `--enable-nontrapping-float-to-int`) to wasm-opt:
  rustc emits `memory.copy`/`memory.fill` for this target by default and the
  binaryen trunk downloads refuses to validate them otherwise.
- **DTOs are a mirror, not a dependency.** `src/dto.rs` redeclares `Todo`,
  `CreateTodo`, `UpdateTodo`, `TodoPriority`, `TodoEvent` and `TodoEventKind`
  from [`libs/domains/todo/src/models.rs`](../../../libs/domains/todo/src/models.rs)
  and `events.rs`. `domain_todo` cannot be a dependency of a wasm crate at any
  feature combination (sea-orm, sqlx, axum, validator, utoipa). Keep the serde
  attributes in sync; the committed wire shape is
  [`docs/openapi/todos.v1.json`](../../../docs/openapi/todos.v1.json).
