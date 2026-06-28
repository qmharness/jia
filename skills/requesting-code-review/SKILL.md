---
description: "Pre-commit self-review pipeline before requesting human review"
auto_evolve: true
---
# Requesting Code Review

Before asking for human review or shipping, run this verification pipeline. It catches issues that automated checks miss.

## Pipeline

### 1. Security Scan

- Scan for secrets, credentials, tokens, API keys in the diff.
- Check for command injection vectors (unsanitized input passed to shell).
- Verify no `unsafe` blocks without safety comments (Rust).
- Check for path traversal, XSS, SQL injection patterns.

### 2. Test Gates

- Run `cargo test` (Rust) or `pnpm test` (TypeScript) on the changed files.
- Verify new behavior has tests, not just existing coverage.
- Check that tests fail for the right reason, not by accident.

### 3. Independent Reviewer

- Launch an Explore subagent with the full diff.
- Ask it to check: correctness, edge cases, security, performance, naming.
- The reviewer must not be the same context that wrote the code.

### 4. Fix Loop

- If issues found, fix the root cause (not the symptom).
- Re-run tests after each fix.
- Maximum 2 auto-fix cycles — if still failing, report the issue and ask for human judgment.

## Pre-Commit Checklist

- [ ] All tests pass on the changed files
- [ ] No secrets or credentials in the diff
- [ ] New behavior is tested
- [ ] Independent reviewer found no blocking issues
- [ ] Lint and format checks pass (`cargo fmt`, `oxfmt`, etc.)