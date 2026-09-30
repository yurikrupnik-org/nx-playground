//! `Option` combinators: what each one is for, and when to reach for it.
//!
//! Prose version: `docs/rust-option-result-methods.md` at the repo root.
//! The `Result` half lives in [`crate::result_methods`].

use crate::domain::{Autoscaler, CfgError, Cluster, Registry};

/// The canonical `and_then`: the closure itself returns an `Option`.
pub fn autoscaler_of<'a>(reg: &'a Registry, key: &str) -> Option<&'a Autoscaler> {
    reg.get(key).and_then(|c| c.autoscaler.as_ref())
}

/// Same walk, but every failure keeps its name. `ok_or` at each optional step.
pub fn autoscaler_or_why<'a>(reg: &'a Registry, key: &str) -> Result<&'a Autoscaler, CfgError> {
    reg.get(key)
        .ok_or(CfgError::Missing("cluster"))
        .and_then(|c| c.autoscaler.as_ref().ok_or(CfgError::Missing("autoscaler")))
}

/// A cluster worth deploying to: present *and* non-empty. `filter` treats
/// "present but useless" exactly like "absent".
pub fn deployable<'a>(reg: &'a Registry, key: &str) -> Option<&'a Cluster> {
    reg.get(key).filter(|c| !c.pools.is_empty())
}

#[cfg(test)]
mod tests {
    // The workspace runs with -W clippy::unwrap-used; unwrapping to assert on a
    // known error is the point here, so opt out for this module only.
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::domain::parse_size;
    use crate::fixtures::{DEV, MISSING, PROD, registry};

    // ---------------------------------------------------------------- map
    // Transform the value inside. The closure returns a plain T, never an Option.
    // Nesting level stays the same: Option<A> -> Option<B>.
    //
    // USE WHEN the transform cannot fail and cannot be absent:
    //   - projecting a field out of a struct you just looked up  (c -> c.region)
    //   - changing representation                                (String -> &str, u32 -> Duration)
    //   - `.map_err(...)` on a Result: same idea on the error channel, e.g. wrapping
    //     a std::io::Error into your own CfgError at an API boundary
    // DON'T USE WHEN the closure returns Option/Result — you get Option<Option<_>>
    // (see `flatten`) or Result<Result<_>>. That's `and_then`'s job.
    #[test]
    fn map_transforms_the_inside() {
        let reg = registry();

        let pool_count: Option<usize> = reg.get(PROD).map(|c| c.pools.len());
        let region: Option<&str> = reg.get(PROD).map(|c| c.region.as_str());

        assert_eq!(pool_count, Some(2));
        assert_eq!(region, Some("eu"));
        // A miss short-circuits: the closure never runs.
        assert_eq!(reg.get(MISSING).map(|c| c.pools.len()), None);
    }

    // ----------------------------------------------------------- and_then
    // Chain an operation that *itself* returns an Option. Option<A> -> Option<B>,
    // flattening as it goes. This is `flatMap` / monadic bind.
    //
    // USE WHEN the next step is itself fallible/optional — each link can bail:
    //   - walking nested optional data     (registry -> cluster -> autoscaler)
    //   - lookups that chain               (map.get(k).and_then(|id| other.get(id)))
    //   - fallible-by-nature APIs          (.first(), .last(), .get(i), .checked_add,
    //                                       .strip_prefix, .parse().ok(), env::var().ok())
    //   - sequencing validation on Result  (parse -> bounds-check -> persist)
    // In a function that already returns Option/Result, `?` says the same thing
    // and reads better; and_then is for expression position — inside another
    // closure, an iterator chain (`.filter_map(|k| reg.get(k).and_then(..))`),
    // or a `let` binding like this one.
    // COST: on Option, every failure collapses to a bare None — you lose *which*
    // link broke. If the caller needs to know, switch to Result via `ok_or` first.
    #[test]
    fn and_then_chains_another_option() {
        let reg = registry();

        // The closure returns Option<&Autoscaler> — that is what makes and_then the
        // right tool. Two "maybe"s collapse into one.
        assert_eq!(
            autoscaler_of(&reg, PROD),
            Some(&Autoscaler { min: 1, max: 20 })
        );
        assert_eq!(autoscaler_of(&reg, DEV), None); // cluster found, but it has no autoscaler
        assert_eq!(autoscaler_of(&reg, MISSING), None); // cluster isn't found at all
        // Note both misses give the same None — and_then erases *why* it failed.
        // autoscaler_or_why() is the same walk with the reasons kept; see the
        // ok_or test below.

        // let s = reg.get(PROD).and_then(|a| a.name.as_str());

        // Chain further: and_then composes indefinitely.
        let first_pool_size: Option<u32> =
            reg.get(PROD).and_then(|c| c.pools.first()).map(|p| p.size);
        assert_eq!(first_pool_size, Some(3));
    }

