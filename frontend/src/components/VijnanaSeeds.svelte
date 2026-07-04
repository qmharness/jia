<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchVijnanaSeeds } from '../lib/api';
  import type { VijnanaSeed } from '../lib/types';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let seeds = $state<VijnanaSeed[]>([]);
  let loading = $state(true);
  let search = $state('');
  let natureFilter = $state('');
  let sourceFilter = $state('');
  let palaceFilter = $state('');

  onMount(async () => {
    try {
      const data = await fetchVijnanaSeeds();
      seeds = data.seeds;
    } catch {
      showToast(t('seeds.loadError'), 'error');
    }
    loading = false;
  });

  const allNatures = $derived([...new Set(seeds.map(s => s.nature))].sort());
  const allSources = $derived([...new Set(seeds.map(s => s.source))].sort());
  const allPalaces = $derived([...new Set(seeds.map(s => s.palace))].sort());

  const filtered = $derived(
    seeds.filter(s => {
      if (search && !JSON.stringify(s).toLowerCase().includes(search.toLowerCase())) return false;
      if (natureFilter && s.nature !== natureFilter) return false;
      if (sourceFilter && s.source !== sourceFilter) return false;
      if (palaceFilter && s.palace !== palaceFilter) return false;
      return true;
    })
  );

  // Distribution by nature
  const natureDist = $derived(
    Object.entries(
      seeds.reduce((acc, s) => {
        acc[s.nature] = (acc[s.nature] || 0) + 1;
        return acc;
      }, {} as Record<string, number>)
    ).sort((a, b) => b[1] - a[1])
  );

  const maxNatureCount = $derived(Math.max(...natureDist.map(([, c]) => c), 1));

  function contentPreview(seed: VijnanaSeed): string {
    const c = seed.content;
    if (typeof c === 'object' && c !== null) {
      return (c as Record<string, unknown>).text as string
        || (c as Record<string, unknown>).value as string
        || JSON.stringify(c).slice(0, 80);
    }
    return String(c).slice(0, 80);
  }

  function strengthPct(s: number) { return Math.round(s * 100); }

  function strengthColor(s: number): string {
    if (s > 0.7) return 'var(--success)';
    if (s > 0.4) return 'var(--warning)';
    return 'var(--text-tertiary)';
  }
</script>

