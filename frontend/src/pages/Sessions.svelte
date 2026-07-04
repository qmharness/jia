<script lang="ts">
  import { fetchSessions, fetchSessionMessages, deleteSession, bulkDeleteSessions, renameSession } from '../lib/api';
  import { store, setPage, showToast, clearActiveSession } from '../lib/store.svelte';
  import { uiStore } from '../lib/stores/ui.svelte';
  import type { SessionMeta } from '../lib/types';
  import { groupKey } from '../lib/time';
  import type { TimeGroup } from '../lib/time';
  import { parseEntries } from '../lib/entries';
  import { t, timeGroupLabel } from '../lib/i18n';
  import SessionsHeader from '../components/SessionsHeader.svelte';
  import SessionCard from '../components/SessionCard.svelte';
  import { archiveSession as archiveApi, unarchiveSession as unarchiveApi } from '../lib/api';

  let tab = $state<'active' | 'archived' | 'all'>('active');
  let list = $derived(uiStore.sessions);
  let loading = $state(true);
  let search = $state('');
  let selectedIds = $state<Set<string>>(new Set());
  let confirmDelete: { single?: string; bulk?: true } | null = $state(null);
  let sortKey = $state<'title' | 'messageCount' | 'updatedAt'>('updatedAt');
  let sortDir = $state<'asc' | 'desc'>('desc');

  const filtered = $derived(
    search ? list.filter(s => s.title.toLowerCase().includes(search.toLowerCase())) : list
  );

  const sorted = $derived.by(() => {
    const arr = [...filtered];
    arr.sort((a, b) => {
      let cmp: number;
      if (sortKey === 'title') {
        cmp = a.title.localeCompare(b.title);
      } else if (sortKey === 'messageCount') {
        cmp = a.messageCount - b.messageCount;
      } else {
        cmp = a.updatedAt - b.updatedAt;
      }
      return sortDir === 'asc' ? cmp : -cmp;
    });
    return arr;
  });

  const grouped = $derived.by(() => {
    const order: TimeGroup[] = ['Today', 'Yesterday', 'This week', 'This month', 'Older'];
    const map = new Map<TimeGroup, SessionMeta[]>();
    for (const s of sorted) {
      const g = groupKey(s.updatedAt);
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(s);
    }
    return order.filter(g => map.has(g)).map(g => [g, map.get(g)!] as const);
  });

  const allFilteredSelected = $derived(
    sorted.length > 0 && sorted.every(s => selectedIds.has(s.id))
  );

  $effect(() => {
    refresh();
  });

  async function refresh() {
    loading = true;
    try {
      uiStore.sessions = await fetchSessions(tab === 'all' ? 'all' : tab === 'archived' ? 'archived' : 'active');
    } catch { showToast(t('sessions.loadFailed'), 'error'); }
    loading = false;
    window.dispatchEvent(new CustomEvent('jia:refresh-sessions'));
  }

  $effect(() => { void tab; refresh(); });

  function toggleSelect(id: string) {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id); else next.add(id);
    selectedIds = next;
  }

  function setSort(key: 'title' | 'messageCount' | 'updatedAt') {
    if (sortKey === key) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortKey = key;
      sortDir = key === 'updatedAt' ? 'desc' : 'asc';
    }
  }

  function sortIndicator(key: typeof sortKey): string {
    if (sortKey !== key) return '';
    return sortDir === 'asc' ? ' ▲' : ' ▼';
  }

  function toggleSelectAll() {
    if (allFilteredSelected) {
      const next = new Set(selectedIds);
      for (const s of sorted) next.delete(s.id);
      selectedIds = next;
    } else {
      selectedIds = new Set([...selectedIds, ...sorted.map(s => s.id)]);
    }
  }

  function clearSelection() {
    selectedIds = new Set();
  }

  function onBulkDelete() {
    if (selectedIds.size === 0) return;
    confirmDelete = { bulk: true };
  }

  async function executeBulkDelete() {
    const ids = [...selectedIds];
    try {
      await bulkDeleteSessions(ids);
      uiStore.sessions = uiStore.sessions.filter(s => !selectedIds.has(s.id));
      window.dispatchEvent(new CustomEvent('jia:refresh-sessions'));
      if (selectedIds.has(store.activeSessionId!)) clearActiveSession();
      selectedIds = new Set();
      showToast(t('sessions.bulkDeletedToast', { n: ids.length }), 'success');
    } catch {
      showToast(t('sessions.bulkDeleteFailed'), 'error');
    }
    confirmDelete = null;
  }

  function onSingleDelete(id: string) {
    confirmDelete = { single: id };
  }

  async function executeSingleDelete() {
    const id = confirmDelete?.single;
    if (!id) return;
    try {
      await deleteSession(id);
      uiStore.sessions = uiStore.sessions.filter(s => s.id !== id);
      if (id === store.activeSessionId) clearActiveSession();
      selectedIds.delete(id);
      selectedIds = new Set(selectedIds);
      showToast(t('sessions.deletedToast'), 'success');
    } catch {
      showToast(t('sessions.deleteFailed'), 'error');
    }
    confirmDelete = null;
  }

  async function onRename(id: string, title: string) {
    try {
      await renameSession(id, title);
      uiStore.sessions = uiStore.sessions.map(s => s.id === id ? { ...s, title } : s);
      window.dispatchEvent(new CustomEvent('jia:refresh-sessions'));
    } catch {
      showToast(t('sessions.renameFailed'), 'error');
    }
  }

  async function onSelect(session: SessionMeta) {
    if (store.loadedSessionId === session.id) {
      window.location.hash = 'session/' + session.id;
      return;
    }
    try {
      const data = await fetchSessionMessages(session.id);
      const newEntries = parseEntries(data);
      store.entries = newEntries;
      store.activeSessionId = session.id;
      store.loadedSessionId = session.id;
      window.location.hash = 'session/' + session.id;
    } catch {
      showToast(t('sessions.loadSessionFailed'), 'error');
    }
  }
