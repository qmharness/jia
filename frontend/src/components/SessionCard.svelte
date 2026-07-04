<script lang="ts">
  import type { SessionMeta } from '../lib/types';
  import { relativeTime } from '../lib/time';
  import { t } from '../lib/i18n';

  let {
    session,
    active,
    selected,
    onselect,
    ontoggle,
    onrename,
    ondelete,
  }: {
    session: SessionMeta;
    active: boolean;
    selected: boolean;
    onselect: () => void;
    ontoggle: () => void;
    onrename: (title: string) => Promise<void>;
    ondelete: () => void;
  } = $props();

  let menuOpen = $state(false);
  let renaming = $state(false);
  let renameText = $state('');
  let renameInput: HTMLInputElement | undefined = $state();

  function startRename() {
    renameText = session.title;
    renaming = true;
    menuOpen = false;
    requestAnimationFrame(() => renameInput?.focus());
  }

  function cancelRename() {
    renaming = false;
  }

  async function commitRename() {
    const t = renameText.trim();
    if (!t || t === session.title) { renaming = false; return; }
    await onrename(t);
    renaming = false;
  }

  function onRenameKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') { e.preventDefault(); commitRename(); }
    if (e.key === 'Escape') cancelRename();
  }

  function onRowLeave() {
    menuOpen = false;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<tr
  class="row"
  class:active
  class:selected
  onclick={onselect}
  onmouseleave={onRowLeave}
  onkeydown={() => {}}
>
  <td class="cell-check" onclick={(e: MouseEvent) => { e.stopPropagation(); ontoggle(); }}>
    <span class="checkbox" class:checked={selected}>
      {#if selected}✓{/if}
    </span>
  </td>
  <td class="cell-title">
    {#if renaming}
      <input
        class="rename-input"
        type="text"
        bind:value={renameText}
        bind:this={renameInput}
        onkeydown={onRenameKeydown}
        onblur={commitRename}
        onclick={(e: MouseEvent) => e.stopPropagation()}
      />
    {:else}
      <span class="title-text truncate">{session.title || t('sessions.untitled')}</span>
    {/if}
  </td>
  <td class="cell-id">
    <span class="id-text truncate">{session.id}</span>
  </td>
  <td class="cell-project">
    {#if session.cwd}
      <span class="project-text truncate" title={session.cwd}>
        <span class="project-name">{session.cwd.split('/').filter(Boolean).pop() || session.cwd}</span>
        <span class="project-path">{session.cwd}</span>
      </span>
    {:else}
      <span class="project-none">—</span>
    {/if}
  </td>
  <td class="cell-msgs">{t('sessions.msgs', { n: session.messageCount })}</td>
  <td class="cell-time">{relativeTime(session.updatedAt)}</td>
  <td class="cell-actions" onclick={(e: MouseEvent) => e.stopPropagation()}>
    <div class="menu-wrap">
      <button class="btn-menu" onclick={() => menuOpen = !menuOpen} aria-label="Actions">⋯</button>
      {#if menuOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="menu-dropdown" onclick={(e: MouseEvent) => e.stopPropagation()} onkeydown={() => {}}>
          <button class="menu-item" onclick={startRename}>{t('sessions.rename')}</button>
          <button class="menu-item menu-danger" onclick={() => { menuOpen = false; ondelete(); }}>{t('sessions.deleteItem')}</button>
        </div>
      {/if}
    </div>
  </td>
</tr>

<style>
  .row {
    cursor: pointer;
    transition: background .1s;
    border-bottom: 1px solid var(--border);
  }
  .row:hover { background: var(--accent-light); }
  .row.active { background: var(--accent-light); }
  .row:last-child { border-bottom: none; }

  td {
    padding: 8px 12px;
    vertical-align: middle;
    white-space: nowrap;
  }

  .cell-check {
    width: 40px;
    text-align: center;
    padding-right: 0;
  }

  .checkbox {
    width: 18px; height: 18px;
    border-radius: 4px;
    border: 2px solid var(--border);
    display: inline-flex; align-items: center; justify-content: center;
    font-size: 11px; color: #fff;
    transition: all .15s;
    cursor: pointer;
  }
  .checkbox.checked {
    background: var(--accent);
    border-color: var(--accent);
  }

  .cell-title {
    max-width: 500px;
  }
  .title-text {
    font-weight: 600;
    font-size: 14px;
    color: var(--text-primary);
    display: block;
  }
  .truncate {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .rename-input {
    width: 100%;
    font-size: 14px; font-weight: 600;
    padding: 2px 6px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--bg-primary);
    outline: none;
  }

  .cell-id {
    width: 280px;
  }
  .id-text {
    font-size: 12px;
    color: var(--text-tertiary);
    font-family: 'SF Mono', 'Fira Code', monospace;
    display: block;
  }

  .cell-project {
    max-width: 200px;
  }
  .project-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .project-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }
  .project-path {
    font-size: 11px;
    color: var(--text-tertiary);
    font-family: 'SF Mono', 'Fira Code', monospace;
  }
  .project-none {
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .cell-msgs {
    font-size: 13px;
    color: var(--text-secondary);
    width: 80px;
  }

  .cell-time {
    font-size: 13px;
    color: var(--text-tertiary);
    width: 100px;
  }

  .cell-actions {
    width: 50px;
    text-align: center;
    padding-left: 0;
  }

  .menu-wrap { position: relative; display: inline-block; }
  .btn-menu {
    width: 26px; height: 26px;
    border-radius: var(--radius-sm);
    display: flex; align-items: center; justify-content: center;
    color: var(--text-tertiary);
    font-size: 14px; transition: all .15s;
  }
  .btn-menu:hover { background: var(--bg-tertiary); color: var(--text-primary); }

  .menu-dropdown {
    position: absolute; right: 0; top: 100%;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-md);
    padding: 4px;
    z-index: 100;
    min-width: 110px;
  }
  .menu-item {
    display: block; width: 100%;
    text-align: left;
    padding: 5px 10px;
    font-size: 13px;
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    transition: background .1s;
  }
  .menu-item:hover { background: var(--bg-secondary); }
  .menu-danger { color: var(--error); }
  .menu-danger:hover { background: var(--error-light); }
</style>
