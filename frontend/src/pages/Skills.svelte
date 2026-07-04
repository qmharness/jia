<script lang="ts">
  import { onMount } from 'svelte';
  import { fetchSkills, fetchEvolution, reloadSkills, toggleSkill, toggleEvolve } from '../lib/api';
  import type { SkillInfo, EvolutionData, ReflectionSummary } from '../lib/api';
  import { showToast } from '../lib/store.svelte';
  import { t } from '../lib/i18n';

  let skills = $state<SkillInfo[]>([]);
  let evolution = $state<EvolutionData | null>(null);
  let loading = $state(true);
  let expanded = $state<string | null>(null);
  let activeTab = $state<Record<string, 'details' | 'evolution'>>({});

  onMount(async () => {
    try {
      skills = await fetchSkills();
      evolution = await fetchEvolution();
    } catch { showToast(t('skills.loadFailed'), 'error'); }
    loading = false;
  });

  async function onReload() {
    try {
      const n = await reloadSkills();
      skills = await fetchSkills();
      evolution = await fetchEvolution();
      showToast(t('skills.reloadedToast', { n }), 'success');
    } catch { showToast(t('skills.reloadFailed'), 'error'); }
  }

  async function onToggle(s: SkillInfo) {
    try {
      const newDisabled = !s.disabled;
      await toggleSkill(s.name, newDisabled);
      s.disabled = newDisabled;
      showToast(newDisabled ? t('skills.disabledToast', { name: s.name }) : t('skills.enabledToast', { name: s.name }), 'success');
    } catch { showToast(t('skills.toggleFailed'), 'error'); }
  }

  async function onToggleEvolve(s: SkillInfo) {
    try {
      const enabled = !s.auto_evolve;
      await toggleEvolve(s.name, enabled);
      s.auto_evolve = enabled;
      showToast(enabled ? t('skills.evolveOnToast', { name: s.name }) : t('skills.evolveOffToast', { name: s.name }), 'success');
    } catch { showToast(t('skills.toggleFailed'), 'error'); }
  }

  function toggleExpand(name: string) {
    if (expanded === name) {
      expanded = null;
    } else {
      expanded = name;
      if (!activeTab[name]) activeTab[name] = 'details';
    }
  }

  function evolveEligible(s: SkillInfo): boolean {
    return !s.always && !s.has_paths;
  }

  function evolveLabel(s: SkillInfo): string {
    if (!evolveEligible(s)) return t('skills.evolveStatic');
    return s.auto_evolve ? t('skills.evolveEvolving') : t('skills.evolveEvolvable');
  }

  function evolveCssClass(s: SkillInfo): string {
    if (!evolveEligible(s)) return 'evolve-tag static';
    return s.auto_evolve ? 'evolve-tag evolving' : 'evolve-tag evolvable';
  }

  function skillSummary(skillName: string): ReflectionSummary | undefined {
    return evolution?.reflection_summaries.find(s => s.skill_name === skillName);
  }

  function skillRevisions(skillName: string) {
    return evolution?.recent_revisions.filter(r => r.skill_name === skillName) ?? [];
  }

  function errorDelta(rev: { pre_revision_error_rate: number | null; post_revision_error_rate: number | null }): string {
    if (rev.pre_revision_error_rate == null || rev.post_revision_error_rate == null) return '';
    if (rev.pre_revision_error_rate === 0) return '';
    const pct = ((rev.pre_revision_error_rate - rev.post_revision_error_rate) / rev.pre_revision_error_rate * 100);
    return (pct >= 0 ? '−' : '+') + Math.abs(pct).toFixed(0) + '%';
  }

  function errorImproved(rev: { pre_revision_error_rate: number | null; post_revision_error_rate: number | null }): boolean {
    return (rev.post_revision_error_rate ?? Infinity) < (rev.pre_revision_error_rate ?? Infinity);
  }

  const enabledCount = $derived(skills.filter(s => s.auto_evolve).length);
  const eligibleCount = $derived(skills.filter(s => evolveEligible(s)).length);
</script>