<div class="seeds-panel">
  {#if loading}
    <p class="msg">{t('common.loading')}</p>
  {:else}
    <div class="toolbar">
      <span class="count">{t('seeds.count', { filtered: filtered.length, total: seeds.length })}</span>
      <input
        class="search-input"
        type="text"
        placeholder={t('seeds.searchPlaceholder')}
        bind:value={search}
      />
    </div>

    <div class="filters">
      <select bind:value={natureFilter}>
        <option value="">{t('seeds.filterAllNature')}</option>
        {#each allNatures as n}
          <option value={n}>{n}</option>
        {/each}
      </select>
      <select bind:value={sourceFilter}>
        <option value="">{t('seeds.filterAllSource')}</option>
        {#each allSources as s}
          <option value={s}>{s}</option>
        {/each}
      </select>
      <select bind:value={palaceFilter}>
        <option value="">{t('seeds.filterAllPalace')}</option>
        {#each allPalaces as p}
          <option value={p}>{p}</option>
        {/each}
      </select>
    </div>

    <!-- ── Distribution ───────────────────────── -->
    {#if natureDist.length > 0}
      <div class="distro">
        {#each natureDist as [nature, count]}
          <div class="distro-item">
            <span class="distro-label">{nature}</span>
            <div class="distro-track">
              <div
                class="distro-fill"
                style="width:{(count / maxNatureCount) * 100}%"
              ></div>
            </div>
            <span class="distro-count">{count}</span>
          </div>
        {/each}
      </div>
    {/if}

    <!-- ── Table ──────────────────────────────── -->
    {#if filtered.length === 0}
      <p class="msg">{t('seeds.noMatch')}</p>
    {:else}
      <div class="table-wrap">
        <table class="seed-table">
          <thead>
            <tr>
              <th>{t('seeds.colContent')}</th>
              <th>{t('seeds.colNature')}</th>
              <th>{t('seeds.colSource')}</th>
              <th>{t('seeds.colPalace')}</th>
              <th class="num">{t('seeds.colStrength')}</th>
            </tr>
          </thead>
          <tbody>
            {#each filtered as seed}
              <tr>
                <td class="content-cell">
                  <span class="content-text">{contentPreview(seed)}</span>
                  {#if seed.intent_stem}
                    <span class="stem-tag">{seed.intent_stem}</span>
                  {/if}
                </td>
                <td><span class="nature-tag">{seed.nature}</span></td>
                <td class="source-cell">{seed.source}</td>
                <td>{seed.palace}</td>
                <td class="num">
                  <div class="strength-cell">
                    <div class="strength-track">
                      <div
                        class="strength-fill"
                        style="width:{strengthPct(seed.strength)}%;background:{strengthColor(seed.strength)}"
                      ></div>
                    </div>
                    <span class="strength-val">{seed.strength.toFixed(2)}</span>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}
</div>

<style>
  .seeds-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .msg {
    text-align: center;
    color: var(--text-secondary);
    padding: 40px;
  }

  /* ── Toolbar ─────────────────────────── */
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .count {
    font-size: 13px;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .search-input {
    flex: 1;
    max-width: 280px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    font-size: 13px;
    outline: none;
  }
  .search-input:focus {
    border-color: var(--accent);
  }

  /* ── Filters ─────────────────────────── */
  .filters {
    display: flex;
    gap: 8px;
  }

  select {
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    font-size: 12px;
    color: var(--text-secondary);
    outline: none;
  }
  select:focus {
    border-color: var(--accent);
  }

  /* ── Distribution ────────────────────── */
  .distro {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--bg-secondary);
  }

  .distro-item {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .distro-label {
    width: 90px;
    font-size: 12px;
    color: var(--text-secondary);
    flex-shrink: 0;
  }

  .distro-track {
    flex: 1;
    height: 8px;
    background: var(--bg-tertiary);
    border-radius: 4px;
    overflow: hidden;
  }

  .distro-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 4px;
    opacity: 0.6;
    transition: width 0.3s ease;
  }

  .distro-count {
    width: 28px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-primary);
    text-align: right;
    flex-shrink: 0;
  }

  /* ── Table ───────────────────────────── */
  .table-wrap {
    max-height: calc(100vh - 340px);
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .seed-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .seed-table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    padding: 8px 10px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
  }

  .seed-table td {
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    color: var(--text-secondary);
  }

  .seed-table tbody tr:hover {
    background: var(--bg-secondary);
  }

  .seed-table .num { text-align: right; }
  .seed-table th.num { text-align: right; }

  .content-cell {
    max-width: 300px;
  }

  .content-text {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .stem-tag {
    display: inline-block;
    margin-top: 2px;
    padding: 1px 6px;
    background: var(--accent-light);
    color: var(--accent);
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
  }

  .nature-tag {
    display: inline-block;
    padding: 1px 8px;
    background: var(--bg-tertiary);
    border-radius: 3px;
    font-size: 11px;
    font-weight: 500;
  }

  .source-cell {
    font-size: 12px;
    color: var(--text-tertiary);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .strength-cell {
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: flex-end;
  }

  .strength-track {
    width: 60px;
    height: 6px;
    background: var(--bg-tertiary);
    border-radius: 3px;
    overflow: hidden;
  }

  .strength-fill {
    height: 100%;
    border-radius: 3px;
    transition: width 0.3s ease;
  }

  .strength-val {
    width: 32px;
    font-size: 12px;
    font-weight: 600;
    text-align: right;
  }
</style>
