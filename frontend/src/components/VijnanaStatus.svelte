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

  function entropyDelta(before: number, after: number): string {
    return `${(before * 100).toFixed(0)} → ${(after * 100).toFixed(0)}`;
  }

  function bucketWidth(count: number, max: number) {
    return max > 0 ? Math.max(4, (count / max) * 100) : 0;
  }

  function toggle(row: number) {
    expanded = expanded === row ? null : row;
  }

  const dims: Array<{ key: keyof VijnanaEntropy['current']; labelKey: string }> = [
    { key: 'staleness', labelKey: 'vijnana.staleness' },
    { key: 'contradiction', labelKey: 'vijnana.contradiction' },
    { key: 'redundancy', labelKey: 'vijnana.redundancy' },
    { key: 'access_decay', labelKey: 'vijnana.accessDecay' },
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
      <div class="stats-row">
        <div class="stat-block ag-block">
          <div class="stat-label">ātma-grāha</div>
          <div class="ag-value" style="color:{agGaugeColor(manas.atma_graha)}">
            {manas.atma_graha.toFixed(2)}
          </div>
          <div class="gauge-track">
            <div
              class="gauge-fill"
              style="width:{pct(manas.atma_graha)}%;background:{agGaugeColor(manas.atma_graha)}"
            ></div>
          </div>
          <div class="gauge-scale">
            <span>0</span><span>0.25</span><span>0.5</span><span>0.75</span><span>1</span>
          </div>
        </div>

        <div class="stat-block">
          <div class="stat-label">{t('vijnana.stability')}</div>
          <div class="stat-value">
            <span class="badge" class:stable={manas.is_stable} class:unstable={!manas.is_stable}>
              {manas.is_stable ? t('vijnana.stable') : t('vijnana.learning')}
            </span>
          </div>
          <div class="stat-sub">{t('vijnana.epochs', { n: manas.stable_epochs })}</div>
        </div>

        <div class="stat-block">
          <div class="stat-label">{t('vijnana.memorySize')}</div>
          <div class="stat-metrics">
            <div class="metric">{t('vijnana.turns', { n: manas.total_turns })}</div>
            <div class="metric">{t('vijnana.seedsCount', { n: manas.total_seeds })}</div>
            <div class="metric">{t('vijnana.consolidations', { n: manas.consolidation_count })}</div>
            <div class="metric">{t('vijnana.patterns', { n: manas.stable_pattern_count })}</div>
          </div>
        </div>
      </div>
    </section>

    <!-- ── Memory Entropy ───────────────────────────────── -->
    {#if entropy}
      <section class="card">
        <h3 class="section-title">{t('vijnana.memEntropy')}</h3>
        <div class="entropy-bars">
          {#each dims as dim}
            {@const v = entropy.current[dim.key]}
            <div class="entropy-row">
              <span class="entropy-label">{t(dim.labelKey)}</span>
              <div class="entropy-track">
                <div
                  class="entropy-fill"
                  style="width:{barWidth(v)}%;background:{barColor(v)}"
                ></div>
              </div>
              <span class="entropy-val" style="color:{barColor(v)}">{v.toFixed(2)}</span>
            </div>
          {/each}
          <div class="entropy-row total-row">
            <span class="entropy-label">{t('vijnana.total')}</span>
            <div class="entropy-track">
              <div
                class="entropy-fill"
                style="width:{barWidth(entropy.current.total)}%;background:{barColor(entropy.current.total)}"
              ></div>
            </div>
            <span class="entropy-val" style="color:{barColor(entropy.current.total)}">{entropy.current.total.toFixed(2)}</span>
            <span class="threshold-mark">{t('vijnana.threshold')}</span>
          </div>
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
  .status-panel {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .msg {
    text-align: center;
    color: var(--text-secondary);
    padding: 40px;
  }

  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: 16px 18px;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
  }

  .section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
    margin: 0;
  }

  .section-title:not(.section-header .section-title) {
    margin-bottom: 14px;
  }

  .help-btn {
    font-size: 11px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 1px solid var(--border);
    color: var(--text-tertiary);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all .15s;
  }
  .help-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  /* ── Config ─────────────────────────── */
  .config-card {
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 10px 14px;
    margin-bottom: 14px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .config-line {
    font-size: 12px;
    color: var(--text-secondary);
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .config-tag {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 3px;
    font-weight: 600;
  }
  .config-tag.del { background: var(--error-light); color: var(--error); }
  .config-tag.weak { background: var(--warning-light); color: var(--warning); }
  .config-tag.keep { background: var(--success-light); color: var(--success); }

  /* ── Stats Row ─────────────────────── */
  .stats-row {
    display: flex;
    gap: 24px;
  }

  .stat-block {
    flex: 1;
    min-width: 0;
  }

  .stat-label {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-bottom: 4px;
  }

  .stat-value {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .stat-sub {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: 2px;
  }

  .stat-metrics {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .metric {
    font-size: 13px;
    color: var(--text-secondary);
  }

  /* ── Gauge ──────────────────────────── */
  .ag-value {
    font-size: 28px;
    font-weight: 700;
    margin-bottom: 4px;
  }

  .gauge-track {
    height: 8px;
    background: var(--bg-tertiary);
    border-radius: 4px;
    overflow: hidden;
  }

  .gauge-fill {
    height: 100%;
    border-radius: 4px;
    transition: width 0.4s ease;
  }

  .gauge-scale {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    color: var(--text-tertiary);
    margin-top: 3px;
  }

  /* ── Badge ──────────────────────────── */
  .badge {
    display: inline-block;
    padding: 2px 10px;
    border-radius: 12px;
    font-size: 13px;
    font-weight: 600;
  }
  .badge.stable {
    background: var(--success-light);
    color: var(--success);
  }
  .badge.unstable {
    background: var(--warning-light);
    color: var(--warning);
  }

  /* ── Entropy Bars ───────────────────── */
  .entropy-bars {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .entropy-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .entropy-label {
    width: 50px;
    font-size: 12px;
    color: var(--text-secondary);
    flex-shrink: 0;
  }

  .entropy-track {
    flex: 1;
    height: 10px;
    background: var(--bg-tertiary);
    border-radius: 5px;
    overflow: hidden;
  }

  .entropy-fill {
    height: 100%;
    border-radius: 5px;
    transition: width 0.4s ease;
  }

  .entropy-val {
    width: 36px;
    font-size: 12px;
    font-weight: 600;
    text-align: right;
    flex-shrink: 0;
  }

  .total-row {
    border-top: 1px dashed var(--border);
    padding-top: 8px;
    margin-top: 4px;
  }

  .threshold-mark {
    font-size: 10px;
    color: var(--text-tertiary);
    margin-left: 4px;
    flex-shrink: 0;
  }

  /* ── History Table ──────────────────── */
  .history-table-wrap {
    max-height: 400px;
    overflow-y: auto;
  }

  .history-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .history-table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }

  .history-table td {
    padding: 7px 8px;
    border-bottom: 1px solid var(--border);
    color: var(--text-secondary);
  }

  .history-table .num { text-align: right; font-variant-numeric: tabular-nums; }
  .history-table th.num { text-align: right; }

  .time-cell { color: var(--text-tertiary); font-size: 12px; }

  .dissolved { color: var(--error); font-weight: 600; }
  .weakened { color: var(--warning); }

  .delta-cell { font-variant-numeric: tabular-nums; }
  .delta { font-size: 12px; }
  .big-drop { font-weight: 700; color: var(--success); }

  .toggle-cell { text-align: right; }
  .toggle-btn {
    font-size: 11px;
    color: var(--text-tertiary);
    padding: 2px 6px;
    border-radius: 3px;
  }
  .toggle-btn:hover { color: var(--accent); background: var(--accent-light); }

  /* ── Expanded Row ──────────────────── */
  .expanded-row td {
    padding: 0;
    border-bottom: 2px solid var(--border);
  }

  .expanded-body {
    padding: 10px 18px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    background: var(--bg-secondary);
  }

  .expanded-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .expanded-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
  }

  /* ── Buckets ───────────────────────── */
  .bucket-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .bucket-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .bucket-label {
    width: 100px;
    font-size: 11px;
    color: var(--text-secondary);
    flex-shrink: 0;
    text-align: right;
  }

  .bucket-track {
    flex: 1;
    height: 8px;
    background: var(--bg-tertiary);
    border-radius: 4px;
    overflow: hidden;
  }

  .bucket-fill {
    height: 100%;
    border-radius: 4px;
    transition: width 0.3s ease;
  }
  .del-fill { background: var(--error); }
  .weak-fill { background: var(--warning); }
  .keep-fill { background: var(--success); }
  .prot-fill { background: var(--text-tertiary); }

  .bucket-count {
    width: 28px;
    font-size: 12px;
    font-weight: 600;
    text-align: right;
    flex-shrink: 0;
    color: var(--text-primary);
  }

  /* ── Samples ────────────────────────── */
  .sample-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .sample-item {
    font-size: 11px;
    color: var(--text-secondary);
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 3px 8px;
  }

  .sample-nature { font-weight: 600; }
  .sample-sep { color: var(--text-tertiary); margin: 0 2px; }
  .sample-source { color: var(--text-secondary); }
  .sample-dim { color: var(--text-tertiary); }
</style>
