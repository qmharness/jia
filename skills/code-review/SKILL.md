---
description: "Rust code review checklist"
auto_evolve: true
evolve_min_confidence: 0.7
evolve_max_revisions_per_session: 3
evolve_reflection_threshold: 3
---
# Code Review

When reviewing Rust code, check for these issues systematically.

## Safety & Correctness
- `unsafe` blocks: every unsafe block must have a safety comment explaining the invariant it maintains.
- Panics: no `.unwrap()` or `.expect()` in production paths. Use `?` or proper error handling.
- Integer overflow: checked in debug, wrapping in release — verify intent is correct.
- Indexing: prefer `.get()` over `[]` when the index is not statically guaranteed.

## Error Handling
- Errors should propagate upward with context (use `.context()` or `.map_err()`).
- Don't silently swallow errors with `let _ = ...` without a comment explaining why.
- Thiserror derive for library errors, anyhow for application code.

## Concurrency
- `Arc<Mutex<T>>` lock ordering: avoid deadlocks by never holding two locks simultaneously.
- `tokio::spawn` requires `'static` lifetime — verify captured references.
- Channel senders should be cloned cheaply; prefer `mpsc::unbounded_channel` for events.

## Resource Management
- Files, connections, and handles must be properly closed (Drop is usually sufficient in Rust).
- Large allocations: watch for unbounded `Vec::push` in loops or long-lived collections.
- Database connections should use connection pooling (r2d2).

## Performance
- Avoid cloning large data structures (clone `Arc` instead of the inner value).
- Use `Cow<str>` when a value might or might not need allocation.
- Prefer `&str` over `String` for function parameters that only read.