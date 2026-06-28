---
description: "Structured brainstorming for design and architecture decisions"
auto_evolve: true
---
# Creative Ideation

When facing an open-ended design or architecture question, use structured ideation to generate concrete options before choosing.

## Process

### 1. Understand the Context

- What problem are you solving? (one sentence)
- What are the constraints? (language, dependencies, performance, timeline)
- What's the user's mood or priority? (speed, correctness, simplicity, learning)

### 2. Apply a Constraint

Pick one constraint that matches the context. Constraints force creative thinking:

- **Solve your own itch** — what would YOU want as the user of this system?
- **Start at the punchline** — design the ideal outcome first, then work backward to the implementation.
- **Hostile UI** — what's the simplest possible interface, even if it feels too minimal?
- **Invert the assumption** — flip the most obvious design choice and see what emerges.
- **Compose, don't extend** — can this be built by combining existing pieces instead of adding new ones?
- **Delete, don't add** — what can be removed to make this simpler, not what can be added?

### 3. Generate

Produce 3 concrete options. For each:
- One-line summary
- Key trade-off (what you gain vs what you lose)
- Rough implementation sketch (2-3 sentences)

### 4. Recommend

Pick the best option with a one-sentence justification. Let the user decide — don't proceed without confirmation for architectural choices.

## When to Use

- Choosing between libraries or frameworks.
- Designing a new API, module boundary, or abstraction.
- Naming things (components, endpoints, configuration keys).
- Resolving a tension between simplicity and flexibility.
- Refactoring decisions where multiple approaches are reasonable.

**Skip when**: the choice is obvious, the user already specified the approach, or there's a clear convention in the codebase.