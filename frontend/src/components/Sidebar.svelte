<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchSessions, fetchConfig, archiveSession as archiveSessionApi } from '../lib/api';
  import type { SessionMeta } from '../lib/types';
  import { setPage } from '../lib/store.svelte';
  import { chatStore } from '../lib/stores/chat.svelte';
  import { settingsStore } from '../lib/stores/settings.svelte';
  import { uiStore } from '../lib/stores/ui.svelte';
  import { t } from '../lib/i18n';
  import { relativeTime } from '../lib/time';

  let pinnedOpen = $state(true);
  let recentsOpen = $state(true);
  let hoveredId = $state<string | null>(null);
  let pinnedHover = $state(false);
  let recentsHover = $state(false);
  let localSessions = $state<SessionMeta[]>([]);

  let pinnedSessions = $derived(localSessions.filter(s => s.pinned));
  let unpinnedSessions = $derived(localSessions.filter(s => !s.pinned));

  async function archive(id: string) {
    try {
      await archiveSessionApi(id);
      localSessions = localSessions.filter(s => s.id !== id);
      uiStore.sessions = uiStore.sessions.filter(s => s.id !== id);
      if (uiStore.activeSessionId === id) {
        uiStore.activeSessionId = null;
        uiStore.loadedSessionId = null;
        chatStore.entries = [];
      }
      setPage('chat');
      requestAnimationFrame(refreshHover);
    } catch { /* noop */ }
  }

  function refreshHover() {
    const el = document.elementFromPoint(lastMouseX, lastMouseY);
    if (!el) { hoveredId = null; return; }
    const row = el.closest('[data-sid]') as HTMLElement | null;
    hoveredId = row?.dataset.sid ?? null;
  }
  let lastMouseX = $state(0);
  let lastMouseY = $state(0);

  const PINNED_KEY = 'jia_pinned_sessions';
  function loadPinned(): Set<string> {
    try { return new Set(JSON.parse(localStorage.getItem(PINNED_KEY) || '[]')); } catch { return new Set(); }
  }
  function savePinned(ids: Set<string>) {
    try { localStorage.setItem(PINNED_KEY, JSON.stringify([...ids])); } catch { /* noop */ }
  }

  function togglePin(id: string) {
    localSessions = localSessions.map(s => {
      if (s.id !== id) return s;
      const pinned = !s.pinned;
      const ids = loadPinned();
      pinned ? ids.add(id) : ids.delete(id);
      savePinned(ids);
      return { ...s, pinned };
    });
    requestAnimationFrame(refreshHover);
  }

  async function load() {
    try { localSessions = await fetchSessions(); } catch (e) {
      // 守护进程不可用时,App.svelte 的启动加载已向用户提示,这里仅记录避免刷新时重复轰炸。
      console.error('[Sidebar] Failed to load sessions:', e);
    }
    // Apply persisted pinned state
    const pinnedIds = loadPinned();
    if (pinnedIds.size > 0) {
      localSessions = localSessions.map(s =>
        pinnedIds.has(s.id) ? { ...s, pinned: true } : s
      );
    }
  }
  async function loadConfig() {
    try {
      await fetchConfig();
    } catch { /* noop */ }
  }
  onMount(() => {
    load();
    loadConfig();
    window.addEventListener('jia:new-session', ((e: CustomEvent) => {
      localSessions = [e.detail, ...localSessions];
    }) as EventListener);
    window.addEventListener('jia:refresh-sessions', () => { load(); });
    window.addEventListener('jia:session-id-updated', ((e: CustomEvent) => {
      localSessions = localSessions.map(s =>
        s.id === e.detail.tempId ? { ...s, id: e.detail.realId } : s
      );
    }) as EventListener);
  });

  function selectSession(id: string) {
    uiStore.projectId = '';
    uiStore.activeSessionId = id;
    uiStore.loadedSessionId = null;  // force reload even if previously loaded
    chatStore.entries = [];
    window.location.hash = 'session/' + id;
  }
  function newChat() {
    uiStore.activeSessionId = null;
    uiStore.loadedSessionId = null;
    chatStore.entries = [];
    setPage('chat');
  }
  function navigateTo(page: string) {
    uiStore.projectId = '';
    uiStore.activeSessionId = null;
    setPage(page as any);
  }

  let activeSessionId = $derived(uiStore.activeSessionId);
  let currentPage = $derived(uiStore.currentPage);
  let hasContentSelection = $derived(!!activeSessionId || !!uiStore.projectId);
  let _locale = $derived(settingsStore.locale);

  function tl(key: string, params?: Record<string, string | number>): string {
    void _locale;
    return t(key, params);
  }
