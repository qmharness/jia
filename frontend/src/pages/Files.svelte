<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchFiles, fetchConfig } from '../lib/api';
  import type { FileContentResponse, FileListResponse } from '../lib/api';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let path = $state('');
  let workspaceRoot = $state('');
  let data = $state<FileListResponse | FileContentResponse | { error: string } | null>(null);
  let loading = $state(true);

  async function load(p?: string) {
    loading = true;
    try {
      data = await fetchFiles(p);
      if (data && 'type' in data && data.type === 'directory') path = p || '';
    } catch {
      showToast(t('files.loadFailed'), 'error');
    }
    loading = false;
  }

  onMount(() => { load(); });

  function goUp() {
    if (!path) return;
    const parts = path.split('/');
    parts.pop();
    load(parts.join('/') || undefined);
  }
</script>

<div class="page">
  <div class="header">
    <div class="header-left">
      <h2 class="title">{t('files.title')}</h2>
      {#if workspaceRoot}
        <span class="workspace-label" title={workspaceRoot + (path ? '/' + path : '')}>📁 {workspaceRoot}{path ? '/' + path : ''}</span>
      {/if}
    </div>
  </div>
  <div class="body">
    {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else if data && 'type' in data}
      {#if data.type === 'directory'}
        <div class="dir-list">
          {#if path}
            <button class="entry" onclick={goUp}>
              <span class="entry-icon">📁</span>
              <span class="entry-name">..</span>
            </button>
          {/if}
          {#each data.entries as item}
            <button class="entry" onclick={() => load(item.path)}>
              <span class="entry-icon">{item.isDir ? '📁' : '📄'}</span>
              <span class="entry-name">{item.name}</span>
            </button>
          {/each}
        </div>
      {:else if data.type === 'file'}
        <pre class="file-content">{data.content}</pre>
      {/if}
    {:else if data && 'error' in data}
      <p class="msg error">{data.error}</p>
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    display: flex; align-items: center; gap: 12px;
    padding: 12px 20px; border-bottom: 1px solid var(--border);
  }
  .header-left { display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: 16px; font-weight: 600; }
  .workspace-label { font-size: 11px; color: var(--text-tertiary); font-family: monospace; }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }
  .msg.error { color: var(--error); }
  .dir-list { display: flex; flex-direction: column; gap: 2px; }
  .entry {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 10px; border-radius: var(--radius-sm);
    text-align: left; width: 100%;
    transition: background .15s;
  }
  .entry:hover { background: var(--bg-secondary); }
  .entry-icon { font-size: 18px; }
  .entry-name { font-size: 14px; color: var(--text-primary); }
  .file-content {
    background: var(--bg-secondary); padding: 14px; border-radius: var(--radius-md);
    font-size: 13px; font-family: monospace; white-space: pre-wrap; overflow-x: auto;
  }
</style>
