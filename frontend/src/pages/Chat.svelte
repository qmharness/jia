<script lang="ts">
  import { onMount } from 'svelte';
  import { store, getSessionProvider, getSessionModel, setPage } from '../lib/store.svelte';
  import { sendMessage } from '../lib/sse';
  import { uiStore } from '../lib/stores/ui.svelte';
  import type { StreamAgentParams } from '../lib/types';
  import { t } from '../lib/i18n';
  import { fetchProjects, fetchConfig } from '../lib/api';

  let text = $state('');
  let projects = $state<{cwd: string, name: string, id: string}[]>([]);
  let selectedCwd = $state('');
  let selectedProjectId = $state('');

  async function loadProjects() {
    try {
      const plist = await fetchProjects();
      projects = plist.map(p => ({ cwd: p.cwd, name: p.name || p.cwd.split('/').pop() || p.cwd, id: p.id }));
    } catch { /* noop */ }
    try {
      await fetchConfig();
    } catch { /* noop */ }
  }
  onMount(() => loadProjects());

  function onSubmit() {
    const trimmed = text.trim();
    if (!trimmed) return;
    const tempId = crypto.randomUUID();
    const now = Math.floor(Date.now() / 1000);
    // Insert temp session entry into sidebar Recents immediately
    const cwd = selectedCwd || '';
    const entry = { id: tempId, title: trimmed, cwd, messageCount: 1, updatedAt: now };
    uiStore.sessions = [entry, ...uiStore.sessions];
    uiStore.activeSessionId = tempId;
    uiStore.loadedSessionId = null;
    window.dispatchEvent(new CustomEvent('jia:new-session', { detail: entry }));

    const params: StreamAgentParams = {
      provider: getSessionProvider(),
      model: getSessionModel() || undefined,
      auxProvider: store.selectedAuxProvider || undefined,
      auxModel: store.selectedAuxModel || undefined,
      messages: [{ role: 'user', content: trimmed }],
      sessionId: null,
      _tempSessionId: tempId,
      cwd: cwd || undefined,
      projectId: selectedProjectId || undefined,
    };
    store.entries = [...store.entries, {
      id: crypto.randomUUID(), role: 'user',
      content: trimmed, createdAt: Date.now(),
    }];
    text = '';
    sendMessage(params);
    // Navigation to session page happens in sse.ts when real session ID arrives
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); onSubmit(); }
  }

  const suggestions = $derived([
    t('chat.emptySugg1'), t('chat.emptySugg2'),
    t('chat.emptySugg4'),
  ]);
</script>

<div class="landing">
  <div class="landing-center">
    <span class="landing-logo">甲</span>
    <h1 class="landing-title">{t('chat.emptyTitle')}</h1>

    {#if projects.length > 0}
      <div class="project-select-wrap">
        <svg class="project-select-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
        <select class="project-select" bind:value={selectedCwd} onchange={(e) => { const p = projects.find(pp => pp.cwd === (e.target as HTMLSelectElement).value); selectedProjectId = p?.id || ''; }}>
          <option value="">{t('projects.workInProject')}</option>
          {#each projects as p}
            <option value={p.cwd}>{p.name}</option>
          {/each}
        </select>
      </div>
    {/if}

    <div class="landing-input-wrap">
      <textarea
        class="landing-input"
        bind:value={text}
        placeholder={t('chat.inputPlaceholder')}
        rows="2"
        onkeydown={onKeydown}
      ></textarea>
      <button class="landing-send" onclick={onSubmit} disabled={!text.trim()}>
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="5" x2="12" y2="19"/><polyline points="5 12 12 5 19 12"/></svg>
      </button>
    </div>

    <div class="landing-suggestions">
      {#each suggestions as s}
        <button class="landing-suggestion" onclick={() => { text = s; onSubmit(); }}>{s}</button>
      {/each}
    </div>
  </div>
</div>

<style>
  .landing {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 40px 24px 80px;
  }

  .landing-center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 24px;
    max-width: 640px;
    width: 100%;
  }

  .landing-logo {
    font-size: 48px; font-weight: 600;
    color: var(--accent);
    line-height: 1;
  }

  .landing-title {
    font-size: 28px;
    font-weight: 400;
    color: var(--text-secondary);
    letter-spacing: -0.5px;
    margin: 0;
  }

  /* Project Select */
  .project-select-wrap {
    display: flex; align-items: center; gap: 8px;
    padding: 6px 14px; border: 1px solid var(--border);
    border-radius: 20px; background: var(--bg-primary);
    transition: border-color .15s;
  }
  .project-select-wrap:focus-within { border-color: var(--accent); }
  .project-select-icon { color: var(--text-tertiary); flex-shrink: 0; }
  .project-select {
    border: none; background: none; outline: none;
    font-size: 13px; color: var(--text-primary); font-family: var(--font-system);
    cursor: pointer; min-width: 200px;
  }

  /* Input */
  .landing-input-wrap {
    display: flex;
    align-items: flex-end;
    width: 100%;
    padding: 10px 16px;
    border: 1px solid var(--border);
    border-radius: 20px;
    background: var(--bg-primary);
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .landing-input-wrap:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-light);
  }

  .landing-input {
    flex: 1;
    border: none;
    background: transparent;
    outline: none;
    resize: none;
    font-size: 15px;
    line-height: 1.6;
    color: var(--text-primary);
    font-family: var(--font-system);
    padding: 0;
    min-height: 48px;
  }

  .landing-input::placeholder { color: var(--text-tertiary); }

  .landing-send {
    width: 32px; height: 32px;
    border-radius: 50%;
    display: flex; align-items: center; justify-content: center;
    flex-shrink: 0;
    border: none; cursor: pointer;
    background: transparent;
    color: var(--text-tertiary);
    transition: all 0.15s;
  }

  .landing-send:not(:disabled) {
    color: var(--accent);
  }

  .landing-send:not(:disabled):hover {
    background: var(--accent-light);
  }

  .landing-send:disabled {
    opacity: 0.3;
    cursor: default;
  }

  /* Suggestions */
  .landing-suggestions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: center;
  }

  .landing-suggestion {
    padding: 6px 16px;
    border: 1px solid var(--border);
    border-radius: 20px;
    font-size: 13px;
    color: var(--text-secondary);
    background: transparent;
    transition: all 0.15s;
    cursor: pointer;
  }

  .landing-suggestion:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
