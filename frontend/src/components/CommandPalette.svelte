<script lang="ts">
  import { setPage } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let { open = $bindable(false) }: { open: boolean } = $props();
  let search = $state('');
  let selectedIdx = $state(0);

  interface Command {
    id: string;
    label: string;
    icon: string;
    action: () => void;
  }

  const commands: Command[] = [
    { id: 'chat',    icon: '💬', label: t('nav.chat'),    action: () => setPage('chat') },
    { id: 'sessions',icon: '📋', label: t('nav.sessions'),action: () => setPage('sessions') },
    { id: 'tools',   icon: '🔧', label: t('nav.tools'),   action: () => setPage('tools') },
    { id: 'skills',  icon: '⚡', label: t('nav.skills'),  action: () => setPage('skills') },
    { id: 'cron',    icon: '⏰', label: t('nav.cron'),    action: () => setPage('cron') },
    { id: 'monitor', icon: '📊', label: t('nav.monitor'), action: () => setPage('monitor') },
    { id: 'vijnana', icon: '🧠', label: t('nav.vijnana'), action: () => setPage('vijnana') },
    { id: 'settings',icon: '⚙',  label: t('nav.settings'),action: () => setPage('settings') },
  ];

  const filtered = $derived(
    search
      ? commands.filter(c => c.label.toLowerCase().includes(search.toLowerCase()))
      : commands
  );

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIdx = Math.min(selectedIdx + 1, filtered.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIdx = Math.max(selectedIdx - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      filtered[selectedIdx]?.action();
      open = false;
    } else if (e.key === 'Escape') {
      open = false;
    }
  }

  $effect(() => { void search; selectedIdx = 0; });
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="fixed inset-0 z-[1000] flex items-start justify-center pt-[20vh] bg-black/30"
    onclick={() => open = false}
    onkeydown={onKeydown}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="w-[480px] max-w-[90vw] rounded-xl shadow-lg border overflow-hidden animate-scale-in"
      style="background: var(--bg-primary); border-color: var(--border)"
      onclick={(e: MouseEvent) => e.stopPropagation()}
      onkeydown={(e: KeyboardEvent) => e.stopPropagation()}
    >
      <!-- svelte-ignore a11y_autofocus -->
      <input
        type="text"
        class="w-full px-5 py-4 text-[15px] bg-transparent border-b outline-none placeholder:opacity-50"
        style="border-color: var(--border); color: var(--text-primary)"
        placeholder="Type a command..."
        bind:value={search}
        autofocus
      />
      <div class="max-h-[320px] overflow-y-auto p-2">
        {#each filtered as cmd, i}
          <button
            class="w-full flex items-center gap-3 px-3 py-2.5 text-[13px] rounded-md text-left transition-colors duration-75"
            style={i === selectedIdx
              ? 'background: var(--accent-light); color: var(--accent)'
              : 'color: var(--text-primary)'}
            onmouseenter={() => selectedIdx = i}
            onclick={() => { cmd.action(); open = false; }}
          >
            <span class="text-base">{cmd.icon}</span>
            <span>{cmd.label}</span>
          </button>
        {/each}
        {#if filtered.length === 0}
          <p class="text-center py-8 text-[13px]" style="color: var(--text-tertiary)">No results</p>
        {/if}
      </div>
      <div class="px-5 py-2 border-t flex gap-4 text-[11px]"
           style="border-color: var(--border); color: var(--text-tertiary)">
        <span>↑↓ Navigate</span><span>↵ Select</span><span>Esc Dismiss</span>
      </div>
    </div>
  </div>
{/if}
