---
description: "Write implementation plans before coding"
auto_evolve: true
evolve_min_confidence: 0.7
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Writing Plans

Before implementing any non-trivial feature, write a plan. This prevents wasted effort and keeps you aligned with the user's intent.

## When to Plan

- New features or significant refactoring
- Changes spanning 3+ files
- Architectural decisions (library choice, pattern selection)
- Unclear requirements that need exploration first

**Skip planning for**: typo fixes, single-line changes, simple renames, trivial logging additions.

## Plan Structure

1. **Context** — why this change, what problem it solves
2. **Approach** — the chosen strategy and why (not all alternatives)
3. **Files** — list of files to create or modify, with a one-line purpose each
4. **Steps** — ordered implementation phases, each independently verifiable
5. **Verification** — how to test end-to-end (commands, expected outputs)

## Rules

- Write the plan to the plan file; do not start coding until approved.
- If requirements are ambiguous, ask the user before committing to an approach.
- Keep the plan concise enough to scan, detailed enough to execute.
- Reuse existing utilities and patterns before proposing new ones.
