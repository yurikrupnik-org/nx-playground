---
description: Get Gemini AI code review of staged changes
allowed-tools: Bash(git diff:*), Bash(gemini:*)
---

# Gemini Code Review

Review staged changes using Gemini AI:

!`git diff --cached | gemini "Review this code change for: 1) Code quality and best practices, 2) Performance issues, 3) Potential bugs, 4) Maintainability concerns. Provide specific, actionable feedback."`

## Summary

Summarize Gemini's feedback:

- Critical issues (must fix before commit)
- Suggestions (nice to have improvements)
- Positive feedback (what's done well)

Categorize by severity: 🔴 Critical, 🟡 Warning, 🟢 Good
