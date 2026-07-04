<script lang="ts">
  import type { ChatEntry } from '../lib/types';
  import MessageBubble from './MessageBubble.svelte';
  import ToolCard from './ToolCard.svelte';

  interface Group {
    role: 'user' | 'assistant' | 'tool';
    entries: ChatEntry[];
  }

  let { group, isLast }: { group: Group; isLast: boolean } = $props();
</script>

<div class="group" class:user={group.role === 'user'} class:assistant={group.role === 'assistant'} class:tool={group.role === 'tool'}>
  <div class="content-col">
    <div class="bubbles">
      {#each group.entries as entry}
        {#if entry.role === 'tool_call'}
          <ToolCard entry={entry} />
        {:else}
          <MessageBubble {entry} />
        {/if}
      {/each}
    </div>
  </div>
</div>

<style>
  .group {
    display: flex;
    margin-bottom: 16px;
  }

  .group.user {
    align-self: flex-end;
    max-width: 85%;
  }

  .group.assistant {
    align-self: flex-start;
  }

  .group.tool {
    align-self: flex-start;
    max-width: 85%;
  }

  .content-col {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .bubbles {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
