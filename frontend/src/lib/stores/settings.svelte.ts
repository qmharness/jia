import type { Provider } from '../types';

const LS_PROVIDER = 'jia_provider';
const LS_THEME = 'jia_theme';
const LS_LOCALE = 'jia_locale';
const LS_MODEL = 'jia_model';
const LS_SESSION_PROVIDERS = 'jia_session_providers';
const LS_SESSION_MODELS = 'jia_session_models';
const LS_PROVIDER_DEFAULT_MODELS = 'jia_provider_default_models';
const LS_AUX_PROVIDER = 'jia_aux_provider';
const LS_AUX_MODEL = 'jia_aux_model';

function loadStr(key: string, fallback: string): string {
  try { return localStorage.getItem(key) ?? fallback; } catch { return fallback; }
}
function saveStr(key: string, val: string) {
  try { localStorage.setItem(key, val); } catch { /* noop */ }
}
function loadJson(key: string): Record<string, string> {
  try { const raw = localStorage.getItem(key); return raw ? JSON.parse(raw) : {}; } catch { return {}; }
}
function saveJson(key: string, val: Record<string, string>) {
  try { localStorage.setItem(key, JSON.stringify(val)); } catch { /* noop */ }
}

export const settingsStore = $state({
  providers: [] as Provider[],
  selectedProvider: loadStr(LS_PROVIDER, ''),
  selectedModel: loadStr(LS_MODEL, '') as string,
  selectedAuxProvider: loadStr(LS_AUX_PROVIDER, '') as string,
  selectedAuxModel: loadStr(LS_AUX_MODEL, '') as string,
  themeId: loadStr(LS_THEME, 'jiamu'),
  locale: (loadStr(LS_LOCALE, 'zh') as 'zh' | 'en'),
  sessionProviders: loadJson(LS_SESSION_PROVIDERS) as Record<string, string>,
  sessionModels: loadJson(LS_SESSION_MODELS) as Record<string, string>,
  providerDefaultModels: loadJson(LS_PROVIDER_DEFAULT_MODELS) as Record<string, string>,
});

// ── Session-scoped model selection ─────────────────────────
export function getSessionProvider(): string {
  // Note: depends on uiStore for activeSessionId.
  // Since this creates a circular import, this function is re-exported
  // from store.svelte.ts (compatibility layer) where both stores are available.
  return settingsStore.selectedProvider;
}

export function getSessionModel(): string {
  return settingsStore.selectedModel;
}

export function setSessionProvider(sessionId: string, provider: string) {
  if (provider === settingsStore.selectedProvider) {
    delete settingsStore.sessionProviders[sessionId];
    delete settingsStore.sessionModels[sessionId];
  } else {
    settingsStore.sessionProviders[sessionId] = provider;
  }
  saveJson(LS_SESSION_PROVIDERS, settingsStore.sessionProviders);
}

export function setSessionModel(sessionId: string, model: string) {
  if (model === settingsStore.selectedModel) {
    delete settingsStore.sessionModels[sessionId];
  } else {
    settingsStore.sessionModels[sessionId] = model;
  }
  saveJson(LS_SESSION_MODELS, settingsStore.sessionModels);
}

export function getProviderDefaultModel(providerName: string): string | undefined {
  return settingsStore.providerDefaultModels[providerName];
}

export function setProviderDefaultModel(providerName: string, model: string) {
  settingsStore.providerDefaultModels[providerName] = model;
  saveJson(LS_PROVIDER_DEFAULT_MODELS, settingsStore.providerDefaultModels);
}

// ── Manual persistence ────────────────────────────────────
export function saveSelectedProvider() { saveStr(LS_PROVIDER, settingsStore.selectedProvider); }
export function saveTheme() { saveStr(LS_THEME, settingsStore.themeId); }

let _lastLocale = 'zh';
export function saveLocale() {
  if (settingsStore.locale === _lastLocale) return;
  _lastLocale = settingsStore.locale;
  saveStr(LS_LOCALE, settingsStore.locale);
}

let _lastModel = '';
export function saveSelectedModel() {
  if (settingsStore.selectedModel === _lastModel) return;
  _lastModel = settingsStore.selectedModel;
  saveStr(LS_MODEL, settingsStore.selectedModel);
}

let _lastAuxProvider = '';
let _lastAuxModel = '';
export function saveSelectedAux() {
  if (settingsStore.selectedAuxProvider !== _lastAuxProvider) {
    _lastAuxProvider = settingsStore.selectedAuxProvider;
    saveStr(LS_AUX_PROVIDER, settingsStore.selectedAuxProvider);
  }
  if (settingsStore.selectedAuxModel !== _lastAuxModel) {
    _lastAuxModel = settingsStore.selectedAuxModel;
    saveStr(LS_AUX_MODEL, settingsStore.selectedAuxModel);
  }
}
