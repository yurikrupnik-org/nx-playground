# @native/field-selector

Node addon (N-API, napi-rs 3) over the `field-selector` Rust crate: project JSON
objects down to the fields a caller requested *and* is allowed to read.

The Rust API (`SelectableFields`) keeps field metadata in `&'static` slices —
unreachable from JS, where the schema is only known at runtime. This addon takes
the schema as a constructor argument and applies the same rules: unknown
requested fields are an error, restricted fields are never emitted, fields above
the caller's role are dropped silently. The `fields=a,b,c` grammar itself is
parsed by `field_selector::FieldSelector`, so it stays single-sourced.

## Use

```ts
import { FieldSchema } from '@native/field-selector';

const schema = new FieldSchema([
  { field: 'id' },
  { field: 'title' },
  { field: 'assigneeEmail', requiredRole: 'user' },
  { field: 'auditTrail', requiredRole: 'admin' },
  { field: 'passwordHash', restricted: true },
]);

schema.filter(task, req.query.fields, auth.role); // one object
schema.filterList(tasks, req.query.fields, auth.role); // a page of rows
schema.allowedFields(req.query.fields, auth.role); // e.g. to build a SQL projection
```

## Who uses it

`todo-astro-web`'s `/api/todos` pass-through proxy
(`apps/todo/web-astro/src/pages/api/todos/[...rest].ts`, schema in
`src/lib/projection.ts`). Successful JSON reads are projected server-side; writes
and error bodies pass through untouched.

That app runs the `@astrojs/node` adapter, which is what makes N-API viable:
**this is a Node ABI and cannot load in a browser.** Anything needing the same
rules client-side has to go through `wasm32-wasip1` instead (napi-rs 3 supports
it, at the cost of the `@napi-rs/wasm-runtime` + emnapi payload).

One caveat that bit during integration: rollup cannot inline a `.node` binary, so
the package must be listed in `vite.ssr.external` (see
`apps/todo/web-astro/astro.config.mjs`) to stay a runtime `require`.

Roles are `'anonymous' | 'user' | 'admin'`, ascending; omitting the role means
`'anonymous'`. Build the schema once per DTO and reuse it — construction walks
the rules, the hot path does not.

Returned objects carry the allowed keys in **alphabetical** order, not the input
order: values cross the boundary as `serde_json::Value`, whose default map is a
`BTreeMap` (the workspace does not enable `preserve_order`). `allowedFields()`
is the one ordered view — it follows schema declaration order.

## Build / test

```
just test-napi                 # every addon: build + node tests
bun nx run @native/field-selector:build
cd libs/native/field-selector && bun run test
```

`index.js`, `index.d.ts` and `*.node` are **generated** (napi-cli) and
gitignored; never edit or lint them. Builds use the workspace `napi` cargo
profile: `release` but with `panic = 'unwind'`, because an aborting panic would
kill the host node process instead of surfacing as a JS exception.

`CARGO=cargo` in the npm scripts is deliberate: `devkit secrets fetch` writes a
crates.io token into `.env.local` as `CARGO=…`, which both just and nx inject,
and napi-cli spawns `$CARGO` as the cargo binary.
