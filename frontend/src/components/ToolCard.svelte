<script lang="ts">
  import type { ToolCallEntry } from '../lib/types';
  import { t } from '../lib/i18n';

  let { entry }: { entry: ToolCallEntry } = $props();
  let collapsed = $state(true);
  let bodyEl = $state<HTMLDivElement>();

  const statusColor = $derived(
    entry.status === 'running' ? 'var(--accent)' :
    entry.status === 'success' ? 'var(--success)' :
    'var(--error)'
  );

  const statusLabel = $derived(
    entry.status === 'running' ? t('toolcard.statusRunning') :
    entry.status === 'success' ? t('toolcard.statusSuccess') :
    t('toolcard.statusError')
  );

  const modeLabel = $derived(entry.executionMode || 'direct');
  const modeText = $derived(
    modeLabel === 'direct' ? t('toolcard.modeDirect') :
    modeLabel === 'guarded' ? t('toolcard.modeGuarded') :
    modeLabel === 'sandbox' ? t('toolcard.modeSandbox') :
    t('toolcard.modeDenied')
  );

  async function highlightElement(el: HTMLElement) {
    const hljs = (await import('highlight.js')).default;
    el.querySelectorAll('pre code').forEach((code) => {
      hljs.highlightElement(code as HTMLElement);
    });
  }

  $effect(() => {
    if (!collapsed && bodyEl) {
      requestAnimationFrame(() => highlightElement(bodyEl!));
    }
  });
</script>

<div class="tool-card" style="--status-color: {statusColor}">
  <button class="tool-header" onclick={() => collapsed = !collapsed}>
    <span class="tool-name">{entry.tool}</span>
    {#if entry.geju}
      <span class="tool-header-meta">{entry.geju}</span>
    {/if}
    <span class="tool-header-meta">{modeText}</span>
    <span class="tool-status" class:running={entry.status === 'running'} class:success={entry.status === 'success'} class:error={entry.status === 'error'}>
      {statusLabel}
    </span>
    <span class="collapse-icon">{collapsed ? '▸' : '▾'}</span>
  </button>

  {#if !collapsed}
    <div class="tool-body" bind:this={bodyEl}>
      <div class="tool-section">
        <span class="tool-label">{t('toolcard.input')}</span>
        <pre class="tool-pre"><code class="language-json">{JSON.stringify(entry.input, null, 2)}</code></pre>
      </div>
      {#if entry.output}
        <div class="tool-section">
          <span class="tool-label">{t('toolcard.output')}</span>
          <pre class="tool-pre"><code>{entry.output}</code></pre>
        </div>
      {/if}
      {#if entry.error}
        <div class="tool-section">
          <span class="tool-label">{t('toolcard.error')}</span>
          <pre class="tool-pre error-pre"><code>{entry.error}</code></pre>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .tool-card {
    border: 1px solid var(--border);
    border-left: 3px solid var(--status-color);
    border-radius: var(--radius-md);
    background: var(--bg-primary);
    overflow: hidden;
    margin: 4px 0;
  }

  .tool-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    width: 100%;
    font-size: 13px;
    color: var(--text-primary);
    transition: background .15s;
  }

  .tool-header:hover {
    background: var(--bg-secondary);
  }

  .tool-name {
    font-weight: 600;
    flex: 1;
    text-align: left;
  }

  .tool-status {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 10px;
    font-weight: 600;
    text-transform: uppercase;
  }

  .tool-status.running { background: var(--accent-light); color: var(--accent); }
  .tool-status.success { background: var(--success-light); color: var(--success); }
  .tool-status.error { background: var(--error-light); color: var(--error); }

  .tool-header-meta {
    font-size: 10px;
    color: var(--text-tertiary);
    font-weight: 400;
  }

  .collapse-icon {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .tool-body {
    padding: 0 12px 10px;
    border-top: 1px solid var(--border);
  }

  .tool-section {
    margin-top: 8px;
  }

  .tool-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    text-transform: uppercase;
    letter-spacing: .5px;
  }

  .tool-pre {
    background: var(--bg-secondary);
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-family: 'SF Mono', 'Fira Code', monospace;
    overflow-x: auto;
    margin-top: 4px;
    max-height: 200px;
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .error-pre {
    color: var(--error);
    background: var(--error-light);
  }
</style>
