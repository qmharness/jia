---
description: "Throwaway experiments to validate feasibility before building"
auto_evolve: true
---
# Spike

A spike is a disposable experiment to answer a specific feasibility question before committing to implementation. Throw the code away afterward.

## When to Spike

- Unsure whether an approach works before committing to it.
- Evaluating a new library or API you haven't used before.
- Estimating the effort or complexity of a design proposal.
- The cost of getting it wrong is higher than the cost of a quick experiment.

**Skip when**: the answer is verifiable by reading docs, the approach is well-understood, or the change is trivial.

## Process

### 1. Decompose

Break the uncertainty into specific, falsifiable questions. Each question should be answerable with a yes/no or a concrete measurement.

### 2. Research

Read upstream docs, source, and types for any dependency. Understand the contract before testing it. No guessing about API behavior, defaults, errors, or timing.

### 3. Build

Create a minimal, isolated prototype in a standalone directory. Use `shell` or `write_file` to create a self-contained test case. Keep it small — a spike that grows beyond ~50 lines is no longer a spike.

### 4. Verdict

Answer each question with evidence from the prototype:
- **Feasible** — the approach works, document the key findings for the real implementation plan.
- **Blocked** — something prevents it (missing API, wrong abstraction). Report what blocks it.
- **Workaround** — possible but needs a different path. Document the trade-off.

## Rules

- Spikes are throwaway code. Never commit them or build production logic on top of them.
- One spike per uncertainty. Don't bundle multiple questions into one experiment.
- Share the verdict before proceeding — don't assume the result validates your preferred approach.