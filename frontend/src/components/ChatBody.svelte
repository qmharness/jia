<script lang="ts">
  import { store } from '../lib/store.svelte';
  import EmptyState from './EmptyState.svelte';
  import MessageGroup from './MessageGroup.svelte';
  import type { ChatEntry } from '../lib/types';

  interface Group {
    role: 'user' | 'assistant' | 'tool';
    entries: ChatEntry[];
  }

  let sentinel: HTMLDivElement | undefined = $state();

  // ── Virtual scroll ──────────────────────────────────────────
  const VIRTUAL_THRESHOLD = 100;
  const OVERSCAN = 3;
  const ESTIMATED_HEIGHT = 100;

  let visibleStart = $state(0);
  let visibleEnd = $state(30);
  let scrollRAF: number | null = null;

  let groupHeights: number[] = $state([]);
  let groupRefs: Map<number, HTMLElement> = new Map();

  function registerGroupEl(idx: number, el: HTMLElement | null) {
    if (el) {
      groupRefs.set(idx, el);
      const heights = [...groupHeights];
      heights[idx] = el.offsetHeight;
      groupHeights = heights;
    } else {
      groupRefs.delete(idx);
    }
  }

  $effect(() => {
    const observer = new ResizeObserver((entries) => {
      const heights = [...groupHeights];
      let changed = false;
      for (const entry of entries) {
        const el = entry.target as HTMLElement;
        for (const [idx, refEl] of groupRefs) {
          if (refEl === el && heights[idx] !== el.offsetHeight) {
            heights[idx] = el.offsetHeight;
            changed = true;
            break;
          }
        }
      }
      if (changed) groupHeights = heights;
    });
    for (const el of groupRefs.values()) observer.observe(el);
    return () => observer.disconnect();
  });

  const groups = $derived(buildGroups(store.entries));
  const shouldVirtualize = $derived(groups.length > VIRTUAL_THRESHOLD);

  const visibleGroups = $derived(
    shouldVirtualize
      ? groups.slice(
          Math.max(0, visibleStart - OVERSCAN),
          Math.min(groups.length, visibleEnd + OVERSCAN)
        )
      : groups
  );

  const topSpacerHeight = $derived(
    shouldVirtualize
      ? getCumulativeHeight(Math.max(0, visibleStart - OVERSCAN))
      : 0
  );
  const bottomSpacerHeight = $derived(
    shouldVirtualize
      ? getCumulativeHeight(groups.length) - getCumulativeHeight(Math.min(groups.length, visibleEnd + OVERSCAN))
      : 0
  );

  function getCumulativeHeight(idx: number): number {
    if (idx <= 0) return 0;
    if (groupHeights.length === 0) return idx * ESTIMATED_HEIGHT;
    let sum = 0;
    for (let i = 0; i < Math.min(idx, groupHeights.length); i++) {
      sum += groupHeights[i] || ESTIMATED_HEIGHT;
    }
    if (idx > groupHeights.length) sum += (idx - groupHeights.length) * ESTIMATED_HEIGHT;
    return sum;
  }

  $effect(() => {
    const _len = store.entries.length;
    const _tick = store.scrollTick;
    void _len; void _tick;
    if (sentinel) {
      sentinel.scrollIntoView({ block: 'end' });
    }
  });

  function onScroll(e: Event) {
    if (groups.length <= VIRTUAL_THRESHOLD) return;
    if (scrollRAF !== null) return;
    scrollRAF = requestAnimationFrame(() => {
      scrollRAF = null;
      const el = e.target as HTMLElement;
      const scrollTop = el.scrollTop;
      const heights = groupHeights;
      if (heights.length === 0) {
        visibleStart = Math.floor(scrollTop / ESTIMATED_HEIGHT);
        visibleEnd = Math.min(groups.length, Math.ceil((scrollTop + el.clientHeight) / ESTIMATED_HEIGHT));
        return;
      }
      let acc = 0;
      for (let i = 0; i < heights.length; i++) {
        acc += heights[i] || ESTIMATED_HEIGHT;
        if (acc > scrollTop) { visibleStart = i; break; }
      }
      acc = 0;
      for (let i = 0; i < heights.length; i++) {
        acc += heights[i] || ESTIMATED_HEIGHT;
        if (acc > scrollTop + el.clientHeight) { visibleEnd = i + 1; break; }
      }
      if (visibleEnd > groups.length) visibleEnd = groups.length;
    });
  }

  function buildGroups(list: ChatEntry[]): Group[] {
    const result: Group[] = [];
    let current: Group | null = null;

    for (const entry of list) {
      const role = entry.role === 'tool_call' ? 'tool' : entry.role;
      if (!current || role !== current.role || role === 'tool') {
        current = { role: role as Group['role'], entries: [entry] };
        result.push(current);
      } else {
        current.entries.push(entry);
      }
    }

    return result;
  }
</script>

<div class="body" onscroll={onScroll}>
  {#if groups.length === 0 && Object.keys(store.cancels).length === 0}
    {#if store.loadingMessages}
      <div class="loading-state">
        <div class="loading-spinner"></div>
        <p class="loading-text">Loading conversation...</p>
      </div>
    {:else}
      <EmptyState />
    {/if}
  {:else}
    {#if shouldVirtualize}
      <div style="height: {topSpacerHeight}px; flex-shrink: 0;"></div>
    {/if}

    {#each visibleGroups as group, gi}
      <MessageGroup
        {group}
        isLast={gi === visibleGroups.length - 1 && (!shouldVirtualize || visibleEnd + OVERSCAN >= groups.length)}
      />
    {/each}

    {#if shouldVirtualize}
      <div style="height: {bottomSpacerHeight}px; flex-shrink: 0;"></div>
    {/if}
    <div class="sentinel" bind:this={sentinel}></div>
  {/if}
</div>

<style>
  .body {
    flex: 1;
    overflow-y: auto;
    padding: 20px;
    display: flex;
    flex-direction: column;
    max-width: 768px;
    width: 100%;
    margin: 0 auto;
  }

  .sentinel {
    height: 1px;
    flex-shrink: 0;
  }

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 40px 20px;
  }

  .loading-spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-text {
    font-size: 13px;
    color: var(--text-tertiary);
  }
</style>
