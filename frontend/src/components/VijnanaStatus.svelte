<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchVijnana } from '../lib/api';
  import type { VijnanaManas, VijnanaEntropy } from '../lib/types';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';
  import { formatTime } from '../lib/time';

  let { hideHistory = false, statusOnly = false }: { hideHistory?: boolean; statusOnly?: boolean } = $props();

  let manas = $state<VijnanaManas | null>(null);
  let entropy = $state<VijnanaEntropy | null>(null);
  let loading = $state(true);
  let expanded = $state<number | null>(null);
  let showConfig = $state(false);

  onMount(async () => {
    try {
      const data = await fetchVijnana();
      manas = data.manas;
      entropy = data.entropy;
    } catch {
      showToast(t('vijnana.loadError'), 'error');
    }
    loading = false;
  });

  function pct(v: number) { return Math.round(v * 100); }
  function barWidth(v: number) { return Math.max(2, pct(v)); }

  function barColor(v: number): string {
    if (v < 0.3) return 'var(--success)';
    if (v < 0.6) return 'var(--warning)';
    return 'var(--error)';
  }

  function agGaugeColor(v: number): string {
    if (v < 0.25) return 'var(--success)';
    if (v < 0.5) return 'var(--warning)';
    return 'var(--error)';
  }

  function agFace(v: number): string {
    if (v < 0.15) return '😴';
    if (v < 0.3) return '🙂';
    if (v < 0.5) return '🤔';
    if (v < 0.7) return '😟';
    return '😰';
  }

  function agLabel(v: number): string {
    if (v < 0.15) return 'Deep trust';
    if (v < 0.3) return 'Comfortable';
    if (v < 0.5) return 'Questioning';
    if (v < 0.7) return 'Skeptical';
    return 'Paranoid';
  }

  function entropyDelta(before: number, after: number): string {
    return `${(before * 100).toFixed(0)} → ${(after * 100).toFixed(0)}`;
  }

  function bucketWidth(count: number, max: number) {
    return max > 0 ? Math.max(4, (count / max) * 100) : 0;
  }

  function toggle(row: number) {
    expanded = expanded === row ? null : row;
  }

  function entropyPoint(v: number, i: number): string {
    const a = (i / 4) * 2 * Math.PI - Math.PI / 2;
    const r = Math.max(0.5, v * 68);
    return `${(r * Math.cos(a)).toFixed(1)},${(r * Math.sin(a)).toFixed(1)}`;
  }

  const radarRaw = $derived(dims.map((d, i) => entropyPoint(entropy?.current[d.key] ?? 0, i)).join(' '));
  const radarWeighted = $derived(dims.map((d, i) => entropyPoint((entropy?.current[d.key] ?? 0) * d.weight * 4, i)).join(' '));

  const dims = [
    { key: 'staleness' as const,     labelKey: 'vijnana.staleness' as const,     weight: 0.30, icon: '🕐', color: '#f59e0b' },
    { key: 'contradiction' as const, labelKey: 'vijnana.contradiction' as const, weight: 0.20, icon: '⚡', color: '#ef4444' },
    { key: 'redundancy' as const,    labelKey: 'vijnana.redundancy' as const,    weight: 0.25, icon: '📋', color: '#8b5cf6' },
    { key: 'access_decay' as const,  labelKey: 'vijnana.accessDecay' as const,   weight: 0.25, icon: '💤', color: '#3b82f6' },
  ];
</script>

