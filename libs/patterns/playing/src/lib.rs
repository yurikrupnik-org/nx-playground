#![allow(dead_code)]

//! Option/Result combinators, demonstrated on a cluster registry.
//!
//! - [`domain`] — the shared types every example uses
//! - [`option_methods`] — one annotated test per `Option` method
//! - [`result_methods`] — the `Result` half, including the mirrored error-channel
//!   family (map_err, or_else, inspect_err, is_err_and) and what `Result` lacks
//!
//! Prose write-up: `docs/rust-option-result-methods.md` at the repo root.
//!
//! PICK BY WHAT THE CLOSURE RETURNS:
//!
//! ```text
//!   T                       -> map            (map_err for the error channel)
//!   Option<T> / Result<T,E> -> and_then       (bind; flattens as it chains)
//!   bool, keep the value    -> filter         (Option only)
//!   bool, that's the answer -> is_some_and / is_ok_and / is_err_and
//!   nothing (log, metric)   -> inspect / inspect_err
//! ```
//!
//! PICK BY WHERE YOU'RE GOING:
//!
//! ```text
//!   Option -> Result     ok_or / ok_or_else      (name the absence, enables ?)
//!   Result -> Option     ok() / err()            (discard the other branch)
//!   -> T                 unwrap_or / unwrap_or_else / unwrap_or_default
//!   -> U                 map_or / map_or_else    (transform + default, in one)
//!   Option<Option<T>>    -> flatten              (Result: and_then(identity))
//!   Option<Result<T,E>> <-> Result<Option<T>,E>  transpose
//!   two Options -> one   zip (both) / or (either) / xor (exactly one)
//!   recover from Err     or_else                 (Result only)
//! ```
//!
//! `_or` vs `_or_else`: the eager form builds its argument even on the happy
//! path. Literals are free; anything that allocates or does work wants `_else`.
//! And inside a fn that already returns Option/Result, `?` usually beats a
//! combinator chain — it also applies `From` on the error, which `and_then`
//! cannot. These earn their keep in expression position.

pub mod domain;
pub mod option_methods;
pub mod result_methods;

#[cfg(test)]
mod fixtures;
