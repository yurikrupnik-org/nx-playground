---
description: Generate conventional commit message from staged changes
allowed-tools: Bash(git status:*), Bash(git diff:*), Bash(git log:*)
---

# Generate Commit Message

## Current Changes

- Staged files: !`git diff --cached --stat`
- Changes detail: !`git diff --cached`
- Recent commits: !`git log --oneline -5`

## Task

Generate a conventional commit message following this format:

**First line** (50 chars or less):

```text
<type>: <brief summary>
```

Types: feat, fix, refactor, docs, test, chore, perf, style

**Body** (if needed):

- Detailed description of changes
- Why the change was made
- Any important context

**Footer** — attribution follows the `Assisted-by` policy (tools/authorship):

- Do NOT add `Co-Authored-By:`, a "Generated with" line, or any model name.
- The lefthook `prepare-commit-msg` hook (`tools/authorship/stamp.ts`) appends
  `Assisted-by: <agent-id>` when the commit is made from an agent shell
  (Claude Code → `claude-code`); the `commit-msg` hook rejects an agent commit
  without it and any `Assisted-by` value that is not a registered agent id
  (`libs/core/authorship/agents.json`). The user runs the commit from their
  own shell, where the hook stamps nothing: when the staged change was written
  with Claude, end the message with the single trailer
  `Assisted-by: claude-code` (agent id only, no model) — the hook never
  duplicates it.
- If the hooks are not installed (`git commit` is denied with that reason),
  run `lefthook install` — never bypass with `--no-verify` or `LEFTHOOK=0`.

## Output Format

Present the commit message in a code block for easy copying:

```text
<commit message here>
```

Then show the command to use it:

```bash
git commit -m "..."
```

**Note**: User will review and approve before committing.
