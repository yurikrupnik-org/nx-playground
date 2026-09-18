---
description: Run quality checks via just flows (cargo-native for Rust, nx for web)
allowed-tools: Bash(just:*), Bash(git diff:*)
---

# Quality Checks

Run the repo's composite gates. Rust goes through cargo's own workspace
orchestration (one `--workspace` run), web through nx — do NOT use
`nx run-many` for Rust targets (benchmarked 3–8x slower: per-crate cargo
processes serialize on the target-dir lock, and nx cache can't fingerprint
the shared `dist/target`).

## Standard gate (lint + build + test, all ecosystems)

```bash
just check
```

## Full pre-push gate (check + proto lint + OSV scan)

```bash
just verify
```

## Quick iteration (no tests/audit)

```bash
just check-quick
```

**Critical**: Stop if any gate fails. `just fix` auto-formats (rust fmt +
cargo sort + proto + biome --write) and re-runs the full gate.
