<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchProjects, createProject, fetchConfig } from '../lib/api';
  import type { ProjectInfo } from '../lib/api';
  import { store, setPage, showToast } from '../lib/store.svelte';
  import { uiStore } from '../lib/stores/ui.svelte';
  import { t } from '../lib/i18n';
  import { relativeTime } from '../lib/time';
  import ShimmerLogo from '../components/ShimmerLogo.svelte';

  let projects = $state<ProjectInfo[]>([]);
  let search = $state('');
  let sortKey = $state<'updatedAt' | 'name' | 'count'>('updatedAt');
  let showNewProject = $state(false);
  let newProjectName = $state('');
  let newProjectCwd = $state('');

  async function load() {
    try { projects = await fetchProjects(); }
    catch { showToast('Failed to load projects', 'error'); }
    try { await fetchConfig(); }
    catch { /* noop */ }
  }
  onMount(() => load());

  const filtered = $derived(search
    ? projects.filter(p => {
        const name = p.cwd.split('/').pop() || p.cwd;
        return name.toLowerCase().includes(search.toLowerCase()) || p.cwd.toLowerCase().includes(search.toLowerCase());
      })
    : projects);

  const sorted = $derived.by(() => {
    const arr = [...filtered];
    if (sortKey === 'name') arr.sort((a, b) => (a.cwd.split('/').pop() || a.cwd).localeCompare(b.cwd.split('/').pop() || b.cwd));
    else if (sortKey === 'count') arr.sort((a, b) => b.sessionCount - a.sessionCount);
    else arr.sort((a, b) => b.updatedAt - a.updatedAt);
    return arr;
  });

  function goProject(id: string, _cwd: string) { uiStore.projectId = id; window.location.hash = 'project/' + id; }

  function startNewProject() { showNewProject = true; newProjectName = ''; newProjectCwd = ''; }
  async function doCreateProject() {
    const name = newProjectName.trim() || newProjectCwd.split('/').pop() || '';
    const cwd = newProjectCwd.trim();
    if (!cwd) { showNewProject = false; return; }
    try {
      const p = await createProject(name || cwd.split('/').pop() || 'project', cwd);
      uiStore.projectId = p.id;
      showNewProject = false;
      projects = await fetchProjects();
      setPage('project');
    } catch { showToast('Failed to create project', 'error'); }
  }
  function handleNewProjectKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') doCreateProject();
    else if (e.key === 'Escape') { showNewProject = false; }
  }
  function cancelNewProject() { showNewProject = false; }
</script>

