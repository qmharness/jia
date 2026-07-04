<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchSessions, fetchConfig, fetchProject, updateProject } from '../lib/api';
  import type { SessionMeta, StreamAgentParams } from '../lib/types';
  import { store, setPage, showToast, getSessionProvider, getSessionModel } from '../lib/store.svelte';
  import { uiStore } from '../lib/stores/ui.svelte';
  import { sendMessage } from '../lib/sse';
  import { t } from '../lib/i18n';
  import { relativeTime } from '../lib/time';

  let sessions = $state<SessionMeta[]>([]);
  let text = $state('');
  let editingProjectName = $state(false);
  let projectNameEdit = $state('');
  let projectDisplayName = $state('');
  let projectId = $state('');
  let projectCwd = $state('');
  let notFound = $state(false);
  let projectPinned = $state(false);
  let projectArchived = $state(false);

  let projectName = $derived(projectDisplayName || (projectCwd ? projectCwd.split('/').pop() || projectCwd : ''));

  let projectSessions = $derived(
    sessions
      .filter(s => s.cwd === projectCwd)
      .sort((a, b) => b.updatedAt - a.updatedAt)
  );

  async function load() {
    try {
      // Determine project: from URL hash ID, or from stored cwd, or first available
      let proj = null;
      const pid = uiStore.projectId;
      if (pid) {
        try { proj = await fetchProject(pid); } catch { /* noop */ }
      }
      if (proj && proj.cwd) {
        projectCwd = proj.cwd;
        projectId = proj.id;
        if (proj.name) projectDisplayName = proj.name;
      } else if (pid) {
        notFound = true;
        return;
      } else {
        // No project ID in URL — redirect to projects page
        setPage('projects');
        return;
      }
      sessions = await fetchSessions();
    } catch { showToast('Failed to load', 'error'); }
  }
  onMount(() => load());

  function onSubmit() {
    const trimmed = text.trim();
    if (!trimmed) return;
    const tempId = crypto.randomUUID();
    const now = Math.floor(Date.now() / 1000);
    const cwd = projectCwd || '';
    const entry = { id: tempId, title: trimmed, cwd, messageCount: 1, updatedAt: now };
    uiStore.sessions = [entry, ...uiStore.sessions];
    uiStore.activeSessionId = tempId;
    uiStore.loadedSessionId = null;
    window.dispatchEvent(new CustomEvent('jia:new-session', { detail: entry }));
    const params: StreamAgentParams = {
      provider: getSessionProvider(),
      model: getSessionModel() || undefined,
      messages: [{ role: 'user', content: trimmed }],
      sessionId: null,
      _tempSessionId: tempId,
      cwd: cwd || undefined,
      projectId: projectId || undefined,
    };
    store.entries = [];
    text = '';
    sendMessage(params);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); onSubmit(); }
  }

  function goSession(id: string) { store.activeSessionId = id; window.location.hash = 'session/' + id; }

  async function saveProjectName() {
    const name = projectNameEdit.trim() || projectName;
    projectDisplayName = name;
    editingProjectName = false;
    if (projectId && name) {
      try { await updateProject(projectId, { name }); } catch (e) {
        console.error('[Project] Failed to save name:', e);
        showToast('重命名失败', 'error');
      }
    }
  }
  function cancelProjectName() { editingProjectName = false; }
  function handleProjectNameKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') saveProjectName();
    else if (e.key === 'Escape') cancelProjectName();
  }
</script>

