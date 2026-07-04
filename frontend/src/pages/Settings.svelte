<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchProviders } from '../lib/api';
  import { store, getProviderDefaultModel, setProviderDefaultModel, showToast } from '../lib/store.svelte';
  import { settingsStore } from '../lib/stores/settings.svelte';
  import { THEMES, type ThemeDef } from '../lib/themes';
  import { t } from '../lib/i18n';

  let loading = $state(true);
  let tab = $state<'llm' | 'language' | 'theme'>('llm');

  const currentTheme = $derived(store.themeId);
  const themeGroups = $derived(groupByCategory(THEMES));

  // Derived: models for main provider
  const mainProvider = $derived(store.providers.find(p => p.name === store.selectedProvider));
  // Derived: models for aux provider
  const auxProvider = $derived(store.providers.find(p => p.name === store.selectedAuxProvider));

  onMount(async () => {
    try {
      const list = await fetchProviders();
      store.providers = list;
      if (!store.selectedProvider && list.length > 0) {
        store.selectedProvider = list[0].name;
        store.selectedModel = list[0].default_model;
      }
    } catch (e) {
      console.error('[Settings] Failed to load providers:', e);
      showToast('加载失败,请检查守护进程是否运行', 'error');
    }
    loading = false;
  });

  function groupByCategory(themes: ThemeDef[]): Array<[string, ThemeDef[]]> {
    const map = new Map<string, ThemeDef[]>();
    for (const t of themes) {
      const list = map.get(t.category) || [];
      list.push(t);
      map.set(t.category, list);
    }
    return Array.from(map.entries());
  }

  function selectTheme(t: ThemeDef) {
    store.themeId = t.id;
  }

  function onMainProviderChange(e: Event) {
    const name = (e.target as HTMLSelectElement).value;
    store.selectedProvider = name;
    const p = store.providers.find(pp => pp.name === name);
    if (p) store.selectedModel = getProviderDefaultModel(name) ?? p.default_model;
  }

  function onMainModelChange(e: Event) {
    const model = (e.target as HTMLSelectElement).value;
    store.selectedModel = model;
    setProviderDefaultModel(store.selectedProvider, model);
  }

  function onAuxProviderChange(e: Event) {
    const name = (e.target as HTMLSelectElement).value;
    store.selectedAuxProvider = name;
    const p = store.providers.find(pp => pp.name === name);
    if (p) store.selectedAuxModel = getProviderDefaultModel(name) ?? p.default_model;
  }

  function onAuxModelChange(e: Event) {
    const model = (e.target as HTMLSelectElement).value;
    store.selectedAuxModel = model;
    setProviderDefaultModel(store.selectedAuxProvider, model);
  }

  function mainModelValue(): string {
    return store.selectedModel || mainProvider?.default_model || '';
  }

  function auxModelValue(): string {
    return store.selectedAuxModel || auxProvider?.default_model || '';
  }
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('settings.title')}</h2>
  </div>
  <div class="body">
    <nav class="tab-sidebar">
      <button class="tab-btn" class:active={tab === 'llm'} onclick={() => tab = 'llm'}>
        {t('settings.tabLlm')}
      </button>
      <button class="tab-btn" class:active={tab === 'language'} onclick={() => tab = 'language'}>
        {t('settings.tabLanguage')}
      </button>
      <button class="tab-btn" class:active={tab === 'theme'} onclick={() => tab = 'theme'}>
        {t('settings.tabTheme')}
      </button>
    </nav>
    <div class="tab-content">
      {#if tab === 'llm'}
      {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else}
      <!-- Main Model Section -->
      <div class="section">
        <h3 class="section-title">{t('settings.mainModel')}</h3>
        <div class="model-row">
          <label class="field">
            <span class="field-label">{t('settings.mainModelProvider')}</span>
            <select class="select" value={store.selectedProvider} onchange={onMainProviderChange}>
              {#each store.providers as p}
                <option value={p.name}>{p.name} ({p.kind})</option>
              {/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">{t('settings.mainModelModel')}</span>
            <select class="select" value={mainModelValue()} onchange={onMainModelChange}>
              {#each mainProvider?.models ?? [] as m}
                <option value={m}>{m}</option>
              {/each}
            </select>
          </label>
        </div>
      </div>

      <!-- Aux Model Section -->
      <div class="section">
        <h3 class="section-title">{t('settings.auxModel')}</h3>
        <p class="section-note">{t('settings.auxModelNote')}</p>
        <div class="model-row">
          <label class="field">
            <span class="field-label">{t('settings.auxModelProvider')}</span>
            <select class="select" value={store.selectedAuxProvider} onchange={onAuxProviderChange}>
              <option value="">{t('settings.auxNone')}</option>
              {#each store.providers as p}
                <option value={p.name}>{p.name} ({p.kind})</option>
              {/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">{t('settings.auxModelModel')}</span>
            <select class="select" value={auxModelValue()} onchange={onAuxModelChange} disabled={!store.selectedAuxProvider}>
              {#each auxProvider?.models ?? [] as m}
                <option value={m}>{m}</option>
              {/each}
            </select>
          </label>
        </div>
      </div>

      <!-- Provider List (info only) -->
      <div class="section">
        <h3 class="section-title">{t('settings.provider')}</h3>
        <div class="provider-list">
          {#each store.providers as p}
            <div class="provider-card">
              <div class="provider-card-header">
                <span class="provider-name">{p.name}</span>
                <span class="provider-kind">({p.kind})</span>
              </div>
              <div class="provider-card-models">
                <span class="models-label">{t('settings.models')}:</span>
                <span class="models-list">{p.models.join(', ')}</span>
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}

  {#if tab === 'language'}
    <div class="section">
      <h3 class="section-title">{t('settings.language')}</h3>
      <div class="lang-options">
        <label class="lang-option">
          <input
            type="radio"
            name="locale"
            value="zh"
            checked={settingsStore.locale === 'zh'}
            onchange={() => settingsStore.locale = 'zh'}
          />
          <span>{t('settings.langZh')}</span>
        </label>
        <label class="lang-option">
          <input
            type="radio"
            name="locale"
            value="en"
            checked={settingsStore.locale === 'en'}
            onchange={() => settingsStore.locale = 'en'}
          />
          <span>{t('settings.langEn')}</span>
        </label>
      </div>
    </div>
  {/if}

  {#if tab === 'theme'}
    <div class="section">
      <h3 class="section-title">{t('theme.title')} <span class="count">{THEMES.length}</span></h3>
      {#each themeGroups as [category, themes]}
        <div class="theme-category">
          <h4 class="theme-cat-title">{category} <span class="count">{themes.length}</span></h4>
          <div class="theme-grid">
            {#each themes as t}
              <button
                class="theme-card"
                class:selected={currentTheme === t.id}
                onclick={() => selectTheme(t)}
              >
                <div class="swatch" style="background: {t.accent}">
                  {#if currentTheme === t.id}
                    <span class="check">✓</span>
                  {/if}
                </div>
                <div class="info">
                  <span class="label">{t.label}{#if t.mode === 'dark'} <span class="moon">🌙</span>{/if}</span>
                  <span class="source">{t.source}</span>
                </div>
              </button>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
    </div>
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  .title { font-size: 16px; font-weight: 600; }

  .body {
    flex: 1;
    display: flex;
    overflow: hidden;
  }

  .tab-sidebar {
    width: 160px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 8px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .tab-btn {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--text-secondary);
    border-radius: var(--radius-sm);
    text-align: left;
    transition: background .15s, color .15s;
  }
  .tab-btn:hover {
    color: var(--text-primary);
    background: var(--bg-tertiary);
  }
  .tab-btn.active {
    background: var(--bg-tertiary);
    color: var(--text-primary);
    font-weight: 600;
  }

  .tab-content {
    flex: 1;
    overflow-y: auto;
    padding: 20px;
  }
  .section { margin-bottom: 24px; }
  .section-title { font-size: 14px; font-weight: 600; margin-bottom: 10px; color: var(--text-secondary); text-transform: uppercase; letter-spacing: .5px; }
  .section-note { font-size: 12px; color: var(--text-tertiary); margin-bottom: 10px; line-height: 1.4; }
  .msg { color: var(--text-secondary); font-size: 14px; }

  .model-row { display: flex; gap: 16px; flex-wrap: wrap; }
  .field { display: flex; align-items: center; gap: 8px; }
  .field-label { font-size: 13px; color: var(--text-secondary); min-width: 50px; }
  .select {
    font-size: 13px; padding: 4px 10px; border: 1px solid var(--border);
    border-radius: var(--radius-sm); background: var(--bg-secondary);
    color: var(--text-primary); outline: none; min-width: 200px;
  }
  .select:focus { border-color: var(--accent); }
  .select:disabled { opacity: .5; cursor: not-allowed; }

  .provider-list { display: flex; flex-direction: column; gap: 8px; }
  .provider-card {
    padding: 10px 14px; border: 1px solid var(--border); border-radius: var(--radius-md);
  }
  .provider-card-header { display: flex; align-items: center; gap: 8px; margin-bottom: 4px; }
  .provider-name { font-weight: 600; font-size: 14px; }
  .provider-kind { font-size: 12px; color: var(--text-tertiary); }
  .provider-card-models { display: flex; align-items: baseline; gap: 6px; }
  .models-label { font-size: 12px; color: var(--text-tertiary); }
  .models-list { font-size: 12px; color: var(--text-secondary); }

  .lang-options { display: flex; gap: 8px; }
  .lang-option {
    display: flex; align-items: center; gap: 6px;
    padding: 8px 14px; border: 1px solid var(--border); border-radius: var(--radius-md);
    cursor: pointer; transition: all .15s; font-size: 14px;
  }
  .lang-option:has(input:checked) {
    border-color: var(--accent); background: var(--accent-light);
  }
  .lang-option:hover { border-color: var(--accent); }

  .theme-category { margin-bottom: var(--space-5); }
  .theme-cat-title {
    font-size: 11px; font-weight: 600; color: var(--text-tertiary);
    text-transform: uppercase; letter-spacing: 1px; margin-bottom: var(--space-3);
    display: flex; align-items: center; gap: var(--space-2);
  }
  .theme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-2);
  }
  .theme-card {
    display: flex; align-items: center; gap: var(--space-4);
    padding: var(--space-4); border: 1px solid var(--border);
    border-radius: var(--radius-md); background: var(--bg-secondary);
    cursor: pointer; transition: all var(--duration-normal) var(--ease-out);
    text-align: left;
  }
  .theme-card:hover {
    border-color: var(--accent);
    background: var(--bg-tertiary);
    transform: translateY(-1px);
    box-shadow: var(--shadow-sm);
  }
  .theme-card.selected {
    border-color: var(--accent);
    background: var(--accent-light);
    box-shadow: 0 0 16px var(--accent-glow);
  }
  .swatch {
    width: 40px; height: 40px; border-radius: 50%; flex-shrink: 0;
    display: flex; align-items: center; justify-content: center;
    color: #fff; font-size: 15px; font-weight: 600;
    box-shadow: 0 2px 8px rgba(0,0,0,.3);
  }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .label { font-size: 14px; font-weight: 550; color: var(--text-primary); }
  .source { font-size: 11px; color: var(--text-tertiary); line-height: 1.4; }
  .count { font-size: 10px; color: var(--text-tertiary); font-weight: 400; }
</style>
