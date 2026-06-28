---
description: "Systematic approach to understanding new codebases"
auto_evolve: true
evolve_min_confidence: 0.7
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Codebase Exploration

When approaching an unfamiliar codebase, follow this structured process. Rushing to edit without understanding causes more harm than good.

## Pass 1: Orientation

- Read root-level marker files: `CLAUDE.md`, `AGENTS.md`, `README.md`, `CONTRIBUTING.md`.
- These define conventions, commands, architecture rules, and workflow expectations.
- Note the language, build system, package manager, and test framework.
- Identify the top-level directory structure and what each subtree contains.

## Pass 2: Module Map

- For each top-level directory, identify its purpose (one sentence).
- Find the main entry point (`main.rs`, `index.ts`, `run_agent.py`).
- Trace module declarations and imports to understand dependencies between modules.
- Build a mental map: what depends on what, what is shared vs isolated.

## Pass 3: Trace a Path

- Pick one concrete execution path (e.g., "user sends a message → agent responds").
- Follow it from entry point to exit, reading only the code on that path.
- Note the key types, function signatures, and data transformations along the way.
- This single path teaches you the framework's conventions better than reading random files.

## Search, Don't Scan

- Use `grep` and `find` for targeted searches, not broad file reading.
- Search for function names, type definitions, and error messages to trace logic.
- Prefer `git log --oneline` and `git blame` to understand WHY code exists, not just WHAT it does.

## Before Making Changes

- Verify your understanding by running existing tests.
- Check if similar changes exist elsewhere (git log for related commits).
- Form a hypothesis about how your change fits into the architecture.