<div class="page">
  {#if notFound}
    <div class="not-found">
      <h2>Project not found</h2>
      <p>The project you are looking for does not exist or has been removed.</p>
    </div>
  {:else}
  <div class="content">
    <div class="content-inner">
      <div class="project-head">
        <div class="project-head-row">
          <div class="project-title-area">
            {#if editingProjectName}
              <div class="project-title-edit">
                <input class="project-title-input" type="text" bind:value={projectNameEdit} onkeydown={handleProjectNameKeydown} onblur={saveProjectName} autofocus />
              </div>
            {:else}
              <h1 class="project-title" onclick={() => { projectNameEdit = projectDisplayName || (projectCwd ? projectCwd.split('/').pop() || '' : ''); editingProjectName = true; }} title="Click to rename">{projectName}</h1>
            {/if}
          </div>
          <div class="project-actions">
            <button class="action-chip" class:active={projectPinned} onclick={() => projectPinned = !projectPinned} title={projectPinned ? 'Pinned' : 'Pin'}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="17" x2="12" y2="22"/><path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76L15 6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z"/></svg>
            </button>
            <button class="action-chip" class:active={projectArchived} onclick={() => projectArchived = !projectArchived} title={projectArchived ? 'Archived' : 'Archive'}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="21 8 21 21 3 21 3 8"/><rect x="1" y="3" width="22" height="5"/><line x1="10" y1="12" x2="14" y2="12"/></svg>
            </button>
          </div>
        </div>
        <p class="project-path">{projectCwd}</p>
      </div>

    <div class="chat-box">
      <textarea
        class="chat-input"
        bind:value={text}
        placeholder={t('chat.inputPlaceholder')}
        rows="2"
        onkeydown={onKeydown}
      ></textarea>
      <button class="chat-send" onclick={onSubmit} disabled={!text.trim()}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><polyline points="5 12 12 5 19 12"/></svg>
      </button>
    </div>

    <div class="recents-section">
      <h3 class="recents-title">{t('nav.recents')}</h3>
      {#if projectSessions.length === 0}
        <p class="msg">No sessions yet. Start a conversation above.</p>
      {:else}
        <div class="recents-list">
          {#each projectSessions as s}
            <div class="session-row" onclick={() => goSession(s.id)} role="button" tabindex="0" onkeydown={(e) => { if (e.key === 'Enter') goSession(s.id); }}>
              <span class="s-status" class:status-active={s.status === 'active'} class:status-error={s.status === 'error'}></span>
              <div class="s-main">
                <span class="s-title">{s.title || t('sessions.untitled')}</span>
                <span class="s-meta">{s.messageCount} msgs · {relativeTime(s.updatedAt)}</span>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    </div>
  </div>
  <div class="panel">
    <p class="sidebar-hint">Details and actions will appear here.</p>
  </div>
  {/if}
</div>

<style>
  .page { display: flex; flex-direction: row; flex: 1; overflow: hidden; }
  .not-found { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: var(--text-secondary); }
  .not-found h2 { font-size: 18px; font-weight: 600; color: var(--text-primary); }
  .not-found p { font-size: 13px; }
  .content { flex: 1; overflow-y: auto; padding: 32px 48px; min-width: 0; }
  .content-inner { max-width: 640px; width: 100%; margin: 0 auto; display: flex; flex-direction: column; gap: 28px; }
  .panel {
    width: 260px; flex-shrink: 0; border-left: 1px solid var(--border);
    background: var(--bg-secondary); padding: 24px 20px;
    display: flex; flex-direction: column; gap: 8px;
    overflow-y: auto;
  }
  /* Project head */
  .project-head { display: flex; flex-direction: column; gap: 2px; }
  .project-head-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .project-title-area { flex: 1; min-width: 0; }
  .project-title { font-size: 28px; font-weight: 700; color: var(--text-primary); letter-spacing: -0.5px; cursor: pointer; border-radius: var(--radius-sm); padding: 2px 6px; margin: -2px -6px; transition: background .15s; }
  .project-title:hover { background: var(--bg-secondary); }
  .project-title-edit { margin: -2px -6px; }
  .project-title-input { font-size: 28px; font-weight: 700; color: var(--text-primary); letter-spacing: -0.5px; border: 1px solid var(--accent); border-radius: var(--radius-sm); background: var(--bg-primary); padding: 2px 6px; outline: none; font-family: var(--font-system); width: 100%; }
  .project-path { font-size: 13px; color: var(--text-tertiary); font-family: var(--font-mono); }

  .project-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .action-chip {
    display: flex; align-items: center; justify-content: center;
    width: 32px; height: 32px; border-radius: var(--radius-sm);
    border: none; background: transparent; color: var(--text-tertiary);
    cursor: pointer; transition: all .15s;
  }
  .action-chip:hover { background: var(--bg-tertiary); color: var(--text-primary); }
  .action-chip.active { color: var(--accent); background: var(--accent-light); }

  /* Chat box */
  .chat-box {
    display: flex; align-items: flex-end;
    padding: 10px 16px; border: 1px solid var(--border);
    border-radius: var(--radius-lg); background: var(--bg-primary);
    transition: border-color .15s, box-shadow .15s;
  }
  .chat-box:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-light); }
  .chat-input {
    flex: 1; border: none; background: none; outline: none; resize: none;
    font-size: 14px; line-height: 1.6; color: var(--text-primary); font-family: var(--font-system);
    padding: 0; min-height: 44px;
  }
  .chat-input::placeholder { color: var(--text-tertiary); }
  .chat-send {
    width: 32px; height: 32px; border-radius: 50%;
    display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    border: none; cursor: pointer; background: transparent; color: var(--text-tertiary);
    transition: all .15s;
  }
  .chat-send:not(:disabled) { color: var(--accent); }
  .chat-send:not(:disabled):hover { background: var(--accent-light); }
  .chat-send:disabled { opacity: .3; cursor: default; }

  /* Recents */
  .recents-section { display: flex; flex-direction: column; gap: 8px; }
  .recents-title { font-size: 13px; font-weight: 600; color: var(--text-secondary); }
  .msg { text-align: center; color: var(--text-secondary); padding: 24px; font-size: 13px; }

  .recents-list { display: flex; flex-direction: column; }
  .session-row {
    display: flex; align-items: center; gap: 10px; padding: 10px 12px;
    border-radius: var(--radius-md); transition: background .15s;
    cursor: pointer; background: none; border: none; text-align: left;
    font-family: var(--font-system); width: 100%;
  }
  .session-row:hover { background: var(--bg-secondary); }
  .s-status { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: var(--text-tertiary); }
  .s-status.status-active { background: var(--success); }
  .s-status.status-error { background: var(--error); }
  .s-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .s-title { font-size: 14px; color: var(--text-primary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .s-meta { font-size: 12px; color: var(--text-tertiary); }
</style>
