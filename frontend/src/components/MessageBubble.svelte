<script lang="ts">
  import { renderMarkdown } from '../lib/markdown';
  import type { StreamingMessage } from '../lib/types';
  import MessageActions from './MessageActions.svelte';
  import { t } from '../lib/i18n';
  import { store } from '../lib/store.svelte';

  let { entry }: { entry: StreamingMessage } = $props();

  let hover = $state(false);

  const streamId = $derived(entry._streamId);
  const isStreaming = $derived(!!streamId);
  const isQueued = $derived(
    streamId !== undefined && store.streamStates[streamId]?.status === 'queued'
  );
  const isFinished = $derived(!isStreaming);

  const html = $derived(isQueued ? '' : renderMarkdown(entry.content || ''));

  function onClick() {
    const streamId = entry._streamId;
    if (streamId && store.cancels[streamId]) {
      store.cancels[streamId]();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="msg"
  class:user={entry.role === 'user'}
  class:assistant={entry.role === 'assistant'}
  class:streaming={isStreaming}
  onmouseenter={() => hover = true}
  onmouseleave={() => hover = false}
  role={isStreaming ? 'button' : undefined}
  tabindex={isStreaming ? 0 : undefined}
  onkeydown={isStreaming ? (e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onClick(); } } : undefined}
>
  <!-- User: bubble style -->
  {#if entry.role === 'user'}
    <div class="user-bubble">
      {#if entry.images?.length}
        <div class="images">
          {#each entry.images as img}
            <img src="data:{img.media_type};base64,{img.data}" alt={t('chat.attachedImage')} class="attached-img" />
          {/each}
        </div>
      {/if}
      {#if entry.content}
        <span class="text">{entry.content}</span>
      {/if}
    </div>
  {/if}

  <!-- Assistant: raw markdown, no background -->
  {#if entry.role === 'assistant'}
    {#if isQueued || (isStreaming && !entry.content)}
      <span class="typing-dots"><span>.</span><span>.</span><span>.</span></span>
    {:else}
      <div class="assistant-content">
        {@html html}
      </div>
    {/if}
  {/if}

  {#if hover && isFinished && entry.role === 'assistant'}
    <div class="actions-wrapper">
      <MessageActions {entry} />
    </div>
  {/if}
</div>

<style>
  .msg {
    position: relative;
    padding: 2px 0;
  }

  .msg.user {
    align-self: flex-end;
  }

  .msg.assistant {
    animation: fadeIn 200ms ease-out;
  }

  /* User bubble — subtle rounded card */
  .user-bubble {
    padding: 10px 18px;
    border-radius: 20px;
    background: var(--bg-secondary);
    color: var(--text-primary);
    font-size: 15px;
    line-height: 1.6;
    word-break: break-word;
    overflow-wrap: break-word;
  }

  /* Assistant content — clean text */
  .assistant-content {
    font-size: 15px;
    line-height: 1.7;
    color: var(--text-primary);
  }

  .assistant-content :global(p) { margin: 0 0 0.8em; }
  .assistant-content :global(p:last-child) { margin-bottom: 0; }
  .assistant-content :global(ul), .assistant-content :global(ol) {
    padding-left: 1.5em; margin: 0.4em 0;
  }
  .assistant-content :global(li) { margin: 0.2em 0; }
  .assistant-content :global(pre) {
    background: var(--bg-secondary);
    padding: 16px;
    border-radius: 12px;
    overflow-x: auto;
    margin: 0.8em 0;
    border: 1px solid var(--border);
  }
  .assistant-content :global(code) {
    font-family: var(--font-mono);
    font-size: 13px;
  }
  .assistant-content :global(pre code) {
    background: none; padding: 0;
  }
  .assistant-content :global(blockquote) {
    border-left: 2px solid var(--accent);
    margin: 0.8em 0;
    padding: 0.2em 1em;
    color: var(--text-secondary);
  }

  .images {
    display: flex; flex-wrap: wrap; gap: 8px;
    margin-bottom: 8px;
  }

  .attached-img {
    max-width: 200px; max-height: 200px;
    border-radius: 8px; object-fit: cover;
    border: 1px solid var(--border);
  }

  .typing-dots {
    display: inline-flex; gap: 4px; align-items: center;
  }

  .typing-dots span {
    width: 5px; height: 5px;
    border-radius: 50%;
    background: var(--text-tertiary);
    animation: dotBounce 1.4s ease-in-out infinite both;
  }

  .typing-dots span:nth-child(1) { animation-delay: 0s; }
  .typing-dots span:nth-child(2) { animation-delay: .2s; }
  .typing-dots span:nth-child(3) { animation-delay: .4s; }

  @keyframes dotBounce {
    0%, 80%, 100% { transform: scale(0.4); opacity: .4; }
    40% { transform: scale(1); opacity: 1; }
  }

  @keyframes fadeIn {
    from { opacity: 0; transform: translateY(4px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  .actions-wrapper {
    position: absolute;
    top: -28px;
    right: 0;
  }
</style>
