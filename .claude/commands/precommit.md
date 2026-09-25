---
description: Two-pass pre-commit review (author pass + independent pass) then commit
allowed-tools: Bash(task:*), Bash(git diff:*), Bash(git status:*), Bash(coderabbit:*), Bash(codex:*), Bash(gemini:*)
---

# Pre-Commit Review

Run the protocol in `.claude/skills/precommit-review/SKILL.md` — read it first;
it is the single source of truth for the steps, the reconciliation rules and
the report format. Do not re-invent a review flow here.

Short form:

1. `task review-bundle` — freezes the STAGED diff into `dist/review/bundle.md`
   (file table, class per file, the gates that diff implies). Exit 1 = hard
   stop (secret-shaped path); exit 2 = nothing staged.
2. Run the gates the bundle listed, plus any proof gate for generated output.
   Red gate → fix and re-bundle before any model reads the diff.
3. **Pass 1 (author)** — this model, against the final staged state, using the
   six-point checklist in the skill.
4. **Pass 2 (independent, concurrent, blind to pass 1)** — `reviewer` and
   `security-reviewer` subagents plus the external CLIs that are actually
   authenticated (`coderabbit review --uncommitted --agent -c AGENTS.md`,
   `codex exec review --uncommitted`). Minimum two components, one a subagent,
   or the review is reported as degraded.
5. **Reconcile** — a claim raised by ≥2 components is blocker-eligible; single
   sources are judged on evidence; the skill's known-false-positive list is
   rejected with a reason, never "fixed".
6. Fix blockers, re-stage, re-run the bundle + gates, author pass over the
   delta only. Max two rounds.
7. Commit with a Conventional Commits message whose body names what the review
   changed. Never on `main`, never `--no-verify`.

For a docs-only or config-only diff (the bundle says so), `/precommit-quick`
skips pass 2.
