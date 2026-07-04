<script lang="ts">
  import type { Message } from '../lib/types';
  import { t } from '../lib/i18n';

  let { entry }: { entry: Message } = $props();

  async function onCopy() {
    try {
      await navigator.clipboard.writeText(entry.content);
    } catch { /* noop */ }
  }
</script>

<div class="actions">
  <button class="action-btn" onclick={onCopy} title={t('chat.copy')}>
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
  </button>
</div>

<style>
  .actions {
    display: flex;
    gap: 2px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 2px;
    box-shadow: var(--shadow-sm);
  }

  .action-btn {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    color: var(--text-secondary);
    transition: all .15s;
  }

  .action-btn:hover {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }
</style>