<div class="status-panel">
  {#if loading}
    <p class="msg">{t('common.loading')}</p>
  {:else if !manas}
    <p class="msg">{t('common.noData')}</p>
  {:else}
    {#if !statusOnly}
    <!-- ── Self Model ────────────────────────────── -->
    <section class="card">
      <h3 class="section-title">{t('vijnana.selfModel')}</h3>
      <div class="self-model">
        <div class="ag-ring-wrap">
          <div class="ag-face" style="transform:scale({1 + manas.atma_graha * 0.3})">{agFace(manas.atma_graha)}</div>
          <div class="ag-value" style="color:{agGaugeColor(manas.atma_graha)}">
            {manas.atma_graha.toFixed(2)}
          </div>
          <div class="ag-mood" style="color:{agGaugeColor(manas.atma_graha)}">{agLabel(manas.atma_graha)}</div>
          <div class="gauge-track">
            <div class="gauge-fill" style="width:{pct(manas.atma_graha)}%;background:{agGaugeColor(manas.atma_graha)}"></div>
          </div>
        </div>
        <div class="model-metrics">
          <div class="model-metric">
            <span class="metric-icon">{manas.is_stable ? '✅' : '🔄'}</span>
            <span class="metric-num">{manas.stable_epochs}</span>
            <span class="metric-desc">{t('vijnana.stability')}</span>
          </div>
          <div class="model-metric">
            <span class="metric-icon">🔄</span>
            <span class="metric-num">{manas.total_turns}</span>
            <span class="metric-desc">Turns</span>
          </div>
          <div class="model-metric">
            <span class="metric-icon">🌱</span>
            <span class="metric-num">{manas.total_seeds}</span>
            <span class="metric-desc">{t('vijnana.tabSeeds')}</span>
          </div>
          <div class="model-metric">
            <span class="metric-icon">📊</span>
            <span class="metric-num">{manas.stable_pattern_count}</span>
            <span class="metric-desc">Patterns</span>
          </div>
        </div>
      </div>
    </section>

    <!-- ── Memory Entropy ───────────────────────────────── -->
    {#if entropy}
      <section class="card">
        <h3 class="section-title">{t('vijnana.memEntropy')}</h3>

        <div class="entropy-grid">
          <section class="card" style="display:flex;align-items:center;justify-content:center;padding:8px">
            <svg viewBox="-90 -90 180 180" class="radar-svg">
              <circle cx="0" cy="0" r="17" fill="none" stroke="var(--bg-tertiary)" stroke-width="0.5"/>
              <circle cx="0" cy="0" r="34" fill="none" stroke="var(--bg-tertiary)" stroke-width="0.5"/>
              <circle cx="0" cy="0" r="51" fill="none" stroke="var(--error)" stroke-width="0.8" stroke-dasharray="2,2"/>
              <circle cx="0" cy="0" r="68" fill="none" stroke="var(--bg-tertiary)" stroke-width="0.5"/>
              <line x1="0" y1="-68" x2="0" y2="68" stroke="var(--bg-tertiary)" stroke-width="0.5"/>
              <line x1="-68" y1="0" x2="68" y2="0" stroke="var(--bg-tertiary)" stroke-width="0.5"/>
              <!-- Icons on invisible circle at r=70, centered properly -->
              <text x="0" y="-84" text-anchor="middle" dominant-baseline="central" font-size="12" fill="var(--text-primary)">{dims[0].icon}</text>
              <text x="84" y="0" text-anchor="middle" dominant-baseline="central" font-size="12" fill="var(--text-primary)">{dims[1].icon}</text>
              <text x="0" y="84" text-anchor="middle" dominant-baseline="central" font-size="12" fill="var(--text-primary)">{dims[2].icon}</text>
              <text x="-84" y="0" text-anchor="middle" dominant-baseline="central" font-size="12" fill="var(--text-primary)">{dims[3].icon}</text>
              <polygon points={radarRaw} fill={barColor(entropy.current.total)} fill-opacity="0.25" stroke={barColor(entropy.current.total)} stroke-width="1.2"/>
              <polygon points={radarWeighted} fill={barColor(entropy.current.total)} fill-opacity="0.12" stroke={barColor(entropy.current.total)} stroke-width="0.6" stroke-dasharray="2,1"/>
            </svg>
          </section>

          <section class="card">
            {#each dims as dim}
              {@const raw = entropy.current[dim.key]}
              {@const contrib = raw * dim.weight}
              <div class="rleg-item">
                <span class="rleg-icon">{dim.icon}</span>
                <span class="rleg-name">{t(dim.labelKey)}</span>
                <span class="rleg-raw" style="color:{dim.color}">{raw.toFixed(2)}</span>
                <span class="rleg-w">×{dim.weight.toFixed(2)}</span>
                <span class="rleg-c" style="color:{dim.color}">={contrib.toFixed(2)}</span>
                <div class="rleg-track">
                  <div class="rleg-fill" style="width:{Math.min(100, raw * 100)}%;background:{dim.color}"></div>
                  <div class="dim-threshold" style="left:75%"></div>
                </div>
              </div>
            {/each}
          </section>
        </div>

        <div class="entropy-summary">
          <span class="sum-label">{t('vijnana.total')} = </span>
          {#each dims as dim, i}
            {@const raw = entropy.current[dim.key]}
            <span style="color:{dim.color}">{raw.toFixed(2)}×{dim.weight.toFixed(2)}</span>
            {#if i < dims.length - 1}<span style="color:var(--text-tertiary)"> + </span>{/if}
          {/each}
          <span> = <b style="color:{barColor(entropy.current.total)}">{entropy.current.total.toFixed(2)}</b></span>
          <span style="color:var(--text-tertiary);margin-left:4px">(threshold 0.75)</span>
          {#if entropy.current.total >= 0.75}
            <span style="color:var(--error);margin-left:8px;font-size:12px;font-weight:600">⚠ exceeded</span>
          {/if}
        </div>
      </section>
    {/if}

    {#if !hideHistory}
      <!-- ── Dissolution History ────────────────────────────── -->
      <section class="card">
        <div class="section-header">
          <h3 class="section-title">{t('vijnana.dissHistory')}</h3>
          <button class="help-btn" onclick={() => showConfig = !showConfig} title={t('vijnana.showParams')}>
            {showConfig ? t('vijnana.hideParams') : t('vijnana.showParams')}
          </button>
        </div>

        {#if showConfig}
          <div class="config-card">
            <div class="config-line">{t('vijnana.configFormula')}</div>
            <div class="config-line">
              <span class="config-tag del">{t('vijnana.configDelete')}</span>
              <span class="config-tag weak">{t('vijnana.configWeaken')}</span>
              <span class="config-tag keep">{t('vijnana.configKeep')}</span>
            </div>
            <div class="config-line">{t('vijnana.configNever')}</div>
          </div>
        {/if}

        {#if entropy.dissolution_history.length === 0}
          <p class="msg">{t('vijnana.noDissRecords')}</p>
        {:else}
          <div class="history-table-wrap">
            <table class="history-table">
              <thead>
                <tr>
                  <th>{t('vijnana.colTime')}</th>
                  <th class="num">{t('vijnana.colExamined')}</th>
                  <th class="num">{t('vijnana.colDissolved')}</th>
                  <th class="num">{t('vijnana.colWeakened')}</th>
                  <th>{t('vijnana.colEntropyChg')}</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {#each entropy.dissolution_history as ev, i}
                  {@const maxBucket = Math.max(ev.dissolved, ev.weakened, ev.kept, ev.protected, 1)}
                  <tr>
                    <td class="time-cell">{formatTime(ev.timestamp)}</td>
                    <td class="num">{ev.examined}</td>
                    <td class="num dissolved">{ev.dissolved}</td>
                    <td class="num weakened">{ev.weakened}</td>
                    <td class="delta-cell">
                      <span class="delta" class:big-drop={(ev.entropy_before - ev.entropy_after) > 0.15}>
                        {entropyDelta(ev.entropy_before, ev.entropy_after)}
                      </span>
                    </td>
                    <td class="toggle-cell">
                      <button class="toggle-btn" onclick={() => toggle(i)}>
                        {expanded === i ? t('vijnana.collapse') : t('vijnana.expand')}
                      </button>
                    </td>
                  </tr>
                  {#if expanded === i}
                    <tr class="expanded-row">
                      <td colspan="6">
                        <div class="expanded-body">
                          <div class="expanded-section">
                            <div class="expanded-label">{t('vijnana.scoreDist')}</div>
                            <div class="bucket-list">
                              <div class="bucket-row">
                                <span class="bucket-label del-label">{t('vijnana.bucketDelete')}</span>
                                <div class="bucket-track">
                                  <div class="bucket-fill del-fill" style="width:{bucketWidth(ev.dissolved, maxBucket)}%"></div>
                                </div>
                                <span class="bucket-count">{ev.dissolved}</span>
                              </div>
                              <div class="bucket-row">
                                <span class="bucket-label weak-label">{t('vijnana.bucketWeaken')}</span>
                                <div class="bucket-track">
                                  <div class="bucket-fill weak-fill" style="width:{bucketWidth(ev.weakened, maxBucket)}%"></div>
                                </div>
                                <span class="bucket-count">{ev.weakened}</span>
                              </div>
                              <div class="bucket-row">
                                <span class="bucket-label keep-label">{t('vijnana.bucketKeep')}</span>
                                <div class="bucket-track">
                                  <div class="bucket-fill keep-fill" style="width:{bucketWidth(ev.kept, maxBucket)}%"></div>
                                </div>
                                <span class="bucket-count">{ev.kept}</span>
                              </div>
                              <div class="bucket-row">
                                <span class="bucket-label prot-label">{t('vijnana.bucketProtected')}</span>
                                <div class="bucket-track">
                                  <div class="bucket-fill prot-fill" style="width:{bucketWidth(ev.protected, maxBucket)}%"></div>
                                </div>
                                <span class="bucket-count">{ev.protected}</span>
                              </div>
                            </div>
                          </div>
                          {#if ev.dissolved_sample.length > 0}
                            <div class="expanded-section">
                              <div class="expanded-label">{t('vijnana.dissSample')}</div>
                              <div class="sample-list">
                                {#each ev.dissolved_sample as d}
                                  <div class="sample-item">
                                    <span class="sample-nature">{d.nature}</span>
                                    <span class="sample-sep">/</span>
                                    <span class="sample-source">{d.source}</span>
                                    <span class="sample-sep">/</span>
                                    <span class="sample-dim">{d.primary_dim}</span>
                                  </div>
                                {/each}
                              </div>
                            </div>
                          {/if}
                        </div>
                      </td>
                    </tr>
                  {/if}
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </section>
    {/if}
  {/if}
  {/if}
</div>

<style>
  .status-panel { display: flex; flex-direction: column; gap: 16px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }
  .card { border: 1px solid var(--border); border-radius: var(--radius-md); padding: 16px 18px; }
  .section-title { font-size: 13px; font-weight: 600; color: var(--text-secondary); margin: 0 0 10px 0; }

  /* Self Model */
  .self-model { display: flex; gap: 24px; align-items: center; }
  .ag-ring-wrap { flex: 1; text-align: center; }
  .ag-face { font-size: 32px; transition: transform 0.4s; margin-bottom: 4px; }
  .ag-value { font-size: 28px; font-weight: 700; }
  .ag-mood { font-size: 11px; margin-top: 2px; }
  .gauge-track { height: 6px; background: var(--bg-tertiary); border-radius: 3px; overflow: hidden; margin-top: 8px; }
  .gauge-fill { height: 100%; border-radius: 3px; transition: width 0.4s; }

  .model-metrics { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; flex: 1; }
  .model-metric { text-align: center; padding: 8px; border-radius: var(--radius-sm); background: var(--bg-secondary); }
  .metric-icon { font-size: 18px; display: block; margin-bottom: 2px; }
  .metric-num { font-size: 20px; font-weight: 700; display: block; }
  .metric-desc { font-size: 11px; color: var(--text-tertiary); }

  /* Entropy Stack */
  .entropy-stack { margin-bottom: 16px; }
  /* Radar chart */
  .entropy-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-bottom: 14px; }
  .radar-svg { width: 180px; height: 180px; }
  .rleg-item { display: flex; align-items: center; gap: 6px; font-size: 12px; }
  .rleg-icon { font-size: 13px; flex-shrink: 0; }
  .rleg-name { color: var(--text-secondary); width: 48px; flex-shrink: 0; }
  .rleg-raw { font-weight: 700; font-size: 12px; width: 32px; text-align: right; flex-shrink: 0; }
  .rleg-w { color: var(--text-tertiary); font-size: 10px; width: 30px; text-align: center; flex-shrink: 0; }
  .rleg-c { font-weight: 600; font-size: 11px; width: 34px; text-align: right; flex-shrink: 0; }
  .rleg-track { flex: 1; height: 5px; background: var(--bg-tertiary); border-radius: 3px; overflow: hidden; position: relative; }
  .rleg-fill { height: 100%; border-radius: 2px; transition: width .5s; }
  .dim-threshold { position: absolute; top: 0; width: 1px; height: 100%; background: var(--error); opacity: 0.5; }

  /* Entropy summary */
  .entropy-summary { font-size: 12px; color: var(--text-secondary); padding-top: 10px; border-top: 1px solid var(--border); display: flex; flex-wrap: wrap; align-items: baseline; gap: 2px; }
  .sum-label { color: var(--text-primary); font-weight: 600; margin-right: 4px; }

  /* History Table */
  .history-table-wrap { max-height: 400px; overflow-y: auto; }
  .history-table { width: 100%; border-collapse: collapse; font-size: 13px; }
  .history-table th { text-align: left; font-size: 11px; font-weight: 600; color: var(--text-tertiary); padding: 6px 8px; border-bottom: 1px solid var(--border); }
  .history-table td { padding: 6px 8px; border-bottom: 1px solid var(--border); color: var(--text-secondary); }
  .num { text-align: right; }
  .mono { font-family: var(--font-mono, monospace); font-size: 11px; }
  .dissolved { color: var(--error); font-weight: 600; }
  .weakened { color: var(--warning); }
  .entropy-down { color: var(--success); }
  .entropy-up { color: var(--error); }
  .expand-row { cursor: pointer; }
  .expand-row:hover td { background: var(--bg-secondary); }
  .buckets { display: flex; gap: 1px; height: 18px; border-radius: 4px; overflow: hidden; margin: 4px 0; }
  .bucket { height: 100%; }
  .bucket.dissolved { background: var(--error); }
  .bucket.weakened { background: var(--warning); }
  .bucket.kept { background: var(--success); }
  .bucket.protected { background: var(--accent); }
  .detail-row td { padding: 10px 8px 14px; }
  .sample-list { display: flex; flex-wrap: wrap; gap: 4px; }
  .sample-chip { font-size: 11px; border: 1px solid var(--border); border-radius: 4px; padding: 3px 8px; }
  .sample-nature { font-weight: 600; }
  .sample-sep { color: var(--text-tertiary); margin: 0 2px; }
  .sample-source { color: var(--text-secondary); }
  .sample-dim { color: var(--text-tertiary); }
  .section-header { display: flex; align-items: center; gap: 8px; margin-bottom: 14px; }
  .help-btn { font-size: 11px; width: 18px; height: 18px; border-radius: 50%; border: 1px solid var(--border); color: var(--text-tertiary); display: flex; align-items: center; justify-content: center; transition: all .15s; }
  .help-btn:hover { border-color: var(--accent); color: var(--accent); }
  .config-card { background: var(--bg-secondary); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 10px 14px; margin-bottom: 14px; display: flex; flex-direction: column; gap: 6px; }
  .config-line { font-size: 12px; color: var(--text-secondary); display: flex; gap: 8px; align-items: center; }
  .config-tag { font-size: 10px; padding: 1px 6px; border-radius: 3px; font-weight: 600; }
  .config-tag.del { background: var(--error-light); color: var(--error); }
  .config-tag.weak { background: var(--warning-light); color: var(--warning); }
  .config-tag.keep { background: var(--success-light); color: var(--success); }
</style>