    // and_then whose closure returns `Some(x)` unconditionally is just map.
    // The allowing below is the proof: clippy::bind_instead_of_map catches this
    // exact shape for you (`cargo clippy --fix` rewrites it to map).
    #[test]
    #[allow(clippy::bind_instead_of_map)]
    fn and_then_with_unconditional_some_is_map() {
        let reg = registry();

        let via_and_then = reg.get(PROD).and_then(|c| Some(c.pools.len())); // don't
        let via_map = reg.get(PROD).map(|c| c.pools.len()); // do

        assert_eq!(via_and_then, via_map);
        // Rule of thumb: closure returns Option -> and_then. Closure returns T -> map.
    }

    // ------------------------------------------------------------- filter
    // Keep Some only if the predicate holds. Option-only; the closure gets &T.
    //
    // USE WHEN "present but useless" should be treated exactly like "absent":
    //   - reject empty/placeholder values   (env::var("X").ok().filter(|s| !s.is_empty()))
    //   - a cache entry that has expired    (entry.filter(|e| e.fresh_at > now))
    //   - guard before an expensive step    (.filter(|c| c.region == "eu").map(deploy))
    //   - narrow before defaulting          (.filter(|n| *n > 0).unwrap_or(1))
    // Equivalent to `.and_then(|x| cond.then_some(x))`, just clearer.
    // NO Result::filter exists — on Result the analogue is
    // `.and_then(|x| if cond { Ok(x) } else { Err(..) })`, because dropping a
    // value there means inventing an error for it.
    #[test]
    fn filter_drops_values_that_fail_a_predicate() {
        let reg = registry();
        assert!(deployable(&reg, PROD).is_some());
        assert!(deployable(&reg, DEV).is_none()); // present, but filtered out
        assert!(deployable(&reg, MISSING).is_none()); // never there to begin with
    }

    // ------------------------------------------------------------ inspect
    // Side effect (log, metric, dbg) without touching the value or the type.
    //
    // USE WHEN you want to observe a pipeline without breaking it apart:
    //   - tracing a chain           (.inspect(|c| tracing::debug!(?c, "found cluster")))
    //   - counters / metrics        (.inspect(|_| CACHE_HITS.inc()))
    //   - `.inspect_err(|e| warn!(...))` on Result: log the failure, still return it
    //     to the caller — the single best use of this family
    // Beats a temporary `let` + `if let` because the chain stays a chain, and it
    // beats `map(|x| { log(x); x })` because it can't accidentally change the type.
    // Requires Rust 1.76+ for Option/Result::inspect.
    #[test]
    fn inspect_peeks_without_changing_anything() {
        let reg = registry();
        let mut seen = Vec::new();

        let out = reg
            .get(PROD)
            .inspect(|c| seen.push(c.name.clone()))
            .map(|c| c.pools.len());

        assert_eq!(out, Some(2));
        assert_eq!(seen, vec![PROD.to_string()]);

        // Skipped entirely on None.
        let mut skipped = 0;
        let _ = reg.get(MISSING).inspect(|_| skipped += 1);
        assert_eq!(skipped, 0);
    }

