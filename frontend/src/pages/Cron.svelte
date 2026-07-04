<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchCronJobs, manageCronJob } from '../lib/api';
  import type { CronJob } from '../lib/api';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let jobs = $state<CronJob[]>([]);
  let loading = $state(true);
  let showModal = $state(false);
  let editing = $state(false);
  let deleting = $state<CronJob | null>(null);
  let form = $state({ name: '', schedule: '', prompt: '', cooldown_secs: '' });

  onMount(async () => {
    try { jobs = await fetchCronJobs(); } catch { showToast(t('cron.loadFailed'), 'error'); }
    loading = false;
  });

  async function reload() {
    try { jobs = await fetchCronJobs(); } catch { /* ignore */ }
  }

  function openAdd() {
    editing = false;
    form = { name: '', schedule: '', prompt: '', cooldown_secs: '' };
    showModal = true;
  }

  function openEdit(j: CronJob) {
    editing = true;
    form = {
      name: j.name,
      schedule: j.schedule,
      prompt: j.prompt,
      cooldown_secs: j.cooldown_secs != null ? String(j.cooldown_secs) : '',
    };
    showModal = true;
  }

  function closeModal() { showModal = false; }

  async function onSave() {
    if (!form.name || !form.schedule) {
      showToast(t('cron.validationRequired'), 'error');
      return;
    }
    const cd = form.cooldown_secs ? Number(form.cooldown_secs) : undefined;
    try {
      await manageCronJob({
        action: editing ? 'update' : 'add',
        name: form.name,
        schedule: form.schedule,
        prompt: form.prompt,
        cooldown_secs: cd,
      });
      showToast(t(editing ? 'cron.editSuccess' : 'cron.addSuccess', { name: form.name }));
      closeModal();
      await reload();
    } catch { showToast(t('cron.actionFailed'), 'error'); }
  }

  async function onDelete() {
    if (!deleting) return;
    try {
      await manageCronJob({ action: 'remove', name: deleting.name });
      showToast(t('cron.deleteSuccess', { name: deleting.name }));
      deleting = null;
      await reload();
    } catch { showToast(t('cron.actionFailed'), 'error'); }
  }

  async function onToggle(j: CronJob) {
    const prev = j.enabled;
    j.enabled = !j.enabled;
    try {
      await manageCronJob({ action: prev ? 'disable' : 'enable', name: j.name });
    } catch {
      j.enabled = prev;
      showToast(t('cron.toggleFailed'), 'error');
    }
  }
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('cron.title')}</h2>
    <div class="header-right">
      <span class="count">{t('cron.jobs', { n: jobs.length })}</span>
      <button class="btn-add" onclick={openAdd}>+ {t('cron.add')}</button>
    </div>
  </div>
  <div class="body">
    {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else if jobs.length === 0}
      <p class="msg">{t('cron.none')}</p>
    {:else}
      <div class="job-list">
        {#each jobs as j}
          <div class="job-card">
            <div class="job-top">
              <span class="job-name">{j.name}</span>
              <span class="job-schedule">{j.schedule}</span>
            </div>
            <p class="job-prompt">{j.prompt}</p>
            <div class="job-meta">
              <span class="job-cooldown">
                {t('cron.cooldown')}: {j.cooldown_secs ?? '—'}s
              </span>
              <span class="job-last">{j.last_fired_at ? new Date(j.last_fired_at * 1000).toLocaleString() : t('cron.never')}</span>
            </div>
            <div class="job-actions">
              <label class="toggle-switch" title={j.enabled ? t('cron.enabled') : t('cron.disabled')}>
                <input type="checkbox" checked={j.enabled} onchange={() => onToggle(j)} />
                <span class="toggle-track"></span>
              </label>
              <button class="btn-icon" onclick={() => openEdit(j)} title={t('cron.edit')}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
              </button>
              <button class="btn-icon btn-danger" onclick={() => deleting = j} title={t('cron.delete')}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<!-- Add/Edit Modal -->
{#if showModal}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-overlay" role="dialog" aria-modal="true" tabindex="-1" onclick={closeModal} onkeydown={(e) => e.key === 'Escape' && closeModal()}>
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_no_noninteractive_element_interactions -->
    <div class="modal" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
      <h3 class="modal-title">{editing ? t('cron.edit') : t('cron.add')}</h3>
      <div class="modal-body">
        <label class="field">
          <span class="field-label">{t('cron.name')}</span>
          <input type="text" bind:value={form.name} disabled={editing} />
        </label>
        <label class="field">
          <span class="field-label">{t('cron.schedule')}</span>
          <input type="text" bind:value={form.schedule} placeholder="*/5 * * * *" />
        </label>
        <label class="field">
          <span class="field-label">{t('cron.prompt')}</span>
          <textarea bind:value={form.prompt} rows="3"></textarea>
        </label>
        <label class="field">
          <span class="field-label">{t('cron.cooldown')}</span>
          <input type="number" bind:value={form.cooldown_secs} placeholder="72000" min="0" />
          <span class="field-hint">{t('cron.cooldownHint')}</span>
        </label>
      </div>
      <div class="modal-footer">
        <button class="btn-cancel" onclick={closeModal}>{t('cron.cancel')}</button>
        <button class="btn-save" onclick={onSave}>{t('cron.save')}</button>
      </div>
    </div>
  </div>
{/if}

<!-- Delete Confirmation Modal -->
{#if deleting}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-overlay" role="dialog" aria-modal="true" tabindex="-1" onclick={() => deleting = null} onkeydown={(e) => e.key === 'Escape' && (deleting = null)}>
    <!-- svelte-ignore a11y_no_static_element_interactions a11y_no_noninteractive_element_interactions -->
    <div class="modal modal-sm" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
      <h3 class="modal-title">{t('cron.delete')}</h3>
      <p class="modal-msg">{t('cron.deleteConfirm', { name: deleting.name })}</p>
      <div class="modal-footer">
        <button class="btn-cancel" onclick={() => deleting = null}>{t('cron.cancel')}</button>
        <button class="btn-save btn-danger-bg" onclick={onDelete}>{t('cron.delete')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 20px; border-bottom: 1px solid var(--border);
  }
  .title { font-size: 16px; font-weight: 600; }
  .header-right { display: flex; align-items: center; gap: 12px; }
  .count { font-size: 13px; color: var(--text-tertiary); }
  .btn-add {
    font-size: 13px; padding: 4px 12px;
    border: 1px solid var(--accent); border-radius: var(--radius-sm);
    color: var(--accent); background: transparent;
    transition: all .15s;
  }
  .btn-add:hover { background: var(--accent); color: #fff; }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }
  .job-list { display: flex; flex-direction: column; gap: 8px; }
  .job-card {
    border: 1px solid var(--border); border-radius: var(--radius-md);
    padding: 12px 14px; display: flex; flex-direction: column; gap: 6px;
  }
  .job-top { display: flex; justify-content: space-between; align-items: center; }
  .job-name { font-weight: 600; font-size: 14px; }
  .job-schedule {
    font-size: 12px; font-family: monospace; color: var(--accent);
    background: var(--accent-light); padding: 2px 8px; border-radius: var(--radius-sm);
  }
  .job-prompt { font-size: 13px; color: var(--text-secondary); margin: 0; }
  .job-meta { display: flex; justify-content: space-between; align-items: center; }
  .job-cooldown { font-size: 12px; color: var(--text-tertiary); }
  .job-last { font-size: 12px; color: var(--text-tertiary); }
  .job-actions {
    display: flex; align-items: center; gap: 8px;
    padding-top: 6px; border-top: 1px solid var(--border);
  }

  /* Toggle switch */
  .toggle-switch { position: relative; display: inline-block; width: 36px; height: 20px; cursor: pointer; }
  .toggle-switch input { opacity: 0; width: 0; height: 0; }
  .toggle-track {
    position: absolute; inset: 0;
    background: var(--border);
    border-radius: 10px;
    transition: background .2s;
  }
  .toggle-track::after {
    content: ''; position: absolute; width: 14px; height: 14px;
    left: 3px; top: 3px;
    background: #fff; border-radius: 50%;
    transition: transform .2s;
  }
  .toggle-switch input:checked + .toggle-track { background: var(--success); }
  .toggle-switch input:checked + .toggle-track::after { transform: translateX(16px); }

  /* Icon buttons */
  .btn-icon {
    display: flex; align-items: center; justify-content: center;
    width: 28px; height: 28px; border: none; border-radius: var(--radius-sm);
    background: transparent; color: var(--text-secondary); cursor: pointer;
    transition: all .15s;
  }
  .btn-icon:hover { background: var(--bg-hover); color: var(--text-primary); }
  .btn-danger:hover { color: var(--danger); background: var(--danger-light); }

  /* Modal */
  .modal-overlay {
    position: fixed; inset: 0; z-index: 100;
    background: rgba(0,0,0,.4); display: flex;
    align-items: center; justify-content: center;
  }
  .modal {
    background: var(--bg-primary); border-radius: var(--radius-lg);
    box-shadow: 0 8px 32px rgba(0,0,0,.2); width: 420px; max-width: 90vw;
  }
  .modal-sm { width: 360px; }
  .modal-title { font-size: 15px; font-weight: 600; padding: 16px 20px 0; }
  .modal-msg { padding: 12px 20px 0; font-size: 14px; color: var(--text-secondary); }
  .modal-body { padding: 16px 20px; display: flex; flex-direction: column; gap: 12px; }
  .modal-footer {
    display: flex; justify-content: flex-end; gap: 8px;
    padding: 0 20px 16px;
  }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .field-label { font-size: 13px; font-weight: 500; color: var(--text-secondary); }
  .field input, .field textarea {
    font-size: 13px; padding: 6px 10px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-input); color: var(--text-primary);
  }
  .field input:focus, .field textarea:focus { outline: none; border-color: var(--accent); }
  .field input:disabled { opacity: .5; }
  .field-hint { font-size: 11px; color: var(--text-tertiary); }
  .btn-cancel {
    font-size: 13px; padding: 6px 16px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: transparent; color: var(--text-secondary);
  }
  .btn-save {
    font-size: 13px; padding: 6px 16px;
    border: none; border-radius: var(--radius-sm);
    background: var(--accent); color: #fff;
  }
  .btn-danger-bg { background: var(--danger); }
</style>
