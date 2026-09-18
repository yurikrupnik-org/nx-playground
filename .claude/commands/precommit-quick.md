---
description: Quick pre-commit path (bundle + gates + commit message, no second review pass)
allowed-tools: Bash(just:*), Bash(git diff:*), Bash(git status:*)
---

# Quick Pre-Commit ⚡

The reduced path of `.claude/skills/precommit-review/SKILL.md`: pass 0 and
pass 1 only, **no independent pass**.

Legal only when `just review-bundle` reports the staged diff as `docs` and/or
`config` classes with no `source`/`test` file — that is the objective test, not
"it feels small". Anything else runs `/precommit`.

1. `just review-bundle` — check the class column. Any `source` or `test` row
   (or a hard stop) → stop and use `/precommit`.
2. Run the gates the bundle listed, and the proof gate for any generated file
   in the diff (`just tilt-check` / `k8s-check` / `proto-check`).
3. Author pass over the staged diff: does the prose still match what the code
   does, and does any claim in it cite a path that exists?
4. Commit with a Conventional Commits message. Never on `main`, never
   `--no-verify`.
