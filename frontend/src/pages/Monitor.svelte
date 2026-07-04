<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { fetchMonitor, fetchActiveSessions } from '../lib/api';
  import type { MonitorData, ActiveSession } from '../lib/api';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let data = $state<MonitorData | null>(null);
  let activeSessions = $state<ActiveSession[]>([]);
  let loading = $state(true);

  // Chart ring buffer: last 60 snapshots (5 min at 5s interval)
  const MAX_POINTS = 60;
  let chartPoints = $state<{ turns: number; toolCalls: number }[]>([]);
  let prevTurns = 0;
  let prevToolCalls = 0;
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let refreshTimer: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    refresh();
    refreshTimer = setInterval(refresh, 5000);
  });

  onDestroy(() => {
    if (refreshTimer) clearInterval(refreshTimer);
  });

  async function refresh() {
    try {
      const [monitor, sessions] = await Promise.all([
        fetchMonitor(),
        fetchActiveSessions(),
      ]);
      data = monitor;
      activeSessions = sessions.sessions;

      // Chart data
      const turns = num(monitor.metrics.jia_turns_total);
      const tc = total(obj(monitor.metrics.jia_tool_calls_total));
      if (prevTurns > 0 || prevToolCalls > 0) {
        chartPoints = [...chartPoints.slice(-(MAX_POINTS - 1)), { turns: turns - prevTurns, toolCalls: tc - prevToolCalls }];
      }
      prevTurns = turns;
      prevToolCalls = tc;
    } catch {
      // keep stale data
    }
    loading = false;
  }

  function num(v: unknown): number {
    if (typeof v === 'number') return v;
    return 0;
  }

  function obj(v: unknown): Record<string, number> | null {
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      const out: Record<string, number> = {};
      for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
        if (typeof val === 'number') out[k] = val;
      }
      return out;
    }
    return null;
  }

  function total(map: Record<string, number> | null): number {
    if (!map) return 0;
    return Object.values(map).reduce((s, v) => s + v, 0);
  }

  let metrics = $derived(data?.metrics ?? {});
  let turnsTotal = $derived(num(metrics.jia_turns_total));
  let toolCalls = $derived(obj(metrics.jia_tool_calls_total));
  let errors = $derived(obj(metrics.jia_errors_total));
  let gejuEvals = $derived(obj(metrics.jia_geju_evals_total));
  let sessionCount = $derived(data?.active_sessions ?? 0);
  let ctxMax = $derived(data?.context_window?.max_tokens ?? 8192);
  let seedsTotal = $derived(num(metrics.jia_seeds_total));
  let atmaGraha = $derived(num(metrics.jia_atma_graha));
  let llmInputTokens = $derived(num(metrics.jia_llm_input_tokens_total));
  let llmOutputTokens = $derived(num(metrics.jia_llm_output_tokens_total));
  let eventbusDrops = $derived(num(metrics.jia_eventbus_drops_total));
  let tokensCompacted = $derived(num(metrics.jia_tokens_compacted_total));
  let requestsTotal = $derived(obj(metrics.jia_requests_total));
  let sessionsCompleted = $derived(num(metrics.jia_sessions_completed_total));

  function atmaGrahaColor(v: number): string {
    if (v >= 0.6) return 'var(--accent)';
    if (v >= 0.3) return '#f59e0b';
    return '#ef4444';
  }

  function formatTs(ts: number): string {
    const d = new Date(ts * 1000);
    return d.toLocaleTimeString();
  }

  // Chart drawing
  $effect(() => {
    const canvas = canvasEl;
    const points = chartPoints;
    if (!canvas || points.length < 2) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const W = rect.width;
    const H = rect.height;
    ctx.clearRect(0, 0, W, H);

    const maxTurns = Math.max(...points.map(p => p.turns), 1);
    const maxTC = Math.max(...points.map(p => p.toolCalls), 1);
    const yMax = Math.max(maxTurns, maxTC) * 1.2;

    function drawLine(g: CanvasRenderingContext2D, vals: number[], color: string) {
      g.beginPath();
      g.strokeStyle = color;
      g.lineWidth = 2;
      for (let i = 0; i < vals.length; i++) {
        const x = (i / (vals.length - 1)) * (W - 40) + 20;
        const y = H - 20 - (vals[i] / yMax) * (H - 40);
        if (i === 0) g.moveTo(x, y); else g.lineTo(x, y);
      }
      g.stroke();
    }

    drawLine(ctx, points.map(p => p.turns), '#6366f1');
    drawLine(ctx, points.map(p => p.toolCalls), '#10b981');

    // Legend
    ctx.font = '11px sans-serif';
    ctx.fillStyle = '#6366f1';
    ctx.fillRect(20, 8, 10, 10);
    ctx.fillStyle = '#888';
    ctx.fillText('Turns/5s', 34, 17);
    ctx.fillStyle = '#10b981';
    ctx.fillRect(100, 8, 10, 10);
    ctx.fillStyle = '#888';
    ctx.fillText('Tool calls/5s', 114, 17);
  });
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('monitor.title')}</h2>
  </div>
  <div class="body">
    {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else if !data}
      <p class="msg">{t('common.noData')}</p>
    {:else}
      <!-- Summary cards -->
      <div class="grid">
        <div class="card">
          <div class="card-label">{t('monitor.activeSessions')}</div>
          <div class="card-value big">{sessionCount}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.turns')}</div>
          <div class="card-value big">{turnsTotal}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.seedsTotal')}</div>
          <div class="card-value big">{seedsTotal}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.atmaGraha')}</div>
          <div class="card-value big" style="color: {atmaGrahaColor(atmaGraha)}">{(atmaGraha * 100).toFixed(0)}%</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.llmInputTokens')}</div>
          <div class="card-value">{llmInputTokens.toLocaleString()}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.llmOutputTokens')}</div>
          <div class="card-value">{llmOutputTokens.toLocaleString()}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.eventbusDrops')}</div>
          <div class="card-value" class:warn={eventbusDrops > 0}>{eventbusDrops}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.tokensCompacted')}</div>
          <div class="card-value">{tokensCompacted.toLocaleString()}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.sessionsCompleted')}</div>
          <div class="card-value big">{sessionsCompleted}</div>
        </div>
        <div class="card">
          <div class="card-label">{t('monitor.requestsTotal')}</div>
          <div class="card-value big">{total(requestsTotal)}</div>
        </div>
      </div>

      <!-- Chart -->
      {#if chartPoints.length >= 2}
        <div class="section">
          <h3 class="section-title">{t('monitor.chartTurns')}</h3>
          <div class="chart-wrap">
            <canvas bind:this={canvasEl} class="chart"></canvas>
          </div>
        </div>
      {/if}

      <!-- Active sessions table -->
      <div class="section">
        <h3 class="section-title">{t('monitor.sessionList')} ({activeSessions.length})</h3>
        {#if activeSessions.length > 0}
          <table class="table">
            <thead><tr>
              <th>{t('monitor.sessionId')}</th>
              <th>{t('monitor.sessionProvider')}</th>
              <th>{t('monitor.sessionModel')}</th>
              <th>{t('monitor.sessionCreated')}</th>
            </tr></thead>
            <tbody>
              {#each activeSessions as s}
                <tr>
                  <td class="mono">{s.id.slice(0, 8)}...</td>
                  <td>{s.provider}</td>
                  <td>{s.model}</td>
                  <td>{formatTs(s.created_at)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="empty">{t('common.noData')}</p>
        {/if}
      </div>

      <!-- Requests by model -->
      {#if requestsTotal && total(requestsTotal) > 0}
        <div class="section">
          <h3 class="section-title">{t('monitor.requestsByModel')}</h3>
          <table class="table">
            <thead><tr><th>Model</th><th class="num">Count</th></tr></thead>
            <tbody>
              {#each Object.entries(requestsTotal).sort(([,a], [,b]) => b - a) as [model, count]}
                <tr><td>{model}</td><td class="num">{count}</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <!-- Tool Calls table -->
      <div class="section">
        <h3 class="section-title">{t('monitor.toolCalls')}</h3>
        {#if toolCalls}
          <table class="table">
            <thead><tr><th>Tool</th><th class="num">Count</th></tr></thead>
            <tbody>
              {#each Object.entries(toolCalls).sort(([,a], [,b]) => b - a) as [name, count]}
                <tr><td>{name}</td><td class="num">{count}</td></tr>
              {/each}
              <tr class="total"><td>Total</td><td class="num">{total(toolCalls)}</td></tr>
            </tbody>
          </table>
        {:else}
          <p class="empty">{t('common.noData')}</p>
        {/if}
      </div>

      <!-- Errors -->
      <div class="section">
        <h3 class="section-title">{t('monitor.errors')}</h3>
        {#if errors}
          <table class="table">
            <thead><tr><th>Source</th><th class="num">Count</th></tr></thead>
            <tbody>
              {#each Object.entries(errors).sort(([,a], [,b]) => b - a) as [source, count]}
                <tr><td>{source}</td><td class="num">{count}</td></tr>
              {/each}
            </tbody>
          </table>
        {:else}
          <p class="empty">{t('common.noData')}</p>
        {/if}
      </div>

      <!-- GeJu Evals -->
      <div class="section">
        <h3 class="section-title">{t('monitor.gejuEvals')}</h3>
        {#if gejuEvals}
          <table class="table">
            <thead><tr><th>Mode</th><th class="num">Count</th></tr></thead>
            <tbody>
              {#each Object.entries(gejuEvals).sort(([,a], [,b]) => b - a) as [mode, count]}
                <tr><td>{mode}</td><td class="num">{count}</td></tr>
              {/each}
              <tr class="total"><td>Total</td><td class="num">{total(gejuEvals)}</td></tr>
            </tbody>
          </table>
        {:else}
          <p class="empty">{t('common.noData')}</p>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 20px; border-bottom: 1px solid var(--border);
  }
  .title { font-size: 16px; font-weight: 600; }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }

  .grid { display: grid; grid-template-columns: repeat(5, 1fr); gap: 10px; margin-bottom: 20px; }
  .card {
    border: 1px solid var(--border); border-radius: var(--radius-md);
    padding: 12px; text-align: center;
  }
  .card-label { font-size: 11px; color: var(--text-tertiary); margin-bottom: 4px; text-transform: uppercase; letter-spacing: .3px; }
  .card-value { font-size: 16px; font-weight: 600; color: var(--text-primary); }
  .card-value.big { font-size: 24px; color: var(--accent); }
  .card-value.warn { color: #ef4444; }

  .chart-wrap { height: 160px; border: 1px solid var(--border); border-radius: var(--radius-md); overflow: hidden; }
  .chart { width: 100%; height: 100%; display: block; }

  .section { margin-bottom: 24px; }
  .section-title { font-size: 14px; font-weight: 600; margin-bottom: 8px; }

  .table { width: 100%; border-collapse: collapse; font-size: 13px; }
  .table th { text-align: left; font-weight: 500; color: var(--text-tertiary); padding: 4px 8px; border-bottom: 1px solid var(--border); }
  .table td { padding: 4px 8px; border-bottom: 1px solid var(--border); color: var(--text-secondary); }
  .table .num { text-align: right; }
  .table .total { font-weight: 600; color: var(--text-primary); }
  .table .total td { border-bottom: none; padding-top: 8px; }
  .table .mono { font-family: var(--font-mono, monospace); font-size: 12px; }

  .empty { text-align: center; color: var(--text-tertiary); padding: 16px; font-size: 13px; }
</style>