    // --------------------------------------------------- ok_or / ok_or_else
    // Option -> Result: attach the reason the value was missing.
    // ok_or takes a value (always built); ok_or_else takes a closure (lazy).
    //
    // USE WHEN absence must reach the caller as a diagnosable failure:
    //   - the boundary of a fn returning Result — converts an Option so `?` works
    //   - config / lookup misses that need a name  ("cluster" vs "autoscaler")
    //   - HTTP handlers: missing row -> 404 with a message, not a silent None
    // PICK ok_or for a cheap unit-ish variant (CfgError::Missing("cluster")) —
    // it is constructed on every call, including the Ok path.
    // PICK ok_or_else when the error allocates or formats — clippy's `or_fun_call`
    // lint flags the eager version for exactly this reason.
    // `.ok()` is the inverse: Result -> Option, discarding the error.
    #[test]
    fn ok_or_gives_the_absence_a_name() {
        let reg = registry();

        assert!(reg.get(PROD).ok_or(CfgError::Missing("cluster")).is_ok());
        assert_eq!(
            reg.get(MISSING)
                .ok_or(CfgError::Missing("cluster"))
                .unwrap_err(),
            CfgError::Missing("cluster")
        );

        // Use ok_or_else when the error is expensive to construct. This particular
        // error isn't, so clippy::unnecessary_lazy_evaluations flags the closure —
        // that lint is exactly the eager/lazy decision, enforced.
        #[allow(clippy::unnecessary_lazy_evaluations)]
        let lazy = reg.get(MISSING).ok_or_else(|| CfgError::Missing("cluster"));
        assert!(lazy.is_err());

        // This is how you recover the distinction and_then threw away — same walk
        // as autoscaler_of(), but the two failures are now different values.
        assert_eq!(
            autoscaler_or_why(&reg, DEV).unwrap_err(),
            CfgError::Missing("autoscaler")
        );
        assert_eq!(
            autoscaler_or_why(&reg, MISSING).unwrap_err(),
            CfgError::Missing("cluster")
        );
    }

    // --------------------------------------------- unwrap_or / unwrap_or_else
    // Leave the Option/Result world with a fallback. unwrap_or's argument is
    // evaluated eagerly, unwrap_or_else's closure only on the empty branch.
    //
    // USE WHEN the caller genuinely doesn't care that the value was missing:
    //   - config defaults        (max_conns.unwrap_or(10))
    //   - counters / aggregates  (count.unwrap_or(0), name.unwrap_or_default())
    //   - the very end of a pipeline, where you must hand back a concrete T
    // PICK unwrap_or for a literal/Copy value; unwrap_or_else when the fallback
    // allocates, reads env, or hits disk — and on Result it also hands you the
    // error (|e| { warn!(?e); Fallback::new() }), which unwrap_or cannot.
    // unwrap_or_default() is the shorthand when T: Default.
    // DON'T USE to paper over a failure the caller should handle — that's what
    // ok_or/`?` are for. A default silently turns "broken config" into "0".
    #[test]
    fn unwrap_or_supplies_a_default() {
        let reg = registry();

        let count = reg.get(MISSING).map(|c| c.pools.len()).unwrap_or(0);
        assert_eq!(count, 0);

        let mut built = 0;
        let name = reg.get(MISSING).map(|c| c.name.clone()).unwrap_or_else(|| {
            built += 1;
            "unknown".to_string()
        });
        assert_eq!(name, "unknown");
        assert_eq!(built, 1); // the eager form would have allocated on every call

        // unwrap_or_default() when T: Default — the same thing, less typing.
        let region: String = reg
            .get(MISSING)
            .map(|c| c.region.clone())
            .unwrap_or_default();
        assert_eq!(region, "");
    }

