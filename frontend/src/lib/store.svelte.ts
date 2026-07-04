// ── Compatibility re-exports from split stores ──────────
// All existing code that imports { store, ... } from './store.svelte'
// continues to work unchanged. New code should prefer importing directly
// from './stores/' for finer-grained reactivity boundaries.
//
// The split stores (chat / settings / ui) exist as logical boundaries.
// The unified `store` object below delegates all reads/writes to them.

import { chatStore, updateLastToolResult } from './stores/chat.svelte';
import {
  settingsStore,
  saveSelectedProvider,
  saveTheme,
  saveSelectedModel,
  saveSelectedAux,
  saveLocale,
  getSessionProvider as _getSessionProvider,
  getSessionModel as _getSessionModel,
  setSessionProvider as _setSessionProvider,
  setSessionModel as _setSessionModel,
  getProviderDefaultModel as _getProviderDefaultModel,
  setProviderDefaultModel as _setProviderDefaultModel,
} from './stores/settings.svelte';
import {
  uiStore,
  setPage as _setPage,
  showToast as _showToast,
  dismissToast as _dismissToast,
  clearActiveSession as _clearActiveSession,
} from './stores/ui.svelte';

// Unified store — delegates to split stores via getter/setter
// 统一 store 的类型 = 三个分片 store 的交集(各委托不同的键,无冲突)。
// 显式标注后 store.providers 等访问获得正确类型,消除各调用点的隐式 any。
type UnifiedStore = typeof chatStore & typeof settingsStore & typeof uiStore;

function buildUnifiedStore(): UnifiedStore {
  const obj: Record<string, any> = {};

  for (const [src, keys] of [
    [chatStore, ['entries', 'streamStates', 'cancels', 'streamSessions', 'scrollTick', 'searchQuery', 'loadingMessages']],
    [settingsStore, ['providers', 'selectedProvider', 'selectedModel', 'selectedAuxProvider', 'selectedAuxModel', 'themeId', 'sessionProviders', 'sessionModels', 'providerDefaultModels']],
    [uiStore, ['currentPage', 'sessions', 'activeSessionId', 'loadedSessionId', 'projectId', 'confirmState', 'toast']],
  ] as const) {
    for (const key of keys) {
      Object.defineProperty(obj, key, {
        get() { return (src as any)[key]; },
        set(v: any) { (src as any)[key] = v; },
        enumerable: true,
        configurable: true,
      });
    }
  }

  // locale needs special handling because saveLocale has _lastLocale tracking
  Object.defineProperty(obj, 'locale', {
    get() { return settingsStore.locale; },
    set(v: any) { settingsStore.locale = v; },
    enumerable: true,
    configurable: true,
  });

  return obj as UnifiedStore;
}

// $state wrapping so Svelte tracks property access through the proxy
export const store = $state(buildUnifiedStore());

// ── Re-export all functions ─────────────────────────────────
export { updateLastToolResult };

// Session-scoped functions (combine uiStore + settingsStore)
export function getSessionProvider(): string {
  if (!uiStore.activeSessionId) return settingsStore.selectedProvider;
  return settingsStore.sessionProviders[uiStore.activeSessionId] ?? settingsStore.selectedProvider;
}
export function getSessionModel(): string {
  if (!uiStore.activeSessionId) return settingsStore.selectedModel;
  return settingsStore.sessionModels[uiStore.activeSessionId] ?? settingsStore.selectedModel;
}
export function setSessionProvider(sessionId: string, provider: string) { _setSessionProvider(sessionId, provider); }
export function setSessionModel(sessionId: string, model: string) { _setSessionModel(sessionId, model); }
export function getProviderDefaultModel(providerName: string) { return _getProviderDefaultModel(providerName); }
export function setProviderDefaultModel(providerName: string, model: string) { _setProviderDefaultModel(providerName, model); }

// Persistence
export { saveSelectedProvider, saveTheme, saveSelectedModel, saveSelectedAux, saveLocale };
export function clearActiveSession() { _clearActiveSession(); }

// UI actions
export function setPage(page: typeof uiStore.currentPage) { _setPage(page); }
export function showToast(message: string, type: 'info' | 'error' | 'success' = 'info') { _showToast(message, type); }
export function dismissToast() { _dismissToast(); }
