//! `Result` combinators: the same shapes as [`crate::option_methods`], but the
//! empty case carries a reason — which changes when each one is the right call.
//!
//! Prose version: `docs/rust-option-result-methods.md` at the repo root.
//!
//! Three differences drive everything below:
//!   1. `Result` has a second type parameter, so there is a whole mirrored
//!      family for the error channel: map_err, or_else, inspect_err, is_err_and.
//!   2. Every link in a chain must agree on `E`. `?` applies `From` for you;
//!      `and_then` does not. That is usually the reason to prefer `?`.
//!   3. `filter` and `zip` don't exist here — dropping a value or merging two
//!      would mean inventing an error, and only you know which one.

use crate::domain::{AppError, CfgError, Registry, bounded, lookup, parse_size};

/// A `Result` chain end to end: lookup -> require field -> transform -> validate.
/// Every link fails with the same `CfgError`, which is what lets `and_then`
/// stay in expression position.
pub fn headroom(reg: &Registry, key: &str) -> Result<u32, CfgError> {
    lookup(reg, key)
        .and_then(|c| c.autoscaler.as_ref().ok_or(CfgError::Missing("autoscaler"))) // chain another fallible step
        .map(|a| a.max - a.min) // plain transform
        .and_then(bounded) // and another
}

/// The same pipeline written with `?`, and returning a *different* error type.
/// Each `?` applies `From<CfgError> for AppError` — the conversion `and_then`
/// cannot do, and the reason `?` wins once error types stop matching.
pub fn provision(reg: &Registry, key: &str, raw_size: &str) -> Result<u32, AppError> {
    let cluster = lookup(reg, key)?;
    let size = bounded(parse_size(raw_size)?)?;
    if cluster.pools.is_empty() {
        return Err(AppError::Io("no pools to resize"));
    }
    Ok(size)
}