</script>

<div class="page">
  <div class="sessions-tabs">
    <button class="sessions-tab" class:active={tab === 'active'} onclick={() => tab = 'active'}>{t('sessions.tabActive')}</button>
    <button class="sessions-tab" class:active={tab === 'archived'} onclick={() => tab = 'archived'}>{t('sessions.tabArchived')}</button>
    <button class="sessions-tab" class:active={tab === 'all'} onclick={() => tab = 'all'}>{t('sessions.tabAll')}</button>
  </div>
  <SessionsHeader
    bind:search
    selectedCount={selectedIds.size}
    totalCount={list.length}
    filteredCount={filtered.length}
    onbulkdelete={onBulkDelete}
    onclearselection={clearSelection}
    onrefresh={refresh}
  />
  <div class="list">
    {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else if sorted.length === 0}
      <p class="msg">{search ? t('sessions.noMatch') : t('sessions.noSessions')}</p>
    {:else}
      <table class="table">
        <thead>
          <tr>
            <th class="col-check" onclick={toggleSelectAll}>
              <span class="checkbox" class:checked={allFilteredSelected}>
                {#if allFilteredSelected}✓{/if}
              </span>
            </th>
            <th class="col-title sortable" onclick={() => setSort('title')}>
              {t('sessions.colTitle')}{sortIndicator('title')}
            </th>
            <th class="col-id">ID</th>
            <th class="col-project">{t('sessions.colProject')}</th>
            <th class="col-msgs sortable" onclick={() => setSort('messageCount')}>
              {t('sessions.colMsgs')}{sortIndicator('messageCount')}
            </th>
            <th class="col-time sortable" onclick={() => setSort('updatedAt')}>
              {t('sessions.colUpdated')}{sortIndicator('updatedAt')}
            </th>
            <th class="col-actions">{t('sessions.colActions')}</th>
          </tr>
        </thead>
        <tbody>
          {#each grouped as [label, sessions]}
            <tr class="group-row">
              <td colspan="7" class="group-label">{timeGroupLabel(label)}</td>
            </tr>
            {#each sessions as s}
              <SessionCard
                session={s}
                active={store.activeSessionId === s.id}
                selected={selectedIds.has(s.id)}
                onselect={() => onSelect(s)}
                ontoggle={() => toggleSelect(s.id)}
                onrename={(title: string) => onRename(s.id, title)}
                ondelete={() => onSingleDelete(s.id)}
              />
            {/each}
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>

{#if confirmDelete}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => confirmDelete = null}>
    <div class="dialog" onclick={(e: MouseEvent) => e.stopPropagation()} onkeydown={() => {}}>
      <h3 class="dialog-title">{t('sessions.deleteTitle')}</h3>
      <p class="dialog-body">
        {#if confirmDelete.bulk}
          {t('sessions.deleteBulkMsg', { n: selectedIds.size })}
        {:else}
          {t('sessions.deleteSingleMsg')}
        {/if}
      </p>
      <div class="dialog-actions">
        <button class="btn-cancel" onclick={() => confirmDelete = null}>{t('sessions.cancel')}</button>
        <button class="btn-confirm" onclick={confirmDelete.bulk ? executeBulkDelete : executeSingleDelete}>{t('sessions.deleteBtn')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    height: 100%;
    .sessions-tabs {
      display: flex; gap: 4px;
      padding: 8px 20px;
      border-bottom: 1px solid var(--border);
    }
    .sessions-tab {
      padding: 4px 14px; border-radius: 6px;
      font-size: 13px; font-weight: 500;
      color: var(--text-secondary);
      transition: all 0.15s;
    }
    .sessions-tab:hover { color: var(--text-primary); background: rgba(0,0,0,.04); }
    .sessions-tab.active { color: var(--text-primary); background: rgba(0,0,0,.06); }

  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 20px 16px;
  }
  .msg {
    text-align: center;
    color: var(--text-secondary);
    padding: 40px;
    font-size: 14px;
  }

  .table {
    width: 100%;
    border-collapse: collapse;
  }

  thead th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: .5px;
    padding: 10px 12px 6px;
    border-bottom: 2px solid var(--border);
    position: sticky;
    top: 0;
    background: var(--bg-primary);
    z-index: 1;
  }

  .col-check {
    width: 40px;
    text-align: center;
    cursor: pointer;
  }

  .col-check .checkbox {
    width: 18px; height: 18px;
    border-radius: 4px;
    border: 2px solid var(--border);
    display: inline-flex; align-items: center; justify-content: center;
    font-size: 11px; color: #fff;
    transition: all .15s;
  }
  .col-check .checkbox.checked {
    background: var(--accent);
    border-color: var(--accent);
  }

  .col-title { width: 10%; }
  .col-id { width: 280px; }
  .col-project { width: 160px; }
  .col-msgs { width: 90px; }
  .col-time { width: 110px; }
  .col-actions { width: 50px; text-align: center; }

  .sortable {
    cursor: pointer;
    user-select: none;
  }
  .sortable:hover {
    color: var(--text-primary);
  }

  .group-row { border-bottom: none; }
  .group-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: .5px;
    padding: 14px 12px 4px;
    border-bottom: none;
  }

  .overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,.3);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: var(--bg-primary);
    border-radius: var(--radius-lg);
    box-shadow: 0 20px 60px rgba(0,0,0,.15);
    padding: 24px;
    width: 380px;
    max-width: 90vw;
  }
  .dialog-title { font-size: 16px; font-weight: 700; margin-bottom: 12px; }
  .dialog-body { font-size: 14px; color: var(--text-secondary); margin-bottom: 20px; line-height: 1.5; }
  .dialog-actions { display: flex; gap: 10px; justify-content: flex-end; }
  .btn-cancel, .btn-confirm {
    padding: 8px 20px; border-radius: var(--radius-sm);
    font-size: 14px; font-weight: 600; transition: all .15s;
  }
  .btn-cancel { border: 1px solid var(--border); color: var(--text-secondary); background: var(--bg-secondary); }
  .btn-cancel:hover { background: var(--bg-tertiary); }
  .btn-confirm { background: var(--error); color: #fff; }
  .btn-confirm:hover { background: #b91c1c; }
</style>
