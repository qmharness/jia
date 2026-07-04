<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchToolGroups } from '../lib/api';
  import type { ToolInfo } from '../lib/api';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let groups = $state<Array<{ category: string; tools: ToolInfo[] }>>([]);
  let loading = $state(true);
  let expanded = $state<Record<string, boolean>>({});

  let totalTools = $derived(groups.reduce((n, g) => n + g.tools.length, 0));

  function toggle(category: string) {
    expanded = { ...expanded, [category]: !expanded[category] };
  }

  onMount(async () => {
    try { groups = await fetchToolGroups(); } catch { showToast(t('tools.loadFailed'), 'error'); }
    loading = false;
  });
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('tools.title')}</h2>
    <span class="count">{t('tools.available', { n: totalTools })}</span>
  </div>
  <div class="body">
    {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else if groups.length === 0}
      <p class="msg">{t('tools.none')}</p>
    {:else}
      <div class="group-list">
        {#each groups as group}
          <div class="group">
            <button class="group-header" onclick={() => toggle(group.category)}>
              <span class="group-name">{t('tools.cat.' + group.category) || group.category}</span>
              <span class="group-count">{group.tools.length}</span>
              <span class="expand-icon">{expanded[group.category] ? '▾' : '▸'}</span>
            </button>
            {#if expanded[group.category]}
              <div class="group-body">
                {#each group.tools as tool}
                  <div class="tool-item">
                    <span class="tool-name">{tool.name}</span>
                    <p class="tool-desc">{tool.description}</p>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 20px; border-bottom: 1px solid var(--border);
  }
  .title { font-size: 16px; font-weight: 600; }
  .count { font-size: 13px; color: var(--text-tertiary); }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }
  .group-list { display: flex; flex-direction: column; gap: 12px; }
  .group { border: 1px solid var(--border); border-radius: var(--radius-md); overflow: hidden; }
  .group-header {
    display: flex; align-items: center; gap: 8px;
    width: 100%; padding: 10px 14px;
    font-size: 14px; font-weight: 600;
    background: var(--bg-secondary);
    transition: background .15s;
  }
  .group-header:hover { background: var(--bg-tertiary); }
  .group-name { flex: 1; text-align: left; }
  .group-count {
    font-size: 12px; color: var(--text-tertiary);
    background: var(--bg-primary); padding: 1px 8px; border-radius: 10px;
  }
  .expand-icon { font-size: 12px; color: var(--text-tertiary); }
  .group-body { padding: 8px 14px 12px; display: flex; flex-direction: column; gap: 8px; }
  .tool-item { padding: 6px 0; border-bottom: 1px solid var(--border); }
  .tool-item:last-child { border-bottom: none; }
  .tool-name { font-weight: 600; font-size: 13px; font-family: monospace; color: var(--accent); }
  .tool-desc { font-size: 12px; color: var(--text-secondary); margin-top: 2px; }
</style>
