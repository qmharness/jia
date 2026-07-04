<script lang="ts">
  import ShimmerLogo from './ShimmerLogo.svelte';
  import { store, getSessionProvider, getSessionModel } from '../lib/store.svelte';
  import { sendMessage } from '../lib/sse';
  import type { StreamAgentParams } from '../lib/types';
  import { t } from '../lib/i18n';

  const suggestionKeys = [
    'chat.emptySugg1',
    'chat.emptySugg2',
    'chat.emptySugg3',
    'chat.emptySugg4',
  ];

  function onClick(text: string) {
    const params: StreamAgentParams = {
      provider: getSessionProvider(),
      model: getSessionModel() || undefined,
      messages: [{ role: 'user', content: text }],
      sessionId: store.activeSessionId,
    };
    store.entries = [...store.entries, { id: crypto.randomUUID(), role: 'user', content: text, createdAt: Date.now() }];
    sendMessage(params);
  }
</script>

<div class="flex-1 flex flex-col items-center justify-center gap-3 p-8 text-center">
  <ShimmerLogo />
  <h3 class="text-xl font-bold tracking-[-0.3px] animate-slide-up" style="color: var(--text-primary)">{t('chat.emptyTitle')}</h3>
  <p class="text-[13px] max-w-[380px] animate-slide-up" style="color: var(--text-secondary)">{t('chat.emptySub')}</p>
  <div class="flex flex-wrap gap-2 justify-center mt-3 animate-slide-up">
    {#each suggestionKeys as key}
      <button class="px-4 py-[7px] border rounded-[20px] text-[12.5px] transition-colors duration-150"
              style="color: var(--text-secondary); border-color: var(--border); background: var(--bg-primary)"
              onmouseenter={(e) => {(e.target as HTMLElement).style.borderColor = 'var(--accent)'; (e.target as HTMLElement).style.color = 'var(--accent)'; (e.target as HTMLElement).style.background = 'var(--accent-light)'}}
              onmouseleave={(e) => {(e.target as HTMLElement).style.borderColor = 'var(--border)'; (e.target as HTMLElement).style.color = 'var(--text-secondary)'; (e.target as HTMLElement).style.background = 'var(--bg-primary)'}}
              onclick={() => onClick(t(key))}>{t(key)}</button>
    {/each}
  </div>
</div>
