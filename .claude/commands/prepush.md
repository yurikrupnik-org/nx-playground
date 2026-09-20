---
description: Pre-push checks (branch validation, remote sync, final quality checks)
allowed-tools: Bash(git:*)
---

# Pre-Push Workflow 🚀

Run checks before pushing to remote. Ensures branch safety and code quality.

## Step 1: Branch Validation 🔒

Check current branch and status:

!`git branch --show-current`
!`git status`

**Verify:**

- Not on `main` or `master` (unless intentional)
- No uncommitted changes
- All changes are committed

**Stop if:**

- Pushing to main/master without confirmation
- Uncommitted changes exist

## Step 2: Remote Sync Check 📡

Check if branch is up to date with remote:

!`git fetch origin`
!`git status`

**Check for:**

- "Your branch is behind" → Need to pull first
- "Your branch is ahead" → Safe to push
- "Your branch has diverged" → Need to resolve

**Stop if:**

- Branch is behind remote (run `git pull --rebase` first)
- Branch has diverged (resolve conflicts first)

## Step 3: Final Quality Checks ⚙️

Run `/git:check` to verify:

- Lint passes
- Build succeeds
- Tests pass

**Critical**: Stop if any checks fail.

## Step 4: Commit History Review 📝

Show commits about to be pushed:

!`git log origin/$(git branch --show-current)..HEAD --oneline`

If no upstream branch exists:
!`git log --oneline -5`

**Review:**

- Commit messages are clear
- No sensitive data in commits
- Commits are logical and atomic

## Step 5: Push Instructions ✅

If all checks pass:

**To push:**

```bash
# Push current branch
git push

# Or if new branch (set upstream)
git push -u origin $(git branch --show-current)
```

**Safety reminders:**

- ⚠️ Never force push to main/master
- ⚠️ Use `git push --force-with-lease` if force push needed
- ✅ Create PR/MR after pushing
- ✅ Tag reviewers

---

**Workflow Summary:**

1. 🔒 Branch validation (not main/master, no uncommitted changes)
2. 📡 Remote sync (fetch, check status)
3. ⚙️ Quality checks (lint, build, test)
4. 📝 Commit history review
5. ✅ Safe to push

**Pro tip**: Always fetch before pushing to avoid conflicts!
