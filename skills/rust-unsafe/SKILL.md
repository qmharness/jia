---
description: "Rust unsafe code audit"
paths:
  - "**/*.rs"
---
# Rust Unsafe Code Audit

When reading or writing Rust files, apply these additional checks.

## Unsafe Block Requirements
- Every `unsafe { }` block must be as small as possible — isolate the unsafe operation.
- A `// SAFETY:` comment must precede every unsafe block, documenting:
  1. Which invariant the caller must uphold.
  2. Why the invariant is satisfied at this call site.
- Prefer safe abstractions: wrap unsafe code in a safe interface with documented invariants.

## FFI Boundaries
- `extern "C"` functions: verify ABI compatibility (calling convention, struct layout).
- Raw pointers across FFI: document ownership — who allocates, who frees.
- `unsafe impl Send/Sync`: must be justified with a comment explaining thread safety.

## Raw Pointer Usage
- Avoid raw pointers unless necessary for FFI or performance-critical custom data structures.
- When using raw pointers, document the provenance chain — where did the pointer come from?
- `.as_ptr()` and `.as_mut_ptr()` from slices/vecs — ensure the original allocation outlives the pointer.

## Transmute
- `std::mem::transmute` is a last resort. Prefer `bytemuck` or safe casts.
- Never transmute between types with different sizes without explicit justification.
