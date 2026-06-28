---
description: "Git commit and pull request workflow"
auto_evolve: true
evolve_min_confidence: 0.7
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Git Workflow

Follow this workflow for all git operations. Consistency prevents lost work and keeps history readable.

## Before Committing

1. Run `git status` to see what changed.
2. Run `git diff` to review all changes (staged and unstaged).
3. Never commit secrets, `.env` files, or large binaries.
4. Stage specific files with `git add <file>`, not `git add -A`.

## Commit Messages

- Format: conventional commits — `type: brief description`
- Types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`
- Describe WHY, not WHAT. The diff shows what changed; the message explains the intent.
- Keep the subject line under 72 characters.
- Example: `fix: case-insensitive tool lookup for LLM casing variance`

## Branching

- No merge commits on `main`. Rebase on latest `origin/main` before pushing.
- `git pull --rebase` before pushing to avoid unnecessary merge commits.
- Never force-push to `main` or `master`. Warn the user if they request it.

## PR Creation

- Create PRs with a real body: Summary (what changed and why) + Verification (how tested).
- Reference related issues with `#number`.
- Do not skip CI hooks (`--no-verify`, `--no-gpg-sign`) unless the user explicitly asks.

## User Commands

- `commit` — commit YOUR changes only.
- `commit all` — commit ALL changes in grouped chunks.
- `push` — may `git pull --rebase` first automatically.
- `ship it` — changelog if needed, commit, pull --rebase, push.
