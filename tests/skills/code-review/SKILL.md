---
name: code-review
description: Review code changes and provide feedback. Use when users ask for code review, checking pull requests, or analyzing code quality.
license: Apache-2.0
compatibility: Requires git installed
allowed-tools: Bash(git:*) Read
metadata:
  author: test
  version: "1.0"
---

# Code Review Skill

When reviewing code, follow these guidelines:

## Workflow

1. **Gather Context**: Understand what files are being reviewed
   - Use `git diff` to see changes
   - Read relevant files for context

2. **Analyze Code**: Look for:
   - Potential bugs
   - Performance issues
   - Security vulnerabilities
   - Code style inconsistencies
   - Missing error handling

3. **Provide Feedback**: Structure your review as:
   - Summary of changes
   - Issues found (if any)
   - Suggestions for improvement
   - Overall assessment

## Review Categories

- **Critical**: Must fix before merge
- **Warning**: Should fix, but not blocking
- **Suggestion**: Nice to have improvements

## Output Format

```
## Code Review Summary

### Files Changed
- file1.rs: +10/-5 lines
- file2.rs: +20/-10 lines

### Issues Found
1. [Critical] Description of critical issue
2. [Warning] Description of warning

### Suggestions
- Consider using X instead of Y for better performance

### Overall Assessment
[Approve/Request Changes/Comment]
```