</script>

<aside class="sidebar" onmousemove={(e: MouseEvent) => { lastMouseX = e.clientX; lastMouseY = e.clientY; }}>
  <nav class="nav">
    <button class="nav-quick" class:on={currentPage === 'chat' && !hasContentSelection} onclick={newChat}>💬 {tl('nav.chat')}</button>
    <button class="nav-quick" class:on={currentPage === 'projects'} onclick={() => navigateTo('projects')}>📋 {tl('nav.projects')}</button>
    <button class="nav-quick" class:on={currentPage === 'cron'} onclick={() => navigateTo('cron')}>⏰ {tl('nav.cron')}</button>
  </nav>
  <div class="session-panel">
    <!-- Pinned -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="section-header" onmouseenter={() => pinnedHover = true} onmouseleave={() => pinnedHover = false}>
      <button class="section-title" onclick={() => pinnedOpen = !pinnedOpen}>
        {tl('nav.pinned')}
        <svg class="chevron" class:open={pinnedOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
      </button>
    </div>

    {#if pinnedOpen}
      <div class="recents-list pinned-list">
        {#each pinnedSessions as s}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="recent-row" data-sid={s.id}
            onmouseenter={() => hoveredId = s.id}
            onmouseleave={() => hoveredId = null}
          >
                          <div role="button" tabindex="0"
              class="recent-item" onkeydown={(e: KeyboardEvent) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectSession(s.id); } }}
              class:active={activeSessionId === s.id}
              onclick={() => selectSession(s.id)}
            >
              <span
                class="status-dot"
                class:active={uiStore.streamingSessionIds[s.id]}
                class:error={s.status === 'error'}
                class:idle={!uiStore.streamingSessionIds[s.id] && s.status !== 'error'}
                title={uiStore.streamingSessionIds[s.id] ? tl('sessions.statusActive') : s.status === 'error' ? tl('sessions.statusError') : ''}
              ></span>
              <span class="recent-title">{s.title || tl('sessions.untitled')}</span>
              {#if hoveredId === s.id}
                <span class="recent-actions">
                  <div class="act-btn" role="button" tabindex="0"
                    onclick={(e: MouseEvent) => { e.stopPropagation(); togglePin(s.id); }}
                    onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); togglePin(s.id); } }}
                    title="Unpin">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M16 12V4h1V2H7v2h1v8l-2 2v2h5.2v6h1.6v-6H18v-2l-2-2z"/></svg>
                  </div>
                  <div class="act-btn" role="button" tabindex="0"
                    onclick={(e: MouseEvent) => { e.stopPropagation(); archive(s.id); }}
                    onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); archive(s.id); } }}
                    title="Archive">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="21 8 21 21 3 21 3 8"/><rect x="1" y="3" width="22" height="5"/><line x1="10" y1="12" x2="14" y2="12"/></svg>
                  </div>
                </span>
              {:else}
                <span class="recent-time">{relativeTime(s.updatedAt)}</span>
              {/if}
            </div>
          </div>
        {/each}
        {#if pinnedSessions.length === 0}
          <p class="recent-empty">{tl('sessions.noPinned')}</p>
        {/if}
      </div>
    {/if}

    <!-- Recents -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="section-header" onmouseenter={() => recentsHover = true} onmouseleave={() => recentsHover = false}>
      <button class="section-title" onclick={() => recentsOpen = !recentsOpen}>
        {tl('nav.recents')}
        <svg class="chevron" class:open={recentsOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><polyline points="9 18 15 12 9 6"/></svg>
      </button>
      {#if recentsHover}
        <button class="view-all" onclick={() => navigateTo('sessions')}>{tl('chat.viewAll')} &rsaquo;</button>
      {/if}
    </div>

    {#if recentsOpen}
      <div class="recents-list recents-scroll">
        {#each unpinnedSessions as s}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="recent-row" data-sid={s.id}
            onmouseenter={() => hoveredId = s.id}
            onmouseleave={() => hoveredId = null}
          >
                          <div role="button" tabindex="0"
              class="recent-item" onkeydown={(e: KeyboardEvent) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectSession(s.id); } }}
              class:active={activeSessionId === s.id}
              onclick={() => selectSession(s.id)}
            >
              <span
                class="status-dot"
                class:active={uiStore.streamingSessionIds[s.id]}
                class:error={s.status === 'error'}
                class:idle={!uiStore.streamingSessionIds[s.id] && s.status !== 'error'}
                title={uiStore.streamingSessionIds[s.id] ? tl('sessions.statusActive') : s.status === 'error' ? tl('sessions.statusError') : ''}
              ></span>
              <span class="recent-title">{s.title || tl('sessions.untitled')}</span>
              {#if hoveredId === s.id}
                <span class="recent-actions">
                  <div class="act-btn" role="button" tabindex="0"
                    onclick={(e: MouseEvent) => { e.stopPropagation(); togglePin(s.id); }}
                    onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); togglePin(s.id); } }}
                    title="Pin">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M16 12V4h1V2H7v2h1v8l-2 2v2h5.2v6h1.6v-6H18v-2l-2-2z"/></svg>
                  </div>
                  <div class="act-btn" role="button" tabindex="0"
                    onclick={(e: MouseEvent) => { e.stopPropagation(); archive(s.id); }}
                    onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); archive(s.id); } }}
                    title="Archive">
                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="21 8 21 21 3 21 3 8"/><rect x="1" y="3" width="22" height="5"/><line x1="10" y1="12" x2="14" y2="12"/></svg>
                  </div>
                </span>
              {:else}
                <span class="recent-time">{relativeTime(s.updatedAt)}</span>
              {/if}
            </div>
          </div>
        {/each}
        {#if unpinnedSessions.length === 0 && pinnedSessions.length === 0}
          <p class="recent-empty">{tl('sessions.noSessions')}</p>
        {/if}
      </div>
    {/if}
  </div>

  <div class="footer">

    <button class="foot-item" class:on={currentPage === 'tools'} onclick={() => navigateTo('tools')}>🔧 {tl('nav.tools')}</button>
    <button class="foot-item" class:on={currentPage === 'skills'} onclick={() => navigateTo('skills')}>⚡ {tl('nav.skills')}</button>

    <button class="foot-item" class:on={currentPage === 'monitor'} onclick={() => navigateTo('monitor')}>📊 {tl('nav.monitor')}</button>
    <button class="foot-item" class:on={currentPage === 'vijnana'} onclick={() => navigateTo('vijnana')}>🧠 {tl('nav.vijnana')}</button>
    <button class="foot-item" class:on={currentPage === 'principles'} onclick={() => navigateTo('principles')}>⚖ {tl('nav.principles')}</button>
    <button class="foot-item" class:on={currentPage === 'settings'} onclick={() => navigateTo('settings')}>🎛 {tl('nav.settings')}</button>
    <span class="version">v0.1.0</span>
  </div>
</aside>

<style>
  .sidebar {
    width: 100%; height: 100vh;
    display: flex; flex-direction: column;
    background: var(--bg-sidebar);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    border-right: 0.5px solid rgba(0, 0, 0, .08);
    padding: var(--space-2) var(--space-2);
    gap: var(--space-1);
  }
  .nav {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
  }
  .session-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .nav-quick {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-1) var(--space-2); font-size: 14px;
    color: var(--text-tertiary); border-radius: var(--radius-sm);
    width: 100%; text-align: left;
    margin-bottom: 1px; flex-shrink: 0;
  }
  .nav-quick:hover { background: rgba(0,0,0,.04); color: var(--text-primary); }
  .nav-quick.on { color: var(--text-primary); font-weight: 500; background: rgba(0,0,0,.06); }

  .section-header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 0 var(--space-2); margin-top: var(--space-1);
    height: 28px; flex-shrink: 0;
  }
  .section-title {
    display: flex; align-items: center; gap: 4px;
    font-size: 12px; font-weight: 600; color: var(--text-tertiary);
    border-radius: 4px; height: 28px;
  }
  .section-title:hover { color: var(--text-primary); }
  .chevron { transition: transform 0.15s; flex-shrink: 0; }
  .chevron.open { transform: rotate(90deg); }
  .view-all {
    font-size: 11px; font-weight: 500; color: var(--text-tertiary);
    padding: 2px 6px; border-radius: 4px; white-space: nowrap;
 transition: color 0.1s;
  }
  .view-all:hover { color: var(--accent); }

  .recents-list {
    display: flex; flex-direction: column;
    overflow-y: auto;
    margin-bottom: 10px;
  }
  .pinned-list {
    overflow-y: auto;
  }
  .recents-scroll {
    flex: 1 1 0;
    min-height: 180px;
    overflow-y: auto;
  }
  .recent-row { position: relative; display: flex; align-items: center; }
  .status-dot {
    width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; margin-right: 5px;
  }
  .status-dot.active {
    background: #22c55e;
    animation: pulse-dot 1.5s ease-in-out infinite;
  }
  .status-dot.error {
    background: #ef4444;
  }
  .status-dot.idle {
    background: var(--text-tertiary);
    opacity: 0.35;
  }
  @keyframes pulse-dot {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .recent-item {
    flex: 1; display: flex; align-items: center;
    padding: 1px var(--space-2); border-radius: var(--radius-sm);
    font-size: 14px; color: var(--text-primary);
    text-align: left; min-height: 26px;
    margin-bottom: 1px; min-width: 0; overflow: hidden;
    cursor: pointer; user-select: none;
  }
  .recent-item:hover { background: rgba(0,0,0,.04); }
  .recent-item.active { background: rgba(0,0,0,.04); }
  .recent-item.active .recent-title { font-weight: 500; }
  .recent-title {
    flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font-size: 13px; font-weight: 400; line-height: 1.3;
  }
  .recent-time {
    font-size: 11px; color: var(--text-secondary); flex-shrink: 0; margin-left: 8px;
    height: 22px; display: flex; align-items: center;
  }
  .recent-actions {
    display: flex; align-items: center; gap: 2px; flex-shrink: 0;
    height: 22px;
  }
  .act-btn {
    display: flex; align-items: center; justify-content: center;
    width: 22px; height: 22px; border-radius: 4px;
    color: #aeaeb2;
    background: transparent !important;
  }
  .act-btn:hover { color: #1d1d1f; background: transparent !important; }

  .recent-empty {
    font-size: 12px; color: var(--text-tertiary);
    padding: var(--space-1) var(--space-2) var(--space-1) 20px; text-align: left;
  }

  .footer {
    padding: var(--space-2) var(--space-1) 0;
    border-top: 0.5px solid rgba(0,0,0,.06);
    display: flex; flex-direction: column; gap: 1px;
  }
  .foot-item {
    display: flex; align-items: center; gap: var(--space-2);
    padding: var(--space-1) var(--space-2); font-size: 14px;
    color: var(--text-tertiary); border-radius: var(--radius-sm);
    width: 100%; text-align: left;
    margin-bottom: 1px;
  }
  .foot-item:hover { background: rgba(0,0,0,.04); color: var(--text-primary); }
  .foot-item.on { color: var(--text-primary); font-weight: 500; }
  .version {
    font-size: 11px; color: var(--text-tertiary);
    padding: var(--space-1) var(--space-2); font-family: var(--font-mono); opacity: .45;
  }
</style>
