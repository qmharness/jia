<script lang="ts">
  import { t } from '../lib/i18n';

  let {
    search = $bindable(''),
    selectedCount,
    totalCount,
    filteredCount,
    onbulkdelete,
    onclearselection,
    onrefresh,
  }: {
    search: string;
    selectedCount: number;
    totalCount: number;
    filteredCount: number;
    onbulkdelete: () => void;
    onclearselection: () => void;
    onrefresh: () => void;
  } = $props();

  let hasSelection = $derived(selectedCount > 0);
</script>

<div class="header">
  <h2 class="title">
    {t('sessions.title')}
    {#if !hasSelection}
      <span class="count">{filteredCount !== totalCount ? `${filteredCount} of ` : ''}{totalCount}</span>
    {/if}
  </h2>
  <div class="actions">
    {#if hasSelection}
      <span class="selected-count">{t('sessions.selected', { n: selectedCount })}</span>
      <button class="btn btn-danger" onclick={onbulkdelete}>{t('sessions.deleteBtn')}</button>
      <button class="btn" onclick={onclearselection}>{t('sessions.cancel')}</button>
    {:else}
      <input class="search" type="text" placeholder={t('sessions.searchPlaceholder')} bind:value={search} />
      <button class="btn" onclick={onrefresh}>{t('sessions.refresh')}</button>
    {/if}
  </div>
</div>

<style>
  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  .title { font-size: 16px; font-weight: 600; }
  .count { font-weight: 400; font-size: 13px; color: var(--text-tertiary); margin-left: 6px; }
  .actions { display: flex; gap: 8px; align-items: center; }
  .search {
    font-size: 13px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-secondary);
    outline: none;
    width: 200px;
  }
  .search:focus { border-color: var(--accent); }
  .selected-count { font-size: 13px; color: var(--text-secondary); font-weight: 500; }
  .btn {
    font-size: 13px;
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    background: var(--bg-secondary);
    transition: all .15s;
  }
  .btn:hover { background: var(--bg-tertiary); color: var(--text-primary); }
  .btn-danger {
    background: var(--error);
    color: #fff;
    border-color: var(--error);
  }
  .btn-danger:hover { background: #b91c1c; }
</style>
