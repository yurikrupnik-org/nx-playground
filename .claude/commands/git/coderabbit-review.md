---
description: Get CodeRabbit AI code review of staged changes
allowed-tools: Bash(git diff:*), Bash(coderabbit:*)
---

# CodeRabbit Code Review

Review staged changes using CodeRabbit CLI:

Get the diff for review:
!`git diff --cached`

Run CodeRabbit review:
!`coderabbit review`

## Summary

Summarize CodeRabbit's feedback:

- Line-by-line comments and suggestions
- Code patterns and anti-patterns identified
- Best practice violations
- Refactoring opportunities

Categorize by severity: 🔴 Critical, 🟡 Warning, 🟢 Good