    // ------------------------------------------------- map_or / map_or_else
    // Transform *and* default in one step: map + unwrap_or fused.
    // Argument order is (default, transform) — the default comes first.
    //
    // USE WHEN you need a plain T out of an Option/Result and the two branches
    // produce different shapes:
    //   - metrics/reporting      (cluster.map_or(0, |c| c.pools.len()))
    //   - rendering              (user.map_or("anonymous", |u| u.name.as_str()))
    //   - map_or_else on Result is the display idiom: one closure formats the
    //     error, the other formats the value, both end up as String
    // It's just `.map(f).unwrap_or(d)` — reach for it when that chain reads
    // clumsily, and remember the reversed argument order (default first, unlike
    // every other method here).
    #[test]
    fn map_or_transforms_or_falls_back() {
        let reg = registry();

        let a = reg.get(PROD).map_or(0, |c| c.pools.len());
        let b = reg.get(MISSING).map_or(0, |c| c.pools.len());
        assert_eq!((a, b), (2, 0));

        // map_or_else: both branches lazy. On Result the first closure gets the error.
        let label = reg
            .get(MISSING)
            .map_or_else(|| "none".to_string(), |c| c.region.clone());
        assert_eq!(label, "none");

        let msg = parse_size("x").map_or_else(|e| format!("{e:?}"), |n| n.to_string());
        assert_eq!(msg, r#"Parse("size")"#);
    }

    // -------------------------------------------- is_some_and / is_ok_and
    // Collapse straight to bool. Nothing survives the call.
    //
    // USE WHEN the answer itself is the boolean and you don't need the value:
    //   - `if` / `while` conditions     (if cluster.is_some_and(|c| c.spot_only()) {..})
    //   - iterator predicates           (.filter(|k| reg.get(k).is_some_and(|c| c.ready)))
    //   - assertions and guard clauses
    // Replaces `matches!(opt, Some(c) if c.ready)` and
    // `opt.map(|c| c.ready).unwrap_or(false)` — same meaning, less noise.
    // Absent and predicate-false both give `false`, so don't use it where those
    // two cases need different handling.
    // Siblings: is_none_or (1.82+), is_err_and. Rust 1.70+ for these two.
    #[test]
    fn is_some_and_answers_a_yes_no_question() {
        let reg = registry();

        assert!(reg.get(PROD).is_some_and(|c| c.pools.len() > 1));
        assert!(!reg.get(DEV).is_some_and(|c| c.pools.len() > 1)); // present, predicate fails
        assert!(!reg.get(MISSING).is_some_and(|c| c.pools.len() > 1)); // absent
        // Same shape as `.filter(..).is_some()`, but reads as a question.

        // is_none_or is the mirror: vacuously true when absent. "If there is a
        // limit, we're under it."
        assert!(reg.get(MISSING).is_none_or(|c| c.pools.is_empty()));
        assert!(reg.get(DEV).is_none_or(|c| c.pools.is_empty()));
    }

    // ------------------------------------------------------------ flatten
    // Peel one layer off Option<Option<T>>. `map` + `flatten` == `and_then`.
    //
    // USE WHEN the double layer arrives from somewhere you don't control:
    //   - a struct field that is genuinely Option<Option<T>> (JSON "absent" vs
    //     explicit null in serde: #[serde(default, with = "double_option")])
    //   - collection APIs returning Option<Option<_>>  (map.get(k).cloned() over
    //     a HashMap<K, Option<V>>, iter().next() over optionals)
    //   - after a `map` you can't rewrite — flatten is the patch
    // If YOU wrote the `map` that produced the nesting, delete both and write
    // `and_then` instead.
    // Result::flatten is still unstable — for Result<Result<T, E>, E> use
    // `.and_then(std::convert::identity)`. Iterator::flatten is the same idea
    // one container over.
    #[test]
    fn flatten_is_and_then_split_in_two() {
        let reg = registry();

        let nested: Option<Option<&Autoscaler>> = reg.get(PROD).map(|c| c.autoscaler.as_ref());
        let flat: Option<&Autoscaler> = nested.flatten();

        assert_eq!(flat, autoscaler_of(&reg, PROD));
        // The nested type is the tell: if `map` hands you Option<Option<_>>,
        // you wanted and_then.
    }

    // ---------------------------------------------------------- transpose
    // Swap the layers: Option<Result<T, E>> <-> Result<Option<T>, E>.
    // "Optional field, but a malformed one is a hard error."
    //
    // USE WHEN an optional input needs fallible processing and you want `?`:
    //   - optional config/env values that must parse
    //     (env::var("PORT").ok().map(|s| s.parse()).transpose()?  -> Option<u16>)
    //   - optional CLI flags, optional JSON fields, nullable DB columns
    //   - optional headers/query params in a handler returning Result
    // Without it you're stuck matching three cases by hand; with it, absence
    // stays Ok(None) and only a *bad* value becomes Err.
    // Cousin: `collect::<Result<Vec<_>, _>>()` does this for many values at once.
    #[test]
    fn transpose_swaps_option_and_result() {
        let present: Option<&str> = Some("12");
        let absent: Option<&str> = None;
        let broken: Option<&str> = Some("twelve");

        let ok: Result<Option<u32>, CfgError> = present.map(parse_size).transpose();
        let skipped: Result<Option<u32>, CfgError> = absent.map(parse_size).transpose();
        let failed: Result<Option<u32>, CfgError> = broken.map(parse_size).transpose();

        assert_eq!(ok, Ok(Some(12)));
        assert_eq!(skipped, Ok(None)); // absent is fine
        assert_eq!(failed, Err(CfgError::Parse("size"))); // malformed is not
        // Now a single `?` propagates the parse error in a function returning Result.
    }

    // ---------------------------------------------------------------- zip
    // Both or nothing: Option<A> + Option<B> -> Option<(A, B)>.
    //
    // USE WHEN two independent optionals are only meaningful together:
    //   - paired coordinates / ranges   (min.zip(max).map(|(a, b)| a..b))
    //   - comparing two lookups         (reg.get(a).zip(reg.get(b)))
    //   - "both flags present" config   (user.zip(password).map(Credentials::new))
    // Replaces nested `if let Some(a) = .. { if let Some(b) = .. }` and reads
    // better than `a.and_then(|a| b.map(|b| (a, b)))`, which is what it does.
    // Doesn't exist on Result (no way to combine two errors) — for that, `?`
    // each one. Inverse: `unzip()`. Related: `or`/`or_else` for
    // either-one-will-do, `xor` for exactly-one.
    #[test]
    fn zip_requires_both_sides() {
        let reg = registry();

        let pair = reg.get(PROD).zip(reg.get(DEV));
        assert!(pair.is_some());

        let same_region = pair.map(|(a, b)| a.region == b.region);
        assert_eq!(same_region, Some(true));

        assert!(reg.get(PROD).zip(reg.get(MISSING)).is_none());

        // or / or_else: keep the first Some. The fallback-source idiom.
        let with_fallback = reg.get(MISSING).or_else(|| reg.get(PROD));
        assert!(with_fallback.is_some());
    }

    // ------------------------------------------------ the whole thing at once
    #[test]
    fn realistic_pipeline() {
        let reg = registry();

        // Same question, three different exits:
        let count: usize = reg.get(MISSING).map_or(0, |c| c.pools.len()); // default
        let any: bool = reg.get(MISSING).is_some_and(|c| !c.pools.is_empty()); // predicate
        let opt: Option<usize> = reg.get(MISSING).map(|c| c.pools.len()); // stay optional
        assert_eq!((count, any, opt), (0, false, None));

        // Where the Option world hands off to the Result world.
        let headroom: Option<u32> = autoscaler_of(&reg, PROD).map(|a| a.max - a.min);
        assert_eq!(headroom, Some(19));
    }
}
