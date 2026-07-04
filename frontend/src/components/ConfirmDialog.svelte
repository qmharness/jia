<script lang="ts">
  import { store, showToast } from '../lib/store.svelte';
  import { confirmAction } from '../lib/api';
  import { t } from '../lib/i18n';

  let timer = $state(0);
  let interval: ReturnType<typeof setInterval> | null = null;

  $effect(() => {
    if (store.confirmState) {
      timer = store.confirmState.timeoutSecs;
      interval = setInterval(() => {
        timer--;
        if (timer <= 0 && store.confirmState) {
          handleResponse(false);
        }
      }, 1000);
    } else {
      if (interval) { clearInterval(interval); interval = null; }
    }
    return () => { if (interval) clearInterval(interval); };
  });

  async function handleResponse(approved: boolean) {
    if (!store.confirmState) return;
    const state = { ...store.confirmState };
    store.confirmState = null;
    if (interval) { clearInterval(interval); interval = null; }
    try {
      await confirmAction(state.id, state.token, approved);
    } catch {
      showToast(t('confirm.failed'), 'error');
    }
  }
</script>

{#if store.confirmState}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => handleResponse(false)}>
    <div class="dialog" onclick={(e: MouseEvent) => e.stopPropagation()} onkeydown={() => {}}>
      <h3 class="dialog-title">{t('confirm.title')}</h3>
      <div class="dialog-body">
        <div class="info-row">
          <span class="label">{t('confirm.tool')}</span>
          <span class="value">{store.confirmState.tool}</span>
        </div>
        <div class="info-row">
          <span class="label">{t('confirm.reason')}</span>
          <span class="value">{store.confirmState.reason}</span>
        </div>
        <div class="timer">
          {t('confirm.autoDenying', { n: timer })}
        </div>
      </div>
      <div class="dialog-actions">
        <button class="btn-deny" onclick={() => handleResponse(false)}>{t('confirm.deny')}</button>
        <button class="btn-confirm" onclick={() => handleResponse(true)}>{t('confirm.confirm')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed; inset: 0;
    background: rgba(0,0,0,.3);
    display: flex; align-items: center; justify-content: center;
    z-index: 1000;
  }
  .dialog {
    background: var(--bg-primary);
    border-radius: var(--radius-lg);
    box-shadow: 0 20px 60px rgba(0,0,0,.15);
    padding: 24px;
    width: 380px;
    max-width: 90vw;
  }
  .dialog-title { font-size: 16px; font-weight: 700; margin-bottom: 16px; }
  .dialog-body { display: flex; flex-direction: column; gap: 10px; margin-bottom: 20px; }
  .info-row { display: flex; gap: 12px; font-size: 14px; }
  .label { color: var(--text-secondary); min-width: 50px; font-weight: 600; }
  .value { color: var(--text-primary); }
  .timer { font-size: 13px; color: var(--warning); text-align: center; }
  .dialog-actions { display: flex; gap: 10px; justify-content: flex-end; }
  .btn-deny, .btn-confirm {
    padding: 8px 20px; border-radius: var(--radius-sm);
    font-size: 14px; font-weight: 600; transition: all .15s;
  }
  .btn-deny { border: 1px solid var(--border); color: var(--text-secondary); background: var(--bg-secondary); }
  .btn-deny:hover { background: var(--bg-tertiary); }
  .btn-confirm { background: var(--accent); color: #fff; }
  .btn-confirm:hover { background: var(--accent-hover); }
</style>
