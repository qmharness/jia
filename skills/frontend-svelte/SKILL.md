---
description: "Svelte 5 frontend patterns"
paths:
  - "**/*.svelte"
  - "**/*.ts"
  - "**/frontend/**"
---
# Svelte 5 Frontend Patterns

When working with frontend files, apply these Svelte 5 conventions.

## Reactivity (Svelte 5 Runes)
- Use `$state()` for reactive local variables, not `let`.
- Use `$derived(expr)` for computed values instead of `$: reactive = expr`.
- Use `$effect(() => { ... })` for side effects, with proper cleanup returns.
- `$state.snapshot()` when you need a plain JS object from a deeply reactive structure.

## Component Design
- One component per file. Extract reusable pieces rather than duplicating.
- Props are function parameters in Svelte 5: `let { prop1, prop2 }: Props = $props();`
- Avoid deeply nested ternaries — use `{#if}`, `{:else if}`, `{:else}` blocks.
- Bindings: use `bind:` for two-way binding to form elements; prefer one-way flow otherwise.

## Styling
- Scoped styles are default — keep component styles inside `<style>` tags.
- Use CSS custom properties for theming rather than hardcoded values.
- Responsive: test layouts at mobile (< 640px), tablet, and desktop breakpoints.

## Performance
- Avoid expensive computations in reactive statements that fire on every keystroke.
- Use `{#key}` blocks to force re-creation of elements when identity changes.
- Lazy-load heavy dependencies with dynamic `import()`.