<div class="page">
  <div class="header">
    <h2 class="title">{t('skills.title')}</h2>
    <div class="actions">
      <span class="count">{t('skills.loaded', { n: skills.length })}</span>
      <button class="btn" onclick={onReload}>{t('skills.reload')}</button>
    </div>
  </div>
  <div class="body">
    {#if loading}
      <p class="msg">{t('common.loading')}</p>
    {:else if skills.length === 0}
      <p class="msg">{t('skills.none')}</p>
    {:else}
      <!-- Evolution summary bar -->
      {#if evolution}
        <div class="evo-summary">
          <span class="evo-summary-title">{t('skills.evolutionSummary')}</span>
          <span class="evo-stat">
            <strong>{t('skills.revisionCount')}</strong> {evolution.total_revisions}
          </span>
          <span class="evo-stat">
            <strong>{t('skills.autoEvolve')}</strong> {enabledCount}/{eligibleCount}
          </span>
          {#if evolution.confidence_trend.length > 1}
            <div class="sparkline" title="{t('skills.confidenceTrend')}">
              {#each evolution.confidence_trend as c}
                <span class="spark-bar" style="height: {Math.max(c * 100, 4)}%"></span>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      <div class="skill-list">
        {#each skills as s}
          <div class="skill-item">
            <div class="skill-header">
              <button class="skill-header-btn" onclick={() => toggleExpand(s.name)}>
                <div class="skill-header-left">
                  <span class="status-dot" class:status-on={!s.disabled} class:status-off={s.disabled} title={s.disabled ? t('skills.disabled') : t('skills.enabled')}></span>
                  <span class="skill-name">{s.name}</span>
                  <span class={evolveCssClass(s)}>{evolveLabel(s)}</span>
                </div>
              </button>
              <div class="header-toggles">
                {#if evolveEligible(s)}
                  <label class="toggle-switch" title={s.auto_evolve ? t('skills.evolveEvolving') : t('skills.evolveEvolvable')}>
                    <input type="checkbox" checked={s.auto_evolve} onchange={() => onToggleEvolve(s)} />
                    <span class="toggle-track">
                      <span class="toggle-label on">{t('skills.evolveOn')}</span>
                      <span class="toggle-label off">{t('skills.evolveOff')}</span>
                      <span class="toggle-knob"></span>
                    </span>
                  </label>
                {/if}
                <label class="toggle-switch" title={s.disabled ? t('skills.disabled') : t('skills.enabled')}>
                  <input type="checkbox" checked={!s.disabled} onchange={() => onToggle(s)} />
                  <span class="toggle-track">
                    <span class="toggle-label on">{t('skills.enabled')}</span>
                    <span class="toggle-label off">{t('skills.disabled')}</span>
                    <span class="toggle-knob"></span>
                  </span>
                </label>
              </div>
              <button class="expand-icon" onclick={() => toggleExpand(s.name)}>{expanded === s.name ? '▾' : '▸'}</button>
            </div>
            <p class="skill-desc">{s.description}</p>
            <span class="skill-path">{s.source_path}</span>

            {#if expanded === s.name}
              <!-- Tab bar -->
              <div class="tabs">
                <button
                  class="tab"
                  class:active={activeTab[s.name] === 'details'}
                  onclick={() => activeTab[s.name] = 'details'}
                >{t('skills.detailsTab')}</button>
                {#if s.auto_evolve}
                  <button
                    class="tab"
                    class:active={activeTab[s.name] === 'evolution'}
                    onclick={() => activeTab[s.name] = 'evolution'}
                  >{t('skills.evolutionTab')}</button>
                {/if}
              </div>

              {#if activeTab[s.name] === 'details'}
                <pre class="skill-prompt">{s.prompt}</pre>
              {:else if activeTab[s.name] === 'evolution'}
                {#if true}
                  {@const summary = skillSummary(s.name)}
                  {@const revisions = skillRevisions(s.name)}
                <div class="evo-detail">
                  {#if summary}
                    <div class="evo-section">
                      <h4 class="evo-section-title">{t('skills.reflectionCount')} ({summary.total_reflections})</h4>
                      {#if summary.by_type.length > 0}
                        <table class="evo-table">
                          <thead>
                            <tr>
                              <th>Type</th>
                              <th>{t('skills.reflectionCount')}</th>
                              <th>{t('skills.confidence')}</th>
                            </tr>
                          </thead>
                          <tbody>
                            {#each summary.by_type as bt}
                              <tr>
                                <td>{bt.reflection_type}</td>
                                <td>{bt.count}</td>
                                <td>{(bt.avg_confidence * 100).toFixed(0)}%</td>
                              </tr>
                            {/each}
                          </tbody>
                        </table>
                      {:else}
                        <p class="evo-empty">{t('skills.noEvolutionData')}</p>
                      {/if}
                    </div>
                  {:else}
                    <p class="evo-empty">{t('skills.noEvolutionData')}</p>
                  {/if}

                  {#if revisions.length > 0}
                    <div class="evo-section">
                      <h4 class="evo-section-title">{t('skills.revisionCount')} ({revisions.length})</h4>
                      {#each revisions as rev}
                        <div class="revision-card">
                          <div class="rev-header">
                            <span class="rev-session">Session: {rev.session_id.slice(0, 12)}...</span>
                            <span class="rev-conf">
                              {t('skills.confidence')}: {(rev.avg_confidence * 100).toFixed(0)}%
                            </span>
                            {#if rev.applied}
                              <span class="rev-badge applied">applied</span>
                            {:else}
                              <span class="rev-badge logged">logged</span>
                            {/if}
                          </div>
                          <!-- Confidence bar -->
                          <div class="conf-bar-track">
                            <div
                              class="conf-bar-fill"
                              class:applied={rev.applied}
                              class:logged={!rev.applied}
                              style="width: {Math.max(rev.avg_confidence * 100, 2)}%"
                            ></div>
                          </div>
                          {#if rev.pre_revision_error_rate != null}
                            <div class="error-rate">
                              <span class="error-label">{t('skills.errorRate')}:</span>
                              <span class="error-val pre">{rev.pre_revision_error_rate.toFixed(1)}</span>
                              {#if rev.post_revision_error_rate != null}
                                <span class="error-arrow">→</span>
                                <span class="error-val post">{rev.post_revision_error_rate.toFixed(1)}</span>
                                {@const delta = errorDelta(rev)}
                                {#if delta}
                                  <span class="error-delta" class:improved={errorImproved(rev)} class:worse={!errorImproved(rev)}>{delta}</span>
                                {/if}
                              {/if}
                              <span class="error-unit"> {t('skills.errorsPerReflection')}</span>
                            </div>
                          {/if}
                          <pre class="diff-text">{rev.diff_text}</pre>
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
                {/if}
              {/if}
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .page { display: flex; flex-direction: column; height: 100%; }
  .header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 12px 20px; border-bottom: 1px solid var(--border);
  }
  .title { font-size: 16px; font-weight: 600; }
  .actions { display: flex; gap: 10px; align-items: center; }
  .count { font-size: 13px; color: var(--text-tertiary); }
  .btn {
    font-size: 13px; padding: 4px 12px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    color: var(--text-secondary); background: var(--bg-secondary);
    transition: all .15s;
  }
  .btn:hover { background: var(--bg-tertiary); }
  .body { flex: 1; overflow-y: auto; padding: 16px 20px; }
  .msg { text-align: center; color: var(--text-secondary); padding: 40px; }

  /* Evolution summary bar */
  .evo-summary {
    display: flex; align-items: center; gap: 16px;
    padding: 10px 14px; margin-bottom: 12px;
    background: var(--bg-secondary); border: 1px solid var(--border);
    border-radius: var(--radius-md); font-size: 13px;
  }
  .evo-summary-title { font-weight: 600; color: var(--text-primary); }
  .evo-stat { color: var(--text-secondary); }
  .evo-stat strong { color: var(--text-primary); margin-right: 4px; }

  /* Skill list */
  .skill-list { display: flex; flex-direction: column; gap: 8px; }
  .skill-item {
    border: 1px solid var(--border); border-radius: var(--radius-md);
    padding: 12px 14px;
  }
  .skill-header {
    display: flex; justify-content: space-between; align-items: center;
    width: 100%;
  }
  .skill-header-btn {
    display: flex; align-items: center; flex: 1;
    text-align: left; min-width: 0;
  }
  .skill-header-left { display: flex; align-items: center; gap: 8px; }
  .skill-name { font-weight: 600; font-size: 14px; }
  .expand-icon { font-size: 12px; color: var(--text-tertiary); cursor: pointer; flex-shrink: 0; margin-left: 12px; }
  .skill-desc { font-size: 13px; color: var(--text-secondary); margin-top: 2px; }
  .skill-path { font-size: 11px; color: var(--text-tertiary); font-family: monospace; }
  .skill-prompt {
    margin-top: 8px; background: var(--bg-secondary); padding: 10px;
    border-radius: var(--radius-sm); font-size: 12px; font-family: monospace;
    max-height: 300px; overflow-y: auto; white-space: pre-wrap;
  }

  /* Status dot */
  .status-dot {
    width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0;
  }
  .status-dot.status-on { background: #28a745; }
  .status-dot.status-off { background: #dc3545; }

  /* Header toggles container */
  .header-toggles { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }

  /* CSS toggle switch */
  .toggle-switch { position: relative; display: inline-block; cursor: pointer; }
  .toggle-switch input { position: absolute; opacity: 0; width: 0; height: 0; }
  .toggle-track {
    display: flex; align-items: center; width: 56px; height: 22px;
    border-radius: 11px; background: #ccc; position: relative;
    transition: background .2s; overflow: hidden;
  }
  .toggle-switch input:checked + .toggle-track { background: #28a745; }
  .toggle-knob {
    position: absolute; top: 2px; left: 2px; width: 18px; height: 18px;
    border-radius: 50%; background: #fff; transition: left .2s;
    box-shadow: 0 1px 3px rgba(0,0,0,.2);
  }
  .toggle-switch input:checked + .toggle-track .toggle-knob { left: 36px; }
  .toggle-label {
    position: absolute; font-size: 9px; font-weight: 600; color: #fff;
    pointer-events: none; user-select: none; transition: opacity .15s;
  }
  .toggle-label.on { left: 7px; opacity: 0; }
  .toggle-label.off { right: 7px; opacity: 1; }
  .toggle-switch input:checked + .toggle-track .toggle-label.on { opacity: 1; }
  .toggle-switch input:checked + .toggle-track .toggle-label.off { opacity: 0; }

  /* Auto-evolve tag */
  .evolve-tag {
    font-size: 10px; padding: 1px 6px; border-radius: 9px; font-weight: 500;
    flex-shrink: 0;
  }
  .evolve-tag.evolving { background: #d4edda; color: #155724; }
  .evolve-tag.static { background: #f1f1f1; color: #999; }
  .evolve-tag.evolvable { background: #fff3cd; color: #856404; }

  /* Tabs */
  .tabs {
    display: flex; gap: 0; margin-top: 10px;
    border-bottom: 2px solid var(--border);
  }
  .tab {
    padding: 6px 14px; font-size: 13px; color: var(--text-secondary);
    border-bottom: 2px solid transparent; margin-bottom: -2px;
    transition: all .15s;
  }
  .tab:hover { color: var(--text-primary); }
  .tab.active {
    color: var(--accent); border-bottom-color: var(--accent); font-weight: 600;
  }

  /* Evolution detail */
  .evo-detail { margin-top: 10px; }
  .evo-section { margin-bottom: 14px; }
  .evo-section-title { font-size: 13px; font-weight: 600; color: var(--text-primary); margin-bottom: 6px; }
  .evo-empty { font-size: 13px; color: var(--text-tertiary); padding: 12px 0; }

  .evo-table {
    width: 100%; font-size: 12px; border-collapse: collapse;
  }
  .evo-table th, .evo-table td {
    text-align: left; padding: 4px 8px; border-bottom: 1px solid var(--border);
  }
  .evo-table th { color: var(--text-tertiary); font-weight: 500; }
  .evo-table td { color: var(--text-secondary); }

  /* Revision cards */
  .revision-card {
    background: var(--bg-secondary); border: 1px solid var(--border);
    border-radius: var(--radius-sm); padding: 10px; margin-bottom: 8px;
  }
  .rev-header {
    display: flex; align-items: center; gap: 10px; font-size: 12px;
    margin-bottom: 6px;
  }
  .rev-session { color: var(--text-tertiary); font-family: monospace; }
  .rev-conf { color: var(--text-secondary); }
  .rev-badge {
    font-size: 10px; padding: 1px 6px; border-radius: 8px; font-weight: 500;
  }
  .rev-badge.applied { background: #d4edda; color: #155724; }
  .rev-badge.logged { background: #f1f1f1; color: #666; }

  /* Confidence bar */
  .conf-bar-track {
    height: 12px; background: var(--bg-tertiary); border-radius: 6px;
    margin-bottom: 8px; overflow: hidden;
  }
  .conf-bar-fill { height: 100%; border-radius: 6px; transition: width .3s; }
  .conf-bar-fill.applied { background: #28a745; }
  .conf-bar-fill.logged { background: #999; }

  .diff-text {
    font-size: 11px; font-family: monospace; background: var(--bg-tertiary);
    padding: 8px; border-radius: 4px; max-height: 200px; overflow-y: auto;
    white-space: pre-wrap; color: var(--text-secondary);
  }

  /* Sparkline */
  .sparkline {
    display: flex; align-items: flex-end; gap: 2px;
    height: 22px; margin-left: auto;
  }
  .spark-bar {
    width: 4px; min-height: 4px; border-radius: 2px;
    background: var(--accent); opacity: 0.7;
  }

  /* Error rate */
  .error-rate {
    font-size: 11px; color: var(--text-secondary); margin-bottom: 6px;
    display: flex; align-items: center; gap: 4px; flex-wrap: wrap;
  }
  .error-label { color: var(--text-tertiary); }
  .error-val.pre { color: var(--text-secondary); font-weight: 600; }
  .error-val.post { color: var(--text-primary); font-weight: 600; }
  .error-arrow { color: var(--text-tertiary); }
  .error-delta { font-size: 10px; }
  .error-delta.improved { color: #28a745; }
  .error-delta.worse { color: #dc3545; }
  .error-unit { color: var(--text-tertiary); }

</style>
