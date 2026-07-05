<script lang="ts">
  import VijnanaStatus from '../components/VijnanaStatus.svelte';
  import VijnanaSeeds from '../components/VijnanaSeeds.svelte';
  import SpiritObserver from '../components/SpiritObserver.svelte';
  import { t } from '../lib/i18n';

  let tab = $state<'status' | 'seeds' | 'history' | 'spirits'>('status');
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('vijnana.title')}</h2>
  </div>
  <nav class="tab-bar">
    <button class="tab-btn" class:active={tab === 'status'}  onclick={() => tab = 'status'}>{t('vijnana.tabStatus')}</button>
    <button class="tab-btn" class:active={tab === 'seeds'}   onclick={() => tab = 'seeds'}>{t('vijnana.tabSeeds')}</button>
    <button class="tab-btn" class:active={tab === 'history'} onclick={() => tab = 'history'}>{t('vijnana.tabHistory')}</button>
    <button class="tab-btn" class:active={tab === 'spirits'} onclick={() => tab = 'spirits'}>{t('vijnana.tabSpirits')}</button>
  </nav>
  <div class="body">
    {#if tab === 'status'}
      <VijnanaStatus hideHistory />
    {:else if tab === 'seeds'}
      <VijnanaSeeds />
    {:else if tab === 'spirits'}
      <SpiritObserver />
    {:else}
      <VijnanaStatus statusOnly />
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header { padding: 12px 20px; border-bottom: 1px solid var(--border); }
  .title { font-size: 16px; font-weight: 600; }

  .tab-bar {
    display: flex; gap: 0;
    padding: 0 20px; border-bottom: 1px solid var(--border);
  }
  .tab-btn {
    padding: 10px 16px; font-size: 13px; color: var(--text-secondary);
    border-bottom: 2px solid transparent; transition: color .15s, border-color .15s;
  }
  .tab-btn:hover { color: var(--text-primary); }
  .tab-btn.active {
    color: var(--accent); border-bottom-color: var(--accent); font-weight: 600;
  }

  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
</style>
