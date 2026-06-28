---
description: "Systematic debugging approach"
auto_evolve: true
evolve_min_confidence: 0.7
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Debugging

When debugging an issue, follow this systematic process.

## Phase 1: Reproduce
- Reconstruct the exact conditions: inputs, environment, sequence of operations.
- Check if the issue is deterministic or intermittent.
- If you cannot reproduce, ask the user for more details.

## Phase 2: Isolate
- Bisect the problem space: which component, which code path, which input triggers it?
- Add targeted logging (`tracing::info!`) at the suspected boundary, not everywhere.
- Check git log / git blame for recent changes in the affected area.

## Phase 3: Hypothesize
- Form a specific, falsifiable hypothesis about the root cause.
- Predict what you would observe if the hypothesis is correct.
- Don't jump to "it's a compiler bug" or "it's a library issue" without evidence.

## Phase 4: Verify
- Write a minimal reproduction case if possible.
- Add a test that fails under the current behavior and passes under the fix.
- Check edge cases: empty input, boundary values, race conditions.

## Phase 5: Fix
- Fix the root cause, not the symptom.
- Verify the fix doesn't break existing tests (`cargo test`).
- If the fix touches a hot path, consider performance implications.
