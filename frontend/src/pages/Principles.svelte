<script lang="ts">
  import { onMount } from 'svelte';
  import { API_BASE, tokenReady, TOKEN } from '../lib/api';
  import { t } from '../lib/i18n';

  interface Principle {
    id: string;
    session_id: string;
    geju_key: string;
    constraint: { type: string; mode?: string; reason?: string; gate?: string };
    confidence: number;
    source_seed_count: number;
    archived?: boolean;
  }

  let principles = $state<Principle[]>([]);
  let loading = $state(true);
  let error = $state('');

  onMount(async () => {
    await tokenReady;
    try {
      const res = await fetch(`${API_BASE}/principles`, {
        headers: { Authorization: `Bearer ${TOKEN}` },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      principles = data.principles ?? [];
    } catch (e: any) {
      error = e.message || 'Failed to load principles';
    } finally {
      loading = false;
    }
  });

  async function toggleArchive(p: Principle) {
    const action = p.archived ? 'unarchive' : 'archive';
    try {
      const res = await fetch(`${API_BASE}/principles/${p.id}/${action}`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${TOKEN}` },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      p.archived = !p.archived;
    } catch (e: any) {
      console.error(`Failed to ${action} principle:`, e);
    }
  }

  function constraintLabel(c: Principle['constraint']): string {
    switch (c.type) {
      case 'EscalateTo': return `⬆ ${c.mode ?? '?'}`;
      case 'AddGuard':       return `🛡 ${c.gate ?? c.reason ?? '?'}`;
      case 'RequireAudit':   return `📋 ${c.reason ?? 'audit'}`;
      default: return c.type;
    }
  }

  const active = $derived(principles.filter(p => !p.archived));
  const archived = $derived(principles.filter(p => p.archived));
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('principles.title')}</h2>
    <p class="subtitle">{t('principles.subtitle')}</p>
  </div>

  <div class="body">
    {#if loading}
      <p class="empty">{t('common.loading')}</p>
    {:else if error}
      <p class="empty" style="color:var(--danger,#e74c3c)">{error}</p>
    {:else if principles.length === 0}
      <p class="empty">{t('principles.empty')}</p>
    {:else}
      <div class="section">
        <h3 class="section-title">{t('principles.active')} ({active.length})</h3>
        {#if active.length === 0}
          <p class="empty">{t('principles.noActive')}</p>
        {:else}
          <table class="table">
            <thead>
              <tr>
                <th>{t('principles.gejuKey')}</th>
                <th>{t('principles.constraint')}</th>
                <th class="num">{t('principles.confidence')}</th>
                <th class="num">{t('principles.seeds')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each active as p (p.id)}
                <tr>
                  <td class="mono">{p.geju_key}</td>
                  <td>{constraintLabel(p.constraint)}</td>
                  <td class="num">{(p.confidence * 100).toFixed(0)}%</td>
                  <td class="num">{p.source_seed_count}</td>
                  <td>
                    <button class="btn-sm" onclick={() => toggleArchive(p)}>
                      {t('principles.archive')}
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>

      {#if archived.length > 0}
        <div class="section">
          <h3 class="section-title">{t('principles.archived')} ({archived.length})</h3>
          <table class="table">
            <thead>
              <tr>
                <th>{t('principles.gejuKey')}</th>
                <th>{t('principles.constraint')}</th>
                <th class="num">{t('principles.confidence')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each archived as p (p.id)}
                <tr class="archived-row">
                  <td class="mono">{p.geju_key}</td>
                  <td>{constraintLabel(p.constraint)}</td>
                  <td class="num">{(p.confidence * 100).toFixed(0)}%</td>
                  <td>
                    <button class="btn-sm" onclick={() => toggleArchive(p)}>
                      {t('principles.restore')}
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header { display: flex; align-items: center; justify-content: space-between; padding: 12px 20px; border-bottom: 1px solid var(--border); }
  .title { font-size: 16px; font-weight: 600; }
  .subtitle { font-size: 12px; color: var(--text-tertiary); margin-top: 4px; }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
  .section { margin-bottom: 24px; }
  .section-title { font-size: 14px; font-weight: 600; margin-bottom: 8px; }
  .table { width: 100%; border-collapse: collapse; font-size: 13px; }
  .table th { text-align: left; font-weight: 500; color: var(--text-tertiary); padding: 4px 8px; border-bottom: 1px solid var(--border); }
  .table td { padding: 4px 8px; border-bottom: 1px solid var(--border); color: var(--text-secondary); }
  .table .num { text-align: right; }
  .table .mono { font-family: var(--font-mono, monospace); font-size: 12px; }
  .empty { text-align: center; color: var(--text-tertiary); padding: 16px; font-size: 13px; }
  .archived-row { opacity: 0.5; }
  .btn-sm { font-size: 12px; padding: 2px 10px; border-radius: 4px; border: 1px solid var(--border); background: transparent; color: var(--text-secondary); cursor: pointer; }
  .btn-sm:hover { background: var(--bg-tertiary); }
</style>
