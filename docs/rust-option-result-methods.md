# Rust `Option` and `Result` methods — which one, and when

Runnable companion: `libs/patterns/playing/`, one annotated test per method.

- `src/domain.rs` — shared types (`Cluster`, `CfgError`, `AppError`, `Registry`)
- `src/fixtures.rs` — the test registry: one populated key, one empty key, one missing key
- `src/option_methods.rs` — the `Option` half
- `src/result_methods.rs` — the `Result` half

```bash
cargo test -p playing
```

## Pick by what the closure returns

| Closure returns | Use |
| --- | --- |
| `T` | `map` (and `map_err` for the error channel) |
| `Option<T>` / `Result<T, E>` | `and_then` |
| `bool`, keep the value | `filter` (Option only) |
| `bool`, that *is* the answer | `is_some_and` / `is_ok_and` / `is_err_and` |
| nothing — log, metric, `dbg!` | `inspect` / `inspect_err` |

## Pick by where you're going

| From → to | Use |
| --- | --- |
| `Option` → `Result` | `ok_or` / `ok_or_else` |
| `Result` → `Option` | `ok()` / `err()` |
| → plain `T` | `unwrap_or` / `unwrap_or_else` / `unwrap_or_default` |
| → some other `U` | `map_or` / `map_or_else` |
| `Option<Option<T>>` → `Option<T>` | `flatten` (on `Result`: `and_then(identity)`) |
| `Option<Result<T, E>>` ↔ `Result<Option<T>, E>` | `transpose` |
| two `Option`s → one | `zip` (both) / `or` (either) / `xor` (exactly one) |
| recover from `Err` | `or_else` (Result only) |

## The methods

- **`map`** — projecting a field, changing representation; `map_err` for wrapping an error at an API boundary. If the closure can fail you'll end up with `Option<Option<_>>` — that's the signal you wanted `and_then`.
- **`and_then`** — nested optional data, chained lookups, fallible-by-nature APIs (`.first()`, `.checked_add`, `.strip_prefix`, `env::var().ok()`). `?` reads better inside a function that already returns `Option`/`Result`, so `and_then` earns its keep in expression position — closures, iterator chains, `let` bindings. On `Result` every link must share one error type; `?` applies `From` for you, `and_then` does not.
- **`filter`** — "present but useless is the same as absent": empty env vars, expired cache entries, narrowing before a default. No `Result::filter` exists — dropping a value there means inventing an error, so spell it `and_then(|x| if p { Ok(x) } else { Err(..) })`.
- **`inspect`** — tracing and metrics mid-chain; `inspect_err` to log a failure and still return it. The best member of this family: it replaces match-log-rewrap, which can drop the error by accident.
- **`ok_or` / `ok_or_else`** — the Option→Result boundary that makes `?` work; eager vs lazy tied to clippy's `or_fun_call`. Also how you recover the "which link failed" information that `and_then` on `Option` throws away.
- **`ok()` / `err()`** — the inverse, discarding the other branch. `ok()` is where error information goes to die: log before you call it, or the failure becomes indistinguishable from absence.
- **`unwrap_or` / `unwrap_or_else`** — config defaults and pipeline ends, with the warning that a default silently turns "broken config" into `0`. On `Result`, `unwrap_or_else` receives the error, so the fallback can depend on *why* it failed.
- **`map_or` / `map_or_else`** — metrics and rendering, plus the reversed argument order (default first). On `Result`, `map_or_else` takes the *error* closure first, then the value closure.
- **`is_some_and` / `is_ok_and`** — `if`/`while` conditions and iterator predicates; replaces `matches!(opt, Some(x) if p)`. Siblings: `is_none_or` (vacuously true when absent) and `is_err_and` (classify a failure without unwrapping it).
- **`or_else`** — the Result-only one with no real `Option` analogue: recover from a failure with something that can itself fail. Fallback config sources, retries, failover. The closure sees the error, so you can recover per-variant and re-raise the rest. If both sides fail you keep the *second* error.
- **`flatten`** — for double layers you didn't create (serde double-option, `HashMap<K, Option<V>>`); if you wrote the `map`, use `and_then`. `Result::flatten` is still unstable — write `.and_then(std::convert::identity)`.
- **`transpose`** — optional config/env/CLI/nullable-column values that must parse, so `?` propagates only a *malformed* value, while absent stays `Ok(None)`. Read the other way it moves an `Option` out of a fallible producer (`Result<Option<Row>, DbErr>`) for iterator and stream code.
- **`zip`** — coordinates, ranges, paired credentials; doesn't exist on `Result`, with `or`/`xor` as siblings. For the `Result` version, `?` each side or write `a.and_then(|x| b.map(|y| (x, y)))` and pick which error wins.
- **`collect::<Result<Vec<_>, _>>()`** — `transpose` for a whole collection: short-circuits at the first `Err`. Use `flat_map` instead when you want the survivors, and log what you dropped.

## Two rules that decide most cases

**`_or` vs `_or_else`.** The eager form builds its argument even on the happy path. Literals are free; anything that allocates, reads env, or hits disk wants `_else`. Clippy enforces both directions — `or_fun_call` for the eager mistake, `unnecessary_lazy_evaluations` for the lazy one.

**`?` vs a combinator chain.** Inside a function that already returns `Option`/`Result`, `?` is shorter and applies `From` on the error. Combinators win where `?` can't go: closures, iterator adapters, `let` bindings, and match arms.

## Clippy lints that encode this

| Lint | Catches |
| --- | --- |
| `bind_instead_of_map` | `and_then(\|x\| Some(y))` — you meant `map` |
| `map_flatten` | `.map(f).flatten()` — you meant `and_then` / `flat_map` |
| `or_fun_call` | eager `unwrap_or(expensive())` |
| `unnecessary_lazy_evaluations` | lazy `unwrap_or_else(\|\| 0)` |
| `option_if_let_else`, `manual_map` | hand-rolled `match` that a combinator says better |

## Version notes

`Option::filter` 1.27 · `Option::transpose` / `Result::transpose` 1.33 · `Option::zip` 1.46 · `is_some_and` / `is_ok_and` / `is_err_and` 1.70 · `inspect` / `inspect_err` 1.76 · `is_none_or` 1.82 · `Result::flatten` still unstable.
