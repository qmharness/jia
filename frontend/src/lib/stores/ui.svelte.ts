import type { ConfirmState, PageId, SessionMeta } from '../types';

export const uiStore = $state({
  currentPage: 'chat' as PageId,
  sessions: [] as SessionMeta[],
  sidebarTick: 0,
  streamingSessionIds: {} as Record<string, boolean>,
  activeSessionId: null as string | null,
  loadedSessionId: null as string | null,
  projectId: '' as string,
  confirmState: null as ConfirmState | null,
  toast: null as { message: string; type: 'info' | 'error' | 'success' } | null,
});

export function setPage(page: PageId) {
  uiStore.currentPage = page;
  const cur = window.location.hash.slice(1);
  if (cur !== page && !cur.startsWith(page + '/')) {
    window.location.hash = page;
  }
}

export function showToast(message: string, type: 'info' | 'error' | 'success' = 'info') {
  uiStore.toast = { message, type };
}

export function dismissToast() {
  uiStore.toast = null;
}

export function clearActiveSession() {
  uiStore.activeSessionId = null;
  uiStore.loadedSessionId = null;
}
