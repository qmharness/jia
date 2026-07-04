<script lang="ts">
  import { fetchFiles } from '../lib/api';

  let { rootPath = '' }: { rootPath?: string } = $props();

  let path = $state('');
  let entries = $state<Array<{name:string;path:string;isDir:boolean}>>([]);
  let loading = $state(false);
  let error = $state('');
  let prevRoot = $state('');

  let rootName = $derived(rootPath ? rootPath.split('/').pop() || rootPath : '');

  let breadcrumbs = $derived.by(() => {
    if (!rootPath) return [];
    if (!path || path === rootPath) return [{ name: rootName, path: rootPath }];
    const rel = path.slice(rootPath.length + 1);
    const parts = rel.split('/');
    const crumbs = [{ name: rootName, path: rootPath }];
    let acc = rootPath;
    for (const p of parts) {
      acc = `${acc}/${p}`;
      crumbs.push({ name: p, path: acc });
    }
    return crumbs;
  });

  async function load(dir: string) {
    if (!rootPath) return;
    loading = true;
    error = '';
    try {
      const result = await fetchFiles(dir || undefined, rootPath || undefined);
      if ('error' in result) {
        error = result.error;
        entries = [];
      } else if (result.type === 'directory') {
        entries = result.entries;
        path = dir;
      }
    } catch (e: any) {
      error = e?.message || 'Failed to load';
      entries = [];
    }
    loading = false;
  }

  function navigate(name: string) {
    if (!path) return;
    load(path + '/' + name);
  }
  function crumbClick(p: string) { load(p); }

  $effect(() => {
    if (rootPath !== prevRoot) {
      prevRoot = rootPath;
      if (rootPath) {
        path = rootPath;
        load(rootPath);
      } else {
        entries = []; path = ''; error = '';
      }
    }
  });
</script>

<div class="file-browser">
  <div class="fb-header">
    {#if breadcrumbs.length > 0}
      {#each breadcrumbs as crumb, i}
        {#if i > 0}
          <svg class="bc-chevron" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="9 18 15 12 9 6"/></svg>
        {/if}
        <button class="crumb" class:last={i === breadcrumbs.length - 1} onclick={() => crumbClick(crumb.path)}>
          {#if i === 0}
            <svg class="fb-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 6a2 2 0 0 1 2-2h5l2 2h9a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6z"/></svg>
          {/if}
          {crumb.name}
        </button>
      {/each}
    {:else}
      <span class="fb-title">{rootName || 'Files'}</span>
    {/if}
  </div>
  <div class="fb-list">
    {#if !rootPath}
      <span class="fb-empty">No project selected</span>
    {:else if loading}
      <span class="fb-empty">Loading...</span>
    {:else if error}
      <span class="fb-empty fb-error">{error}</span>
    {:else if entries.length === 0}
      <span class="fb-empty">Empty</span>
    {:else}
      {#each entries as e}
        <button class="fb-item" class:is-dir={e.isDir} onclick={() => e.isDir ? navigate(e.path) : null}>
          <svg class="fb-item-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            {#if e.isDir}
              <path d="M2 6a2 2 0 0 1 2-2h5l2 2h9a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6z"/>
            {:else}
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/>
            {/if}
          </svg>
          <span class="fb-name">{e.name}</span>
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  .file-browser {
    display: flex; flex-direction: column;
    height: 100%;
    background: var(--bg-secondary);
  }
  .fb-header {
    display: flex; align-items: center; flex-wrap: nowrap;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
    gap: 2px;
    flex-shrink: 0;
    overflow-x: auto;
  }
  .fb-title {
    font-size: 11px; font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase; letter-spacing: 0.5px;
    padding: var(--space-1) 0;
  }
  .bc-chevron {
    flex-shrink: 0; color: var(--text-tertiary);
    opacity: 0.5;
  }
  .crumb {
    display: flex; align-items: center; gap: 4px;
    font-size: 12px; color: var(--text-secondary);
    padding: var(--space-1) 4px; border-radius: 4px;
    white-space: nowrap;
    font-weight: 400;
  }
  .crumb:hover { background: var(--bg-tertiary); color: var(--text-primary); }
  .crumb.last { font-weight: 600; color: var(--text-primary); }
  .fb-icon { flex-shrink: 0; color: var(--accent); opacity: 0.7; }

  .fb-list {
    flex: 1; overflow-y: auto;
    padding: var(--space-1);
  }
  .fb-empty {
    font-size: 12px; color: var(--text-tertiary);
    padding: var(--space-5);
    display: block; text-align: center;
    line-height: 1.5;
  }
  .fb-error { color: var(--error); }
  .fb-item {
    display: flex; align-items: center; gap: var(--space-2);
    width: 100%; padding: var(--space-1) var(--space-2);
    font-size: 12px; color: var(--text-primary);
    border-radius: var(--radius-sm);
    text-align: left;
  }
  .fb-item:hover { background: rgba(0, 0, 0, .04); }
  .fb-item-icon { flex-shrink: 0; color: var(--accent); opacity: 0.7; }
  .fb-item:not(.is-dir) .fb-item-icon { color: var(--text-tertiary); opacity: 1; }
  .fb-name {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
</style>
