<script lang="ts">
  import { store, getSessionProvider, getSessionModel, setSessionProvider, setSessionModel, showToast } from '../lib/store.svelte';
  import { sendMessage } from '../lib/sse';
  import type { ImageContent, StreamAgentParams, Message } from '../lib/types';
  import { t } from '../lib/i18n';

  let text = $state('');
  let images = $state<ImageContent[]>([]);
  let fileInput: HTMLInputElement | undefined = $state();
  let textareaEl: HTMLTextAreaElement | undefined = $state();
  let hasActive = $derived(
    Object.keys(store.cancels).some(
      sid => store.streamSessions[sid] === store.activeSessionId
    )
  );
  let currentProvider = $derived(store.providers.find(p => p.name === getSessionProvider()));

  function onProviderChange(e: Event) {
    const newProvider = (e.target as HTMLSelectElement).value;
    const sid = store.activeSessionId;
    if (sid) {
      setSessionProvider(sid, newProvider);
      const p = store.providers.find(pp => pp.name === newProvider);
      if (p) setSessionModel(sid, p.default_model);
    }
  }

  function onModelChange(e: Event) {
    const newModel = (e.target as HTMLSelectElement).value;
    const sid = store.activeSessionId;
    if (sid) setSessionModel(sid, newModel);
  }

  import { onMount } from 'svelte';
  onMount(() => {
    textareaEl?.focus();
  });

  function triggerUpload() {
    fileInput?.click();
  }

  const MAX_IMAGE_BYTES = 10 * 1024 * 1024; // 10 MB

  async function onFiles(e: Event) {
    const input = e.target as HTMLInputElement;
    const files = input.files;
    if (!files) return;

    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      if (!file.type.startsWith('image/')) continue;
      if (file.size > MAX_IMAGE_BYTES) {
        showToast(`图片过大(${Math.round(file.size / 1024 / 1024)}MB),上限 10MB`, 'error');
        continue;
      }
      const data = await readAsBase64(file);
      images = [...images, { data, media_type: file.type }];
    }
    // Reset so the same file can be re-selected
    input.value = '';
    textareaEl?.focus();
  }

  function readAsBase64(file: File): Promise<string> {
    return new Promise((resolve) => {
      const reader = new FileReader();
      reader.onload = () => {
        const url = reader.result as string;
        // Strip "data:image/xxx;base64," prefix — backend expects raw base64
        const base64 = url.substring(url.indexOf(',') + 1);
        resolve(base64);
      };
      reader.readAsDataURL(file);
    });
  }

  function removeImage(idx: number) {
    images = images.filter((_, i) => i !== idx);
  }

  function onSubmit() {
    const hasText = !!text.trim();
    const hasImages = images.length > 0;
    if (!hasText && !hasImages) return;

    const messages: StreamAgentParams['messages'] = [{
      role: 'user',
      content: text.trim(),
      ...(hasImages ? { images: images.map(i => ({ data: i.data, media_type: i.media_type })) } : {}),
    }];

    const params: StreamAgentParams = {
      provider: getSessionProvider(),
      model: getSessionModel() || undefined,
      auxProvider: store.selectedAuxProvider || undefined,
      auxModel: store.selectedAuxModel || undefined,
      messages,
      sessionId: store.activeSessionId,
      cwd: undefined,
    };

    // Add user message to entries for display
    store.entries = [...store.entries, {
      id: crypto.randomUUID(),
      role: 'user',
      content: text.trim(),
      createdAt: Date.now(),
      ...(hasImages ? { images: images.map(i => ({ data: i.data, media_type: i.media_type })) } : {}),
    }];
    text = '';
    images = [];
    sendMessage(params);
  }

  function onStop() {
    // Cancel the most recent active stream for the current session
    const streamIds = Object.keys(store.cancels).filter(
      sid => store.streamSessions[sid] === store.activeSessionId
    );
    if (streamIds.length > 0) {
      store.cancels[streamIds[streamIds.length - 1]]?.();
    }
  }

  function undoLastUser() {
    // Cancel only streams belonging to the current session
    for (const sid of Object.keys(store.cancels)) {
      if (store.streamSessions[sid] === store.activeSessionId) {
        store.cancels[sid]?.();
      }
    }
    // Find the last user message and remove it + everything after
    const entries = store.entries;
    let lastUserIdx = -1;
    for (let i = entries.length - 1; i >= 0; i--) {
      if (entries[i].role === 'user') { lastUserIdx = i; break; }
    }
    if (lastUserIdx >= 0) {
      const undone = entries[lastUserIdx] as Message;
      store.entries = entries.slice(0, lastUserIdx);
      // Restore text and images
      text = undone.content;
      if (undone.images?.length) {
        images = undone.images;
      }
    }
    textareaEl?.focus();
  }

  function onSendClick() {
    if (hasActive) {
      onStop();
      undoLastUser();
    } else {
      onSubmit();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onSendClick();
    }
  }
