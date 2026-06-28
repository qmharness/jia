---
description: "Test-driven development: RED-GREEN-REFACTOR"
paths:
  - "**/*.rs"
  - "**/*.test.ts"
---
# Test-Driven Development

Follow the RED-GREEN-REFACTOR cycle. Never skip a phase.

## RED — Write a Failing Test

- Write the smallest test that captures the desired behavior.
- The test must fail for the right reason (not a syntax error or missing import).
- Test the behavior, not the implementation. What should happen, not how.
- Name tests descriptively: `test_<scenario>_<expected_outcome>`.

**Rust conventions:**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input_returns_default() {
        assert_eq!(parse(""), Ok(Default::default()));
    }
}
```

**TypeScript conventions (Vitest):**
```typescript
import { describe, it, expect } from 'vitest';

describe('parse', () => {
  it('returns default for empty input', () => {
    expect(parse('')).toEqual(defaultValue);
  });
});
```

## GREEN — Make It Pass

- Write the minimum code to make the test pass. No more.
- Don't add error handling, optimization, or abstraction the test doesn't demand.
- If the minimal code feels hacky, that's fine — REFACTOR handles that next.

## REFACTOR — Clean Up

- Eliminate duplication introduced in GREEN.
- Improve names, extract helpers, simplify conditionals.
- Keep all tests green throughout.
- Don't add new behavior during refactoring — that belongs in the next RED phase.

## Edge Cases

After the happy path, add tests for:
- Empty / null / zero inputs
- Boundary values (max, min, off-by-one)
- Error conditions
- Concurrent access (if applicable)
