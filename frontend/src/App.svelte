<script lang="ts">
  import { store, setPage, saveSelectedProvider, saveTheme, saveLocale, saveSelectedModel, saveSelectedAux, showToast } from './lib/store.svelte';
  import { uiStore } from './lib/stores/ui.svelte';
  import { getTheme } from './lib/themes';
  import { fetchProviders, fetchConfig, API_BASE } from './lib/api';
  import { onMount, onDestroy } from 'svelte';
  import type { SSEEvent } from './lib/types';

  let eventsSource: EventSource | null = null;

  onMount(async () => {
    try {
      const [list, config] = await Promise.all([fetchProviders(), fetchConfig()]);
      store.providers = list;
      if (!store.selectedProvider && list.length > 0) {
        store.selectedProvider = list[0].name;
      }
      if (!store.selectedModel) {
        const cur = list.find(p => p.name === store.selectedProvider);
        if (cur) store.selectedModel = cur.default_model;
      }
    } catch (e) {
      // config/providers 加载失败会让整个应用空壳且无反馈,必须提示。
      console.error('[App] Failed to load config/providers:', e);
      showToast('加载配置失败,请确认 jia 守护进程正在运行', 'error');
    }

    // Connect to server-sent events for real-time cron notifications.
    connectEvents();
  });

  onDestroy(() => {
    eventsSource?.close();
  });

  function connectEvents() {
    try {
      const token = (window as any).__JIA_TOKEN__ || '';
      const url = `${API_BASE}/events${token ? `?token=${encodeURIComponent(token)}` : ''}`;
      eventsSource = new EventSource(url);

      eventsSource.onmessage = (e) => {
        try {
          const event: SSEEvent = JSON.parse(e.data);
          if (event.type === 'cron_notification') {
            showToast(`定时任务「${event.job_name}」已触发`, 'info');
            store.entries = [...store.entries, {
              id: crypto.randomUUID(),
              role: 'assistant' as const,
              content: `🔔 **定时提醒 (${event.job_name})**\n\n${event.response}`,
              createdAt: Date.now(),
            }];
          }
        } catch { /* ignore parse errors */ }
      };

      eventsSource.onerror = () => {
        eventsSource?.close();
        eventsSource = null;
        setTimeout(connectEvents, 30_000);
      };
    } catch { /* EventSource not supported */ }
  }

  $effect(() => { saveSelectedProvider(); });
  $effect(() => { saveSelectedModel(); });
  $effect(() => { saveSelectedAux(); });
  $effect(() => { saveTheme(); });
  $effect(() => { saveLocale(); });

  // Dynamic page title based on locale
  $effect(() => {
    document.title = store.locale === 'zh'
      ? '甲 - Just Intelligence Agent'
      : 'JIA | Just Intelligence Agent';
  });

  // Apply theme CSS variables (accent + optional dark mode)
  $effect(() => {
    const theme = getTheme(store.themeId);
    const root = document.documentElement;
    root.style.setProperty('--accent', theme.accent);
    root.style.setProperty('--accent-hover', theme.hover);
    root.style.setProperty('--accent-light', theme.light);

    const d = theme.mode === 'dark' ? theme.dark! : null;
    root.style.setProperty('--bg-primary', d?.bgPrimary ?? '');
    root.style.setProperty('--bg-secondary', d?.bgSecondary ?? '');
    root.style.setProperty('--bg-tertiary', d?.bgTertiary ?? '');
    root.style.setProperty('--bg-sidebar', d?.bgSecondary ? 'rgba(26, 27, 46, 0.7)' : '');
    root.style.setProperty('--text-primary', d?.textPrimary ?? '');
    root.style.setProperty('--text-secondary', d?.textSecondary ?? '');
    root.style.setProperty('--text-tertiary', d?.textTertiary ?? '');
    root.style.setProperty('--border', d?.border ?? '');
    root.style.setProperty('--shadow-sm', d ? '0 1px 2px rgba(0,0,0,.3)' : '');
    root.style.setProperty('--shadow-md', d ? '0 4px 12px rgba(0,0,0,.4)' : '');
  });

  // Static imports for always-visible components
  import Sidebar from './components/Sidebar.svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import Toast from './components/Toast.svelte';
  import CommandPalette from './components/CommandPalette.svelte';

  // ── Lazy page loading via dynamic import ──────────────────
  let pageCache = $state<Record<string, any>>({});

  async function loadPage(name: string) {
    if (pageCache[name]) return pageCache[name];
    let mod: any;
    switch (name) {
      case 'chat':     mod = await import('./pages/Chat.svelte'); break;
      case 'session':  mod = await import('./pages/Session.svelte'); break;
      case 'projects': mod = await import('./pages/Projects.svelte'); break;
      case 'project': mod = await import('./pages/Project.svelte'); break;
      case 'sessions': mod = await import('./pages/Sessions.svelte'); break;
      case 'tools':    mod = await import('./pages/Tools.svelte'); break;
      case 'skills':   mod = await import('./pages/Skills.svelte'); break;
      case 'cron':     mod = await import('./pages/Cron.svelte'); break;
      case 'monitor':  mod = await import('./pages/Monitor.svelte'); break;
      case 'settings': mod = await import('./pages/Settings.svelte'); break;
      case 'vijnana':  mod = await import('./pages/Vijnana.svelte'); break;
      default: return null;
    }
    pageCache[name] = mod.default;
    return mod.default;
  }

  // Page component resolver
  let Page = $state<any>(null);

  $effect(() => {
    const page = uiStore.currentPage;
    loadPage(page).then(comp => {
      if (uiStore.currentPage === page) Page = comp;
    }).catch(err => {
      console.error(`[App] Failed to load page "${page}":`, err);
    });
  });

  // ── Routing ───────────────────────────────────────────────
  function hashToPage(hash: string): string {
    const p = hash.slice(1);
    // Support #project/<id>
    if (p.startsWith('project/')) {
      const id = p.slice('project/'.length);
      uiStore.projectId = id;
      return 'project';
    }
    // Support #session/<id>
    if (p.startsWith('session/')) {
      const id = p.slice('session/'.length);
      uiStore.activeSessionId = id;
      return 'session';
    }
    const valid = ['chat', 'session', 'sessions', 'projects', 'project', 'tools', 'skills', 'cron', 'monitor', 'settings', 'vijnana'];
    return valid.includes(p) ? p : 'chat';
  }

  setPage(hashToPage(window.location.hash || 'chat') as typeof store.currentPage);

  function onHashChange() {
    setPage(hashToPage(window.location.hash) as typeof store.currentPage);
  }

  // ── ⌘K command palette ────────────────────────────────────
  let cmdPaletteOpen = $state(false);

  function onGlobalKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      cmdPaletteOpen = true;
    }
    if (e.key === 'Escape' && cmdPaletteOpen) {
      cmdPaletteOpen = false;
    }
  }

  // ── Resizable sidebar ─────────────────────────────────────
  let sidebarWidth = $state(248);
  let sidebarDragging = $state(false);

  function onSidebarDividerDown(e: MouseEvent) {
    e.preventDefault();
    sidebarDragging = true;
    const startX = e.clientX;
    const startW = sidebarWidth;

    function onMove(ev: MouseEvent) {
      const delta = ev.clientX - startX;
      sidebarWidth = Math.max(160, Math.min(480, startW + delta));
    }

    function onUp() {
      sidebarDragging = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    }

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }
</script>

<svelte:window onkeydown={onGlobalKeydown} onhashchange={onHashChange} />

<div class="layout">
  <div style="width: {sidebarWidth}px; min-width: {sidebarWidth}px; flex-shrink: 0;">
    <Sidebar />
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="sidebar-divider"
    class:dragging={sidebarDragging}
    onmousedown={onSidebarDividerDown}
    role="separator"
    tabindex="0"
  ></div>
  <main class="main">
    {#if Page}
      <Page />
    {:else}
      <div class="flex-1 flex items-center justify-center">
        <div class="animate-pulse text-[13px]" style="color: var(--text-tertiary)">Loading...</div>
      </div>
    {/if}
  </main>
</div>

<CommandPalette bind:open={cmdPaletteOpen} />
<ConfirmDialog />
<Toast />
