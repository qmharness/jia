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
      case 'AddGuard': return `🛡 ${c.gate ?? c.reason ?? '?'}`;
      case 'RequireAudit': return `📋 ${c.reason ?? 'audit'}`;
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
      <p class="muted">{t('common.loading')}</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else if principles.length === 0}
      <p class="muted">{t('principles.empty')}</p>
    {:else}
      <section>
        <h3>{t('principles.active')} ({active.length})</h3>
        {#if active.length === 0}
          <p class="muted">{t('principles.noActive')}</p>
        {:else}
          <table class="table">
            <thead>
              <tr>
                <th>{t('principles.gejuKey')}</th>
                <th>{t('principles.constraint')}</th>
                <th>{t('principles.confidence')}</th>
                <th>{t('principles.seeds')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each active as p (p.id)}
                <tr>
                  <td><code>{p.geju_key}</code></td>
                  <td>{constraintLabel(p.constraint)}</td>
                  <td>{(p.confidence * 100).toFixed(0)}%</td>
                  <td>{p.source_seed_count}</td>
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
      </section>

      {#if archived.length > 0}
        <section style="margin-top: 2rem;">
          <h3>{t('principles.archived')} ({archived.length})</h3>
          <table class="table">
            <thead>
              <tr>
                <th>{t('principles.gejuKey')}</th>
                <th>{t('principles.constraint')}</th>
                <th>{t('principles.confidence')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each archived as p (p.id)}
                <tr class="archived-row">
                  <td><code>{p.geju_key}</code></td>
                  <td>{constraintLabel(p.constraint)}</td>
                  <td>{(p.confidence * 100).toFixed(0)}%</td>
                  <td>
                    <button class="btn-sm" onclick={() => toggleArchive(p)}>
                      {t('principles.restore')}
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </section>
      {/if}
    {/if}
  </div>
</div>

<style>
  .table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .table th, .table td {
    text-align: left;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .table th {
    color: var(--text-tertiary);
    font-weight: 500;
  }
  .archived-row {
    opacity: 0.55;
  }
  .btn-sm {
    font-size: 12px;
    padding: 2px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }
  .btn-sm:hover {
    background: var(--bg-tertiary);
  }
  .muted {
    color: var(--text-tertiary);
    font-size: 13px;
  }
  .error {
    color: #e74c3c;
    font-size: 13px;
  }
</style>