/// `or_else` as fallback-source: try the primary, fall back on failure, and
/// keep the *second* error if both fail.
pub fn size_with_fallback(primary: &str, fallback: &str) -> Result<u32, CfgError> {
    parse_size(primary).or_else(|_| parse_size(fallback))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::fixtures::{DEV, MISSING, PROD, registry};
    use std::convert::identity;

    // ------------------------------------------------------ map / map_err
    // map touches the Ok value, map_err the Err value. Neither can fail, and
    // neither runs on the other branch.
    //
    // USE map WHEN the transform is infallible: project a field, change units,
    // wrap a value in your own type.
    // USE map_err AT EVERY BOUNDARY you own — it is the single most-used Result
    // method in real code:
    //   - turn a library error into yours   (.map_err(|e| CfgError::Parse("size")))
    //   - add context                       (.map_err(|e| MyErr::Db { source: e, id }))
    //   - satisfy a chain whose E disagrees (.map_err(Into::into) before and_then)
    // If a `From` impl exists, `?` already calls it — map_err is for when it
    // doesn't, or when you want to attach context the From impl can't see.
    #[test]
    fn map_and_map_err_touch_one_branch_each() {
        let doubled: Result<u32, CfgError> = parse_size("12").map(|n| n * 2);
        assert_eq!(doubled, Ok(24));

        // map leaves Err untouched — the closure never runs.
        let still_err: Result<u32, CfgError> = parse_size("oops").map(|n| n * 2);
        assert_eq!(still_err, Err(CfgError::Parse("size")));

        // map_err is the mirror: Ok passes through, Err is rewritten.
        let widened: Result<u32, AppError> = parse_size("oops").map_err(AppError::Cfg);
        assert_eq!(widened, Err(AppError::Cfg(CfgError::Parse("size"))));
        assert_eq!(parse_size("12").map_err(AppError::Cfg), Ok(12));
    }

    // ----------------------------------------------------------- and_then
    // Bind on the Ok branch: the closure returns another Result, and the first
    // Err short-circuits everything after it.
    //
    // USE WHEN each stage is fallible and they all share one error type:
    //   - read -> parse -> validate -> persist
    //   - deserialize, then check invariants
    //   - any place `?` isn't available: closures, iterator chains
    //     (.map(|s| parse_size(s).and_then(bounded)).collect::<Result<Vec<_>, _>>())
    // DON'T USE when error types differ — either .map_err(Into::into) between
    // links or drop into a function body and use `?`, which applies From for you.
    // DON'T USE when the closure can't fail: that's map.
    #[test]
    fn and_then_chains_fallible_steps() {
        assert_eq!(parse_size("12").and_then(bounded), Ok(12));
        assert_eq!(
            parse_size("999").and_then(bounded),
            Err(CfgError::Bounds(1, 100))
        );
        assert_eq!(
            parse_size("oops").and_then(bounded),
            Err(CfgError::Parse("size"))
        );
        // The first error wins; `bounded` never runs on a parse failure — which is
        // exactly why the third case reports Parse, not Bounds.

        let reg = registry();
        assert_eq!(headroom(&reg, PROD), Ok(19));
        assert_eq!(headroom(&reg, DEV), Err(CfgError::Missing("autoscaler")));
        assert_eq!(headroom(&reg, MISSING), Err(CfgError::Missing("cluster")));
    }

    // `and` / `or` are the eager, closure-free versions: both sides are evaluated
    // before the call. Fine for values already in hand, wasteful otherwise.
    #[test]
    fn and_or_are_the_eager_forms() {
        let a: Result<u32, CfgError> = Ok(1);
        let b: Result<u32, CfgError> = Ok(2);
        assert_eq!(a.clone().and(b.clone()), Ok(2)); // keep the second if the first is Ok
        assert_eq!(
            Err::<u32, _>(CfgError::Parse("x")).and(b),
            Err(CfgError::Parse("x"))
        );
        assert_eq!(Err::<u32, _>(CfgError::Parse("x")).or(a), Ok(1)); // recover
    }

    // ----------------------------------------------------------- or_else
    // and_then for the error branch: given the error, produce another Result.
    // This is the Result-only method with no Option analogue worth the name.
    //
    // USE WHEN failure is recoverable and recovery can itself fail:
    //   - fallback config source   (env -> file -> default)
    //   - retry with different args, failover to a replica
    //   - repair a specific error  (.or_else(|e| if e.is_not_found() { create() } else { Err(e) }))
    // The closure receives the error, so you can decide per-variant whether to
    // recover or re-raise. `unwrap_or_else` is the version that must succeed.
    #[test]
    fn or_else_recovers_from_a_failure() {
        assert_eq!(size_with_fallback("12", "99"), Ok(12)); // primary wins
        assert_eq!(size_with_fallback("oops", "99"), Ok(99)); // fell back
        // Both failed: you get the *last* error, not the first.
        assert_eq!(
            size_with_fallback("oops", "nope"),
            Err(CfgError::Parse("size"))
        );
    }

    // ---------------------------------------------------- inspect / inspect_err
    // Side effects that leave the value and the type alone.
    //
    // inspect_err is the one to remember: log or count a failure and still hand
    // it to the caller. The alternative — matching, logging, re-wrapping — is
    // three lines that can drop the error by accident.
    //   .inspect_err(|e| tracing::warn!(?e, "config load failed"))?
    // inspect (the Ok side) is for tracing and metrics mid-chain.
    // Rust 1.76+.
    #[test]
    fn inspect_err_logs_without_swallowing() {
        let mut logged: Vec<CfgError> = Vec::new();
        let mut ok_seen = 0;

        let out = parse_size("oops")
            .inspect(|_| ok_seen += 1)
            .inspect_err(|e| logged.push(e.clone()))
            .and_then(bounded);

        assert_eq!(out, Err(CfgError::Parse("size"))); // still propagates
        assert_eq!(logged, vec![CfgError::Parse("size")]);
        assert_eq!(ok_seen, 0); // the Ok side never ran
    }

    // -------------------------------------------------------------- ok / err
    // Result -> Option, discarding the other branch. The inverse of ok_or.
    //
    // USE ok() WHEN the reason genuinely doesn't matter downstream:
    //   - optional-by-nature reads   (env::var("X").ok())
    //   - feeding a combinator that only speaks Option (filter, zip, ?)
    // USE err() to test or extract the failure  (assert_eq!(r.err(), Some(..))).
    // Both consume the Result — as_ref() first if you need it afterwards.
    // WATCH OUT: `.ok()` is where error information goes to die. If nobody
    // logged it first, the failure becomes indistinguishable from absence.
    #[test]
    fn ok_and_err_convert_to_option() {
        assert_eq!(parse_size("12").ok(), Some(12));
        assert_eq!(parse_size("oops").ok(), None); // reason discarded
        assert_eq!(parse_size("oops").err(), Some(CfgError::Parse("size")));
        assert_eq!(parse_size("12").err(), None);

        // The Option-only combinators become available once you're through .ok():
        let big = parse_size("12").ok().filter(|n| *n > 10);
        assert_eq!(big, Some(12));

        // Log before you discard, or use ok_or/`?` and keep the error.
        let quiet: Option<u32> = parse_size("oops")
            .inspect_err(|_e| { /* warn!(...) */ })
            .ok();
        assert_eq!(quiet, None);
    }

    // ------------------------------------------ unwrap_or / unwrap_or_else
    // Leave the Result world with a fallback value. Same rules as on Option,
    // with one addition: unwrap_or_else receives the error, so the fallback can
    // depend on *why* it failed.
    //
    // USE WHEN the caller can't act on the failure anyway:
    //   - defaults from partially-broken config  (parse_size(raw).unwrap_or(10))
    //   - degraded-mode reads                    (cache.get().unwrap_or_else(|_| compute()))
    //   - unwrap_or_default() when T: Default
    // DON'T USE at a boundary that should report the problem — `?` instead.
    // A silent default turns "operator typo" into "cluster of size 0".
    #[test]
    fn unwrap_or_leaves_the_result_world() {
        assert_eq!(parse_size("12").unwrap_or(10), 12);
        assert_eq!(parse_size("oops").unwrap_or(10), 10);
        assert_eq!(parse_size("oops").unwrap_or_default(), 0);

        // The error is available to the fallback — impossible on Option.
        let recovered = parse_size("oops").unwrap_or_else(|e| match e {
            CfgError::Parse(_) => 1,
            CfgError::Bounds(min, _) => min,
            CfgError::Missing(_) => 0,
        });
        assert_eq!(recovered, 1);
    }

    // --------------------------------------------------- map_or / map_or_else
    // Transform and default in one step. On Result, map_or_else takes the error
    // closure FIRST, then the value closure — the opposite reading order from
    // map_or, whose plain default comes first.
    //
    // USE WHEN both branches must end as the same concrete type:
    //   - rendering / logging  (r.map_or_else(|e| format!("error: {e:?}"), |v| v.to_string()))
    //   - metrics and status codes  (r.map_or(500, |_| 200))
    // map_or ignores the error entirely; map_or_else is how you keep it.
    #[test]
    fn map_or_else_handles_both_branches() {
        assert_eq!(parse_size("12").map_or(0, |n| n * 2), 24);
        assert_eq!(parse_size("oops").map_or(0, |n| n * 2), 0); // error dropped

        // (error closure, value closure) — in that order.
        let msg = parse_size("oops").map_or_else(|e| format!("{e:?}"), |n| n.to_string());
        assert_eq!(msg, r#"Parse("size")"#);
        assert_eq!(
            parse_size("12").map_or_else(|e| format!("{e:?}"), |n| n.to_string()),
            "12"
        );
    }

    // ----------------------------------------------- is_ok_and / is_err_and
    // Straight to bool, predicate included.
    //
    // USE WHEN the question is yes/no and the value is irrelevant afterwards:
    //   - guards and conditions   (if res.is_ok_and(|n| n > threshold) {..})
    //   - iterator predicates     (.filter(|s| parse_size(s).is_ok_and(|n| n > 10)))
    // is_err_and is the one worth knowing: classify a failure without unwrapping.
    //   if e.is_err_and(|e| e.is_retryable()) { retry() }
    // Both Err-and-false and Ok-with-false-predicate give `false`, so don't use
    // it where those need different handling. Rust 1.70+.
    #[test]
    fn is_ok_and_asks_a_yes_no_question() {
        assert!(parse_size("12").is_ok_and(|n| n > 10));
        assert!(!parse_size("2").is_ok_and(|n| n > 10)); // Ok, predicate false
        assert!(!parse_size("oops").is_ok_and(|n| n > 10)); // Err

        // Classify the failure without matching or unwrapping.
        assert!(parse_size("oops").is_err_and(|e| matches!(e, CfgError::Parse(_))));
        assert!(
            parse_size("999")
                .and_then(bounded)
                .is_err_and(|e| matches!(e, CfgError::Bounds(..)))
        );
    }

    // ------------------------------------------------------------- flatten
    // Result<Result<T, E>, E> -> Result<T, E>.
    //
    // Result::flatten is still UNSTABLE (unlike Option::flatten). Spell it
    // `.and_then(identity)` — same thing, stable, and it reads as "unwrap one
    // layer of fallibility".
    //
    // USE WHEN the nesting comes from something you don't control:
    //   - a fn returning Result whose Ok is itself a Result
    //   - JoinHandle/task results: Result<Result<T, TaskErr>, JoinErr> (map_err
    //     one side first so both errors agree)
    // If you produced the nesting with your own `map`, use and_then instead.
    #[test]
    fn flatten_needs_and_then_identity_on_result() {
        let nested: Result<Result<u32, CfgError>, CfgError> = Ok(parse_size("12"));
        assert_eq!(nested.and_then(identity), Ok(12));

        let inner_failed: Result<Result<u32, CfgError>, CfgError> = Ok(parse_size("oops"));
        assert_eq!(
            inner_failed.and_then(identity),
            Err(CfgError::Parse("size"))
        );

        let outer_failed: Result<Result<u32, CfgError>, CfgError> =
            Err(CfgError::Missing("cluster"));
        assert_eq!(
            outer_failed.and_then(identity),
            Err(CfgError::Missing("cluster"))
        );
        // Both layers collapse into one error channel; which layer failed is lost
        // unless the error variants already say so.
    }

    // ----------------------------------------------------------- transpose
    // Result<Option<T>, E> -> Option<Result<T, E>>. The same call as on Option,
    // read in the other direction.
    //
    // USE WHEN you have a fallible producer of an optional value and the consumer
    // wants the Option outside:
    //   - a DB query returning Result<Option<Row>, DbErr> feeding
    //     stream/iterator code that expects Option
    //   - `.transpose()` inside filter_map to skip absent rows but keep errors
    // The Option-side direction (optional input that must parse) is the common
    // one — see option_methods::transpose_swaps_option_and_result.
    #[test]
    fn transpose_moves_the_option_outward() {
        let found: Result<Option<u32>, CfgError> = Ok(Some(12));
        let empty: Result<Option<u32>, CfgError> = Ok(None);
        let failed: Result<Option<u32>, CfgError> = Err(CfgError::Missing("row"));

        assert_eq!(found.transpose(), Some(Ok(12)));
        assert_eq!(empty.transpose(), None); // no row, nothing to report
        assert_eq!(failed.transpose(), Some(Err(CfgError::Missing("row")))); // error survives
    }

    // ------------------------------------------------------- no zip, no filter
    // Result has neither, and the absence is principled: zip would have to pick
    // one of two errors, filter would have to invent one. Both choices are yours.
    //
    // For zip: `?` each side, or and_then/map by hand.
    // For filter: and_then with an explicit Err.
    #[test]
    fn zip_and_filter_are_spelled_out_by_hand() {
        // "zip": first error wins, because you said so.
        let pair: Result<(u32, u32), CfgError> =
            parse_size("3").and_then(|a| parse_size("10").map(|b| (a, b)));
        assert_eq!(pair, Ok((3, 10)));
        assert_eq!(
            parse_size("oops").and_then(|a: u32| parse_size("10").map(|b| (a, b))),
            Err(CfgError::Parse("size"))
        );

        // "filter": name the error you'd be inventing.
        let only_big = parse_size("3").and_then(|n| {
            if n > 10 {
                Ok(n)
            } else {
                Err(CfgError::Bounds(11, 100))
            }
        });
        assert_eq!(only_big, Err(CfgError::Bounds(11, 100)));
    }

    // ------------------------------------------ collect: transpose for many
    // `collect::<Result<Vec<_>, _>>()` is the whole-collection version: it stops
    // at the first Err and returns it, or gives you every value.
    //
    // USE WHEN validating a batch where any single failure sinks the operation:
    //   - parsing a config list, a CSV, a set of CLI args
    // Use partition/filter_map instead when you want the good ones anyway.
    #[test]
    fn collect_is_transpose_for_a_whole_collection() {
        let all: Result<Vec<u32>, CfgError> = ["1", "2", "3"].into_iter().map(parse_size).collect();
        assert_eq!(all, Ok(vec![1, 2, 3]));

        let any_bad: Result<Vec<u32>, CfgError> =
            ["1", "x", "3"].into_iter().map(parse_size).collect();
        assert_eq!(any_bad, Err(CfgError::Parse("size"))); // short-circuits at "x"

        // Keep the survivors instead. Result is IntoIterator (one item if Ok, none
        // if Err), so flat_map drops the failures — silently, so log them first.
        let survivors: Vec<u32> = ["1", "x", "3"].into_iter().flat_map(parse_size).collect();
        assert_eq!(survivors, vec![1, 3]);
    }

    // ------------------------------------------------- `?` and the From impl
    // The reason `?` usually beats a combinator chain: it applies
    // From<CfgError> for AppError at every call site, for free. and_then cannot —
    // it requires the error types to already match.
    #[test]
    fn question_mark_converts_error_types() {
        let reg = registry();

        assert_eq!(provision(&reg, PROD, "50"), Ok(50));
        // CfgError converted to AppError on the way out:
        assert_eq!(
            provision(&reg, MISSING, "50"),
            Err(AppError::Cfg(CfgError::Missing("cluster")))
        );
        assert_eq!(
            provision(&reg, PROD, "oops"),
            Err(AppError::Cfg(CfgError::Parse("size")))
        );
        assert_eq!(
            provision(&reg, PROD, "999"),
            Err(AppError::Cfg(CfgError::Bounds(1, 100)))
        );
        // And an error this function raises itself, with no conversion involved:
        assert_eq!(
            provision(&reg, DEV, "50"),
            Err(AppError::Io("no pools to resize"))
        );
    }
}
