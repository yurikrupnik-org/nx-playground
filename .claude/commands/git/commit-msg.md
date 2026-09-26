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

**Footer**:

```text
🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude Sonnet 4.5 <noreply@anthropic.com>
```

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
