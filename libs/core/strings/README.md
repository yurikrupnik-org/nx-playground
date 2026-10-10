# core_strings

Zero-dependency string helpers shared across workspace crates. This README is
the crate-level rustdoc (`#![doc = include_str!("../README.md")]`), so the code
blocks below run as doctests.

## `capitalize_first_letter`

Uppercases the first character of a string, leaving the rest untouched.
Unicode-aware: uses `char::to_uppercase`, so a first character may expand to
multiple characters (e.g. `ß` → `SS`).

```rust
use core_strings::capitalize_first_letter;

assert_eq!(capitalize_first_letter("users"), "Users");
assert_eq!(capitalize_first_letter("API"), "API");
assert_eq!(capitalize_first_letter(""), "");
assert_eq!(capitalize_first_letter("ßeta"), "SSeta");
```

## Consumers

- `api_resource` — capitalizes default API tags derived from collection names
- `sea_orm_resource` — builds Title Case tags from `snake_case` table names
