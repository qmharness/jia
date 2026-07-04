<script lang="ts">
  import { store, dismissToast } from '../lib/store.svelte';

  let timeout: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (store.toast) {
      if (timeout) clearTimeout(timeout);
      timeout = setTimeout(() => { dismissToast(); }, 3500);
    }
    return () => { if (timeout) clearTimeout(timeout); };
  });
</script>

{#if store.toast}
  <div class="toast" class:error={store.toast.type === 'error'} class:success={store.toast.type === 'success'}>
    <span class="toast-msg">{store.toast.message}</span>
    <button class="toast-close" onclick={dismissToast} aria-label="Close">×</button>
  </div>
{/if}

<style>
  .toast {
    position: fixed; bottom: 24px; right: 24px;
    display: flex; align-items: center; gap: 10px;
    padding: 10px 16px;
    background: var(--text-primary); color: #fff;
    border-radius: var(--radius-md);
    box-shadow: 0 8px 24px rgba(0,0,0,.15);
    font-size: 14px; z-index: 2000;
    max-width: 400px;
    animation: slideUp .25s ease-out;
  }
  .toast.error { background: var(--error); }
  .toast.success { background: var(--success); }
  .toast-msg { flex: 1; }
  .toast-close {
    color: rgba(255,255,255,.7); font-size: 18px; line-height: 1;
    padding: 0 2px;
  }
  .toast-close:hover { color: #fff; }

  @keyframes slideUp {
    from { transform: translateY(20px); opacity: 0; }
    to { transform: translateY(0); opacity: 1; }
  }
</style>