</script>

<div class="input-area">
  {#if images.length > 0}
    <div class="previews">
      {#each images as img, i}
        <div class="preview-item">
          <img src="data:{img.media_type};base64,{img.data}" alt="preview" class="preview-img" />
          <button class="preview-remove" onclick={() => removeImage(i)} aria-label={t('chat.removeImage')}>&times;</button>
        </div>
      {/each}
    </div>
  {/if}
  <div class="input-wrapper">
    <input type="file" accept="image/*" multiple class="file-input" bind:this={fileInput} onchange={onFiles} />
    <button class="btn-upload" onclick={triggerUpload} aria-label={t('chat.uploadImage')}>
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg>
    </button>
    <textarea
      class="input"
      bind:value={text}
      bind:this={textareaEl}
      placeholder={t('chat.inputPlaceholder')}
      rows="2"
      onkeydown={onKeydown}
    ></textarea>
    <button
      class="btn-send"
      class:btn-stop-visual={hasActive}
      onclick={onSendClick}
      aria-label={hasActive ? t('chat.cancelUndo') : t('chat.send')}
    >
      {#if hasActive}
        <span class="stop-icon">■</span>
      {:else}
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
      {/if}
    </button>
  </div>
  <div class="model-row">
    <select class="model-select" value={getSessionProvider()} onchange={onProviderChange}>
      {#each store.providers as p}
        <option value={p.name}>{p.name}</option>
      {/each}
    </select>
    <select class="model-select" value={getSessionModel()} onchange={onModelChange}>
      {#each currentProvider?.models ?? [] as m}
        <option value={m}>{m}</option>
      {/each}
    </select>
  </div>
</div>

<style>
  .input-area {
    max-width: 768px;
    width: 100%;
    margin: 0 auto;
    padding: 0 20px 20px;
    background: var(--bg-primary);
  }

  .previews {
    display: flex;
    gap: 8px;
    margin-bottom: 8px;
    flex-wrap: wrap;
  }

  .preview-item {
    position: relative;
    width: 56px;
    height: 56px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    border: 1px solid var(--border);
  }

  .preview-img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .preview-remove {
    position: absolute;
    top: 0;
    right: 0;
    width: 18px;
    height: 18px;
    background: rgba(0,0,0,.5);
    color: #fff;
    font-size: 12px;
    line-height: 18px;
    text-align: center;
    border-radius: 0 0 0 var(--radius-sm);
    cursor: pointer;
    border: none;
  }

  .preview-remove:hover {
    background: rgba(0,0,0,.7);
  }

  .input-wrapper {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 22px;
    padding: 10px 16px;
    transition: border-color var(--duration-normal) var(--ease-out),
                box-shadow var(--duration-normal) var(--ease-out);
    box-shadow: var(--shadow-xs);
  }

  .input-wrapper:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-light);
  }

  .file-input {
    display: none;
  }

  .btn-upload {
    width: 34px;
    height: 34px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    color: var(--text-secondary);
    transition: all var(--duration-fast) var(--ease-out);
  }

  .btn-upload:hover:not(:disabled) {
    background: var(--bg-tertiary);
    color: var(--text-primary);
  }

  .btn-upload:disabled {
    opacity: .3;
    cursor: default;
  }

  .input {
    flex: 1;
    border: none;
    background: transparent;
    outline: none;
    resize: none;
    font-size: 14px;
    line-height: 1.6;
    max-height: 120px;
    color: var(--text-primary);
  }

  .input::placeholder {
    color: var(--text-tertiary);
  }

  .btn-send {
    width: 32px; height: 32px;
    border-radius: 50%;
    display: flex; align-items: center; justify-content: center;
    flex-shrink: 0;
    background: var(--accent);
    color: #fff;
    font-size: 16px;
  }
  .btn-send:hover:not(:disabled) { background: var(--accent-hover); }
  .btn-send:disabled { opacity: .3; cursor: default; }

  .btn-stop-visual { background: var(--error); }
  .btn-stop-visual:hover { background: #e02d20; }

  .stop-icon {
    font-size: 10px;
  }

  .model-row {
    display: flex; gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .model-select {
    font-size: 11px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-secondary);
    color: var(--text-secondary);
    outline: none;
  }
  .model-select:focus { border-color: var(--accent); }
</style>
