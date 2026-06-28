---
description: "When and how to delegate work to sub-agents"
auto_evolve: true
evolve_min_confidence: 0.85
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Subagent Delegation

You have access to the `delegate` tool for spawning sub-agents (Explore, Plan, and general-purpose types). Use them strategically to manage context and parallelism.

## Agent Types

- **Explore agent** — Read-only codebase exploration: find files by pattern, search for keywords, answer architecture questions. Never writes or edits code.
- **Plan agent** — Design implementation approaches. Returns step-by-step plans with file lists and architectural trade-offs. Never writes or edits code.
- **General-purpose agent** — Full capabilities for complex multi-step research tasks.

## When to Delegate

- **Cross-cutting searches**: finding all usages of a pattern across the codebase.
- **Parallel research**: when you need to explore two independent areas simultaneously.
- **Context management**: when a large codebase exploration would overflow your context window.
- **Second opinion**: use a Plan agent to independently validate your approach.

## When NOT to Delegate

- **Known targets**: you already know the exact file path to read.
- **Trivial operations**: single `grep`, one-file read, simple rename.
- **Direct edits**: you have the file open and know exactly what to change.
- **User interaction**: sub-agents cannot ask the user questions.

## Writing Effective Prompts

A subagent prompt must be self-contained — it has no prior conversation context:

1. **State the goal**: what are you trying to accomplish and why.
2. **Describe what you know**: relevant findings, ruled-out approaches, constraints.
3. **Specify the output**: format, word limit, level of detail expected.
4. **Set boundaries**: read-only vs can-edit, specific directories to search, files to exclude.

## Parallel Delegation

When launching multiple agents, send them in a single message for true parallel execution. They must be independent — no agent should depend on another's output.
