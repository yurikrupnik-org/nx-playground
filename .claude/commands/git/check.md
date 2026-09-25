---
description: Run quality checks via task flows (cargo-native for Rust, nx for web)
allowed-tools: Bash(task:*), Bash(git diff:*)
---

# Quality Checks

Run the repo's composite gates. Rust goes through cargo's own workspace
orchestration (one `--workspace` run), web through nx — do NOT use
`nx run-many` for Rust targets (benchmarked 3–8x slower: per-crate cargo
processes serialize on the target-dir lock, and nx cache can't fingerprint
the shared `dist/target`).

## Standard gate (lint + build + test, all ecosystems)

```bash
task check
```

## Full pre-push gate (check + proto lint + OSV scan)

```bash
task verify
```

## Quick iteration (no tests/audit)

```bash
task check-quick
```

**Critical**: Stop if any gate fails. `task fix` auto-formats (`task fmt`: rust
fmt + cargo sort + buf + biome --write + rumdl + typos) and re-runs the full gate.