<div class="page">
  <div class="header">
    <div class="header-left">
      <h2 class="title">{t('nav.projects')}</h2>
    </div>
    <div class="header-right">
      {#if showNewProject}
        <div class="new-project-input-wrap">
          <input class="new-project-input-name" type="text" placeholder="Project name (optional)" bind:value={newProjectName} onkeydown={handleNewProjectKeydown} />
          <input class="new-project-input-cwd" type="text" placeholder="Absolute path e.g. /home/user/project" bind:value={newProjectCwd} onkeydown={handleNewProjectKeydown} onblur={cancelNewProject} autofocus />
        </div>
      {:else}
        <button class="new-project-btn" onclick={startNewProject} title={t('projects.newProject')}>
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          <span>{t('projects.newProject')}</span>
        </button>
      {/if}
      <div class="search-wrap">
        <svg class="search-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
        <input class="search-input" type="text" placeholder={t('projects.searchPlaceholder')} bind:value={search} />
      </div>
      <div class="sort-wrap">
        <select class="sort-select" bind:value={sortKey}>
          <option value="updatedAt">{t('projects.sortLastActive')}</option>
          <option value="name">{t('projects.sortName')}</option>
          <option value="count">{t('projects.sortCount')}</option>
        </select>
      </div>
    </div>
  </div>
  <div class="body">
    {#if projects.length === 0}
      <div class="empty">
        <ShimmerLogo />
        <h3 class="empty-title">{t('projects.emptyTitle')}</h3>
        <p class="empty-sub">{t('projects.emptySub')}</p>
      </div>
    {:else if sorted.length === 0}
      <p class="msg">{t('projects.noMatch')}</p>
    {:else}
      <div class="card-grid">
        {#each sorted as p, i}
          {@const name = p.cwd.split('/').pop() || p.cwd}
          <button class="card" style="animation-delay:{i * 40}ms" onclick={() => goProject(p.id, p.cwd)}>
            <div class="card-info">
              <div class="card-name">{name}</div>
              <div class="card-path" title={p.cwd}>{p.cwd}</div>
            </div>
            <div class="card-stats">
              <span class="card-sessions">{t('projects.sessionsCount', { n: p.sessionCount })}</span>
              <span class="card-time">{p.updatedAt ? relativeTime(p.updatedAt) : ''}</span>
            </div>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    padding: 12px 20px; border-bottom: 1px solid var(--border); flex-shrink: 0;
    display: flex; align-items: center; justify-content: space-between; gap: 16px;
  }
  .header-left { display: flex; align-items: center; }
  .header-right { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
  .new-project-btn {
    display: flex; align-items: center; gap: 4px; padding: 4px 10px;
    border-radius: var(--radius-sm); border: 1px solid var(--border);
    color: var(--text-secondary); background: var(--bg-primary);
    font-size: 12px; font-family: var(--font-system); cursor: pointer;
    transition: all .15s; white-space: nowrap;
  }
  .new-project-btn:hover { border-color: var(--accent); color: var(--accent); background: var(--accent-light); }
  .new-project-input-wrap {
    display: flex; align-items: center; background: var(--bg-primary);
    border: 1px solid var(--accent); border-radius: var(--radius-sm);
    padding: 4px 8px; box-shadow: 0 0 0 3px var(--accent-light);
  }
  .new-project-input-name {
    border: none; background: none; outline: none; font-size: 12px;
    color: var(--text-primary); font-family: var(--font-system); width: 140px;
    padding: 0; border-right: 1px solid var(--border); margin-right: 6px; padding-right: 6px;
  }
  .new-project-input-cwd {
    border: none; background: none; outline: none; font-size: 12px;
    color: var(--text-primary); font-family: var(--font-mono); width: 260px;
    padding: 0;
  }
  .new-project-input-name::placeholder,
  .new-project-input-cwd::placeholder { color: var(--text-tertiary); }
  .title { font-size: 18px; font-weight: 700; color: var(--text-primary); }
  .search-wrap {
    display: flex; align-items: center; gap: 6px;
    background: var(--bg-primary); border: 1px solid var(--border);
    border-radius: var(--radius-sm); padding: 4px 8px; width: 180px;
    transition: border-color .15s, box-shadow .15s;
  }
  .search-wrap:focus-within { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-light); }
  .search-icon { color: var(--text-tertiary); flex-shrink: 0; }
  .search-input { flex: 1; border: none; background: none; outline: none; font-size: 12px; color: var(--text-primary); padding: 0; font-family: var(--font-system); }
  .search-input::placeholder { color: var(--text-tertiary); }
  .sort-select {
    font-size: 12px; color: var(--text-secondary); background: var(--bg-secondary);
    border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 4px 8px;
    font-family: var(--font-system); outline: none; cursor: pointer;
  }
  .sort-select:focus { border-color: var(--accent); }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }

  .card-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 12px; }
  .card {
    display: flex; flex-direction: column;
    border-radius: var(--radius-lg); border: 1px solid var(--border);
    background: var(--bg-secondary); text-align: left;
    transition: all .2s var(--ease-out);
    animation: cardIn 350ms var(--ease-spring) both;
  }
  .card:hover { border-color: var(--accent); transform: translateY(-2px); box-shadow: 0 4px 16px rgba(0,0,0,.06); }
  .card-info { padding: 16px 16px 4px; min-width: 0; }
  .card-name { font-size: 14px; font-weight: 600; color: var(--text-primary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .card-path { font-size: 11px; color: var(--text-tertiary); font-family: var(--font-mono); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-top: 1px; }
  .card-stats { display: flex; flex-direction: column; gap: 2px; padding: 0 16px 16px; font-size: 11px; color: var(--text-tertiary); }
  .card-sessions { color: var(--text-secondary); }
  .card-time { margin-top: 4px; }

  .empty { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 48px 20px; text-align: center; }
  .empty-title { font-size: 16px; font-weight: 700; color: var(--text-primary); }
  .empty-sub { font-size: 13px; color: var(--text-secondary); max-width: 340px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }

  @keyframes cardIn {
    from { opacity: 0; transform: translateY(12px) scale(0.97); }
    to { opacity: 1; transform: translateY(0) scale(1); }
  }
</style>
