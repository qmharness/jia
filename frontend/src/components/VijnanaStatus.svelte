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

  const dims = [
    { key: 'staleness' as const,     labelKey: 'vijnana.staleness' as const,     weight: 0.30 },
    { key: 'contradiction' as const, labelKey: 'vijnana.contradiction' as const, weight: 0.20 },
    { key: 'redundancy' as const,    labelKey: 'vijnana.redundancy' as const,    weight: 0.25 },
    { key: 'access_decay' as const,  labelKey: 'vijnana.accessDecay' as const,   weight: 0.25 },
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
        <div class="entropy-gauge-row">
          <div class="entropy-big-num" style="color:{barColor(entropy.current.total)}" class:over={entropy.current.total >= 0.75}>
            {(entropy.current.total * 100).toFixed(0)}<span class="entropy-unit">%</span>
          </div>
          <div class="entropy-ring">
            <svg viewBox="0 0 80 80" class="entropy-svg">
              <circle cx="40" cy="40" r="34" fill="none" stroke="var(--bg-tertiary)" stroke-width="6"/>
              <circle cx="40" cy="40" r="34" fill="none" stroke={barColor(entropy.current.total)} stroke-width="6"
                stroke-dasharray="{2 * Math.PI * 34}" stroke-dashoffset="{2 * Math.PI * 34 * (1 - Math.min(entropy.current.total, 1))}"
                stroke-linecap="round" transform="rotate(-90 40 40)" style="transition:stroke-dashoffset .5s"/>
            </svg>
            <div class="threshold-tick" style="transform:rotate({-90 + 270 * 0.75}deg) translateX(34px)"></div>
          </div>
        </div>

        <div class="weighted-bars">
          {#each dims as dim}
            {@const raw = entropy.current[dim.key]}
            {@const contrib = raw * dim.weight}
            <div class="wbar-row">
              <span class="wbar-label">{t(dim.labelKey)}</span>
              <span class="wbar-raw">{raw.toFixed(2)}</span>
              <span class="wbar-times">×{dim.weight.toFixed(2)}</span>
              <div class="wbar-track">
                <div class="wbar-fill" style="width:{Math.min(100, contrib * 100)}%;background:{barColor(raw)}"></div>
              </div>
              <span class="wbar-contrib" style="color:{barColor(raw)}">={contrib.toFixed(2)}</span>
            </div>
          {/each}
        </div>

        <div class="threshold-line">
          <span class="threshold-label">→ {t('vijnana.threshold')} 0.75</span>
          {#if entropy.current.total >= 0.75}
            <span class="threshold-over">⚠ {t('vijnana.threshold')} exceeded</span>
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
  /* Entropy gauge */
  .entropy-gauge-row { display: flex; align-items: center; gap: 20px; margin-bottom: 16px; }
  .entropy-big-num { font-size: 36px; font-weight: 700; }
  .entropy-unit { font-size: 18px; font-weight: 400; }
  .entropy-ring { position: relative; width: 64px; height: 64px; }
  .entropy-svg { width: 100%; height: 100%; display: block; }
  .threshold-tick { position: absolute; top: 50%; left: 50%; width: 2px; height: 8px; background: var(--error); border-radius: 1px; transform-origin: 0 0; }

  /* Weighted bars */
  .weighted-bars { display: flex; flex-direction: column; gap: 6px; }
  .wbar-row { display: flex; align-items: center; gap: 8px; font-size: 12px; }
  .wbar-label { width: 70px; color: var(--text-secondary); flex-shrink: 0; }
  .wbar-raw { width: 32px; text-align: right; font-weight: 600; flex-shrink: 0; }
  .wbar-times { width: 32px; color: var(--text-tertiary); text-align: center; flex-shrink: 0; }
  .wbar-track { flex: 1; height: 8px; background: var(--bg-tertiary); border-radius: 4px; overflow: hidden; }
  .wbar-fill { height: 100%; border-radius: 4px; transition: width .5s; }
  .wbar-contrib { width: 42px; font-weight: 600; text-align: right; flex-shrink: 0; }

  /* Threshold */
  .threshold-line { font-size: 12px; color: var(--text-tertiary); margin-top: 12px; display: flex; gap: 12px; align-items: center; }
  .threshold-over { color: var(--error); font-weight: 600; }

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