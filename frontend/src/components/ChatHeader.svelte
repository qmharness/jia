<script lang="ts">
  import { fetchSessions, fetchSessionMessages, renameSession } from '../lib/api';
  import { store, showToast, clearActiveSession, getSessionProvider, getSessionModel, setSessionProvider, setSessionModel } from '../lib/store.svelte';
  import { uiStore } from '../lib/stores/ui.svelte';
  import type { ChatEntry, SessionMeta } from '../lib/types';
  import { t } from '../lib/i18n';

  let sessions = $state<SessionMeta[]>([]);

  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let pollDeadline = 0;
  let editing = $state(false);
  let editValue = $state('');

  let currentProvider = $derived(store.providers.find(p => p.name === getSessionProvider()));
  let currentSession = $derived(sessions.find(s => s.id === uiStore.activeSessionId));
  let sessionTitle = $derived(currentSession?.title || '');
  let projectName = $derived(currentSession?.projectName || '');

  function onProviderChange(e: Event) {
    const newProvider = (e.target as HTMLSelectElement).value;
    const sid = uiStore.activeSessionId;
    if (sid) {
      setSessionProvider(sid, newProvider);
      const p = store.providers.find(pp => pp.name === newProvider);
      if (p) setSessionModel(sid, p.default_model);
    }
  }

  function onModelChange(e: Event) {
    const newModel = (e.target as HTMLSelectElement).value;
    const sid = uiStore.activeSessionId;
    if (sid) setSessionModel(sid, newModel);
  }

  import { parseEntries } from '../lib/entries';

  function isSessionComplete(entries: any[]): boolean {
    if (entries.length === 0) return false;
    const last = entries[entries.length - 1];
    if (last.role === 'assistant') return true;
    for (const e of entries) {
      if (e.role === 'tool_call' && e.status === 'error') return true;
    }
    return false;
  }

  function stopPolling() {
    if (pollTimer) { clearTimeout(pollTimer); pollTimer = null; }
    pollDeadline = 0;
  }

  async function pollOnce(id: string) {
    if (Date.now() > pollDeadline) { stopPolling(); return; }
    try {
      const data = await fetchSessionMessages(id);
      // Staleness guard: if user navigated away during poll interval, discard
      if (id !== uiStore.activeSessionId) { stopPolling(); return; }
      const entries = parseEntries(data, { createdAt: true });
      store.entries = entries;
      uiStore.loadedSessionId = id;
      loadSessions();
      if (isSessionComplete(entries)) { stopPolling(); return; }
      pollDeadline = Date.now() + 60_000;
      pollTimer = setTimeout(() => pollOnce(id), 1500);
    } catch (err) {
      console.error(`[ChatHeader] Poll failed for session ${id}:`, err);
      stopPolling();
    }
  }

  function startPolling(id: string) {
    stopPolling();
    pollDeadline = Date.now() + 60_000;
    pollTimer = setTimeout(() => pollOnce(id), 1500);
  }

  async function loadSessions() {
    try { sessions = await fetchSessions(); } catch (err) { console.error('[ChatHeader] Failed to load sessions:', err); }
  }

  $effect(() => {
    const id = uiStore.activeSessionId;
    // NOTE: Do NOT read store.entries in this $effect — it would track entries
    // and re-fire when loadSessionMessages sets store.entries = [], causing an infinite loop.
    loadSessions();
    if (id && uiStore.loadedSessionId !== id) {
      store.loadingMessages = true;
      loadSessionMessages(id);
    }
  });

  $effect(() => {
    return () => stopPolling();
  });

  async function loadSessionMessages(id: string) {
    const maxRetries = 4;
    store.entries = [];
    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      // Staleness guard: if user navigated away, stop
      if (id !== uiStore.activeSessionId) return;

      try {
        const data = await fetchSessionMessages(id);
        // Staleness guard: if user navigated away during fetch, discard
        if (id !== uiStore.activeSessionId) return;
        const entries = parseEntries(data, { createdAt: true });
        store.entries = entries;
        uiStore.loadedSessionId = id;
        store.loadingMessages = false;
        break;
      } catch (err: any) {
        console.error(
          `[ChatHeader] Failed to load session messages for ${id} ` +
          `(attempt ${attempt + 1}/${maxRetries + 1}):`,
          err
        );
        if (attempt < maxRetries) {
          await new Promise(r => setTimeout(r, 200 * Math.pow(2, attempt)));
        } else {
          showToast(t('sessions.loadSessionMessagesFailed'), 'error');
          store.loadingMessages = false;
        }
      }
    }
    // Staleness guard: if user navigated away, don't reconnect or poll
    if (id !== uiStore.activeSessionId) return;

    // Reconnect to active SSE stream if session is still running
    for (const [sid, sessId] of Object.entries(store.streamSessions)) {
      if (sessId === id && store.cancels[sid]) {
        store.entries = [...store.entries, {
          id: crypto.randomUUID(),
          role: 'assistant' as const,
          get content() { return store.streamStates[sid]?.text ?? ''; },
          _streamId: sid,
          createdAt: Date.now(),
        } as any];
        break;
      }
    }
    if (Object.keys(store.cancels).length === 0) {
      startPolling(id);
    }
  }

  function startEdit() {
    editing = true;
    editValue = sessionTitle;
  }

  async function saveEdit() {
    editing = false;
    const title = editValue.trim();
    if (!title || title === sessionTitle) return;
    const sid = uiStore.activeSessionId;
    if (!sid) return;
    try {
      await renameSession(sid, title);
      await loadSessions();
      window.dispatchEvent(new CustomEvent('jia:refresh-sessions'));
    } catch (err) { console.error('[ChatHeader] Failed to rename session:', err); showToast(t('sessions.renameFailed'), 'error'); }
  }

  function cancelEdit() { editing = false; }

  function onEditKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') { e.preventDefault(); saveEdit(); }
    if (e.key === 'Escape') cancelEdit();
  }

  function downloadMarkdown() {
    let md = '';
    for (const e of store.entries) {
      if (e.role === 'tool_call') {
        md += `### 🔧 ${e.tool}\n\n`;
        md += '```json\n' + JSON.stringify(e.input, null, 2) + '\n```\n\n';
        if (e.output) md += '```\n' + e.output + '\n```\n\n';
        if (e.error) md += '> Error: ' + e.error + '\n\n';
      } else {
        const role = e.role === 'user' ? `**${t('chat.roleYou')}**` : `**${t('chat.roleJia')}**`;
        md += `${role}\n\n${e.content}\n\n`;
      }
    }
    const blob = new Blob([md], { type: 'text/markdown' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `chat-${uiStore.activeSessionId?.slice(0, 8) || 'export'}.md`;
    a.click();
    URL.revokeObjectURL(url);
  }
</script>

<div class="header">
  <div class="header-row">
    <div class="header-breadcrumb">
      <button class="project-name" onclick={() => { const pid = currentSession?.projectId; if (pid) { window.location.hash = 'project/' + pid; } }}>
        {projectName || t('chat.title')}
      </button>
      <span class="sep">/</span>
      {#if editing}
        <input class="edit-input" type="text" bind:value={editValue} onblur={saveEdit} onkeydown={onEditKeydown} autofocus />
      {:else if sessionTitle}
        <button class="session-title-btn" onclick={startEdit} title={t('sessions.rename') || 'Rename'}>
          {sessionTitle}
          <svg class="edit-hint" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
        </button>
      {:else}
        <span class="session-title-empty">&mdash;</span>
      {/if}
    </div>
    <div class="header-actions">
      <input class="search-input" type="text" placeholder={t('chat.searchPlaceholder')} bind:value={store.searchQuery} />
      <button class="btn-download" onclick={downloadMarkdown} title={t('chat.downloadMarkdown')}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
      </button>
    </div>
  </div>
</div>

<style>
  .header {
    display: flex;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-primary);
  }

  .header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    gap: var(--space-3);
  }

  .header-breadcrumb {
    display: flex; align-items: center; gap: 6px;
    min-width: 0; flex: 1;
  }

  .project-name {
    font-size: 13px; font-weight: 500;
    color: var(--accent);
    padding: 0; white-space: nowrap;
  }
  .project-name:hover { text-decoration: underline; }

  .sep {
    font-size: 13px; color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .header-actions {
    display: flex; align-items: center; gap: var(--space-2);
    flex-shrink: 0;
  }

  .search-input {
    font-size: 12px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-secondary);
    color: var(--text-primary);
    outline: none;
    width: 180px;
  }
  .search-input:focus { border-color: var(--accent); }
  .search-input::placeholder { color: var(--text-tertiary); }

  .btn-download {
    width: 28px; height: 28px; border-radius: var(--radius-sm);
    display: flex; align-items: center; justify-content: center;
    color: var(--text-tertiary);
  }
  .btn-download:hover { background: var(--bg-tertiary); color: var(--text-primary); }

  .session-title-btn {
    display: flex; align-items: center; gap: var(--space-2);
    font-size: 14px; font-weight: 500;
    color: var(--text-primary);
    padding: 0; border-radius: var(--radius-sm);
  }
  .session-title-btn:hover { color: var(--accent); }
  .session-title-btn:hover .edit-hint { opacity: 1; }

  .edit-hint {
    opacity: 0;
    color: var(--text-tertiary);
    transition: opacity var(--duration-fast);
  }

  .session-title-empty {
    font-size: 14px;
    color: var(--text-tertiary);
  }

  .edit-input {
    font-size: 14px; font-weight: 500;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    color: var(--text-primary);
    outline: none;
    width: 300px;
  }
</style>
