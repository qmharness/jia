import type { Provider, SessionMeta, StreamAgentParams, VijnanaState, VijnanaSeedsResponse } from './types';

// In Vite dev mode, use same-origin requests (proxied to daemon via vite.config.ts).
// In Tauri production, use the __JIA_API_BASE__ injected by initialization_script.
// In jia web, use window.location.origin (daemon serves the HTML, so origin is the
// correct address even with --port override). Fall back to 127.0.0.1:3000 otherwise.
const isViteDev = typeof window !== 'undefined' && window.location.hostname === 'localhost' && window.location.port === '5173';
const injectedBase = (typeof window !== 'undefined' && (window as any).__JIA_API_BASE__) || '';
const webOrigin = (typeof window !== 'undefined' && window.location.origin) || '';
export const API_BASE = isViteDev ? '' : (injectedBase || webOrigin || 'http://127.0.0.1:3000');
const D = API_BASE;
// 本模块内所有 fetch 调用先等 token 就绪再发出。
// 用同名 const 遮蔽全局 fetch,使下方所有 fetch(...) 自动经过 token 门控,无需逐处改动。
// NOTE: _origFetch 必须在 tokenReady 之前声明，因为 tokenReady 用它调 /auth/session。
const _origFetch: typeof window.fetch = window.fetch.bind(window);
const fetch: typeof window.fetch = (input, init) => tokenReady.then(() => _origFetch(input, init));

// Token: resolves asynchronously via Tauri IPC (jia-app) or POST /auth/session (jia web).
// All API calls await tokenReady before sending.
export let TOKEN = (typeof window !== 'undefined' && (window as any).__JIA_TOKEN__) || '';

export const tokenReady: Promise<void> = (async () => {
  try {
    const invoke = (window as any)?.__TAURI__?.core?.invoke;
    if (typeof invoke === 'function') {
      const t = await invoke('api_token');
      if (typeof t === 'string' && t) TOKEN = t;
      return;
    }
  } catch {
    // Not running inside Tauri — fall through to auth/session.
  }

  // jia web / Vite dev mode: token is not injected into HTML.
  // Fetch it from the daemon's localhost-gated /auth/session endpoint.
  if (!TOKEN) {
    try {
      const resp = await _origFetch(`${D}/auth/session`, { method: 'POST' });
      if (resp.ok) {
        const data = await resp.json();
        if (typeof data.token === 'string' && data.token) TOKEN = data.token;
      }
    } catch {
      // Daemon not reachable or auth/session not available (gateway-only mode).
    }
  }
})();

export function authHeaders(): Record<string, string> {
  return {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${TOKEN}`,
  };
}

export interface SessionMessagesResponse {
  session_id: string;
  entries: Array<Record<string, unknown>>;
}

export async function fetchSessionMessages(id: string, signal?: AbortSignal): Promise<SessionMessagesResponse> {
  const resp = await fetch(`${D}/sessions/${encodeURIComponent(id)}`, {
    headers: authHeaders(),
    ...(signal ? { signal } : {}),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function fetchConfig(): Promise<Record<string, any>> {
  const resp = await fetch(D + '/config', { headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function fetchProviders(): Promise<Provider[]> {
  const resp = await fetch(D + '/providers', { headers: authHeaders() });
  if (!resp.ok) return [];
  return resp.json();
}

export async function fetchSessions(filter?: string): Promise<SessionMeta[]> {
  const qs = filter ? `?filter=${filter}` : '?filter=active';
  const resp = await fetch(D + '/sessions' + qs, { headers: authHeaders() });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.sessions || [];
}

export async function archiveSession(id: string): Promise<void> {
  await fetch(`${D}/sessions/${encodeURIComponent(id)}/archive`, { method: 'POST', headers: authHeaders() });
}

export async function unarchiveSession(id: string): Promise<void> {
  await fetch(`${D}/sessions/${encodeURIComponent(id)}/unarchive`, { method: 'POST', headers: authHeaders() });
}

export async function deleteSession(id: string): Promise<void> {
  const resp = await fetch(`${D}/sessions/${encodeURIComponent(id)}`, { method: 'DELETE', headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
}

export async function bulkDeleteSessions(ids: string[]): Promise<void> {
  const resp = await fetch(D + '/sessions/bulk-delete', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ ids }),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
}

export async function renameSession(id: string, title: string): Promise<void> {
  const resp = await fetch(`${D}/sessions/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    headers: authHeaders(),
    body: JSON.stringify({ title }),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
}

export async function streamAgent(
  { provider, model, auxProvider, auxModel, messages, sessionId, cwd, projectId }: StreamAgentParams,
  signal: AbortSignal,
): Promise<{ reader: ReadableStreamDefaultReader<Uint8Array>; decoder: TextDecoder }> {
  const resp = await fetch(D + '/agent', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ provider, model, aux_provider: auxProvider || null, aux_model: auxModel || null, messages, session_id: sessionId || null, cwd: cwd || null, project_id: projectId || null }),
    signal,
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return {
    reader: resp.body!.getReader(),
    decoder: new TextDecoder(),
  };
}

export async function confirmAction(
  id: string,
  token: string,
  approved: boolean,
): Promise<{ status: string; resolved: boolean }> {
  const resp = await fetch(D + '/confirm', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ id, token, approved }),
  });
  return resp.json();
}

// ── Tools ──────────────────────────────────────────────

export interface ToolInfo {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export interface ToolGroup {
  category: string;
  tools: ToolInfo[];
}

export async function fetchToolGroups(): Promise<ToolGroup[]> {
  const resp = await fetch(D + '/tools', { headers: authHeaders() });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.groups || [];
}

// ── Files ──────────────────────────────────────────────

export interface FileListResponse {
  type: 'directory';
  entries: Array<{ name: string; path: string; isDir: boolean }>;
}

export interface FileContentResponse {
  type: 'file';
  content: string;
  name: string;
  path: string;
}

export async function fetchFiles(path?: string, root?: string): Promise<FileListResponse | FileContentResponse | { error: string }> {
  const params = new URLSearchParams();
  if (path) params.set('path', path);
  if (root) params.set('root', root);
  const qs = params.toString();
  const resp = await fetch(`${D}/files${qs ? '?' + qs : ''}`, { headers: authHeaders() });
  if (!resp.ok) return { error: `HTTP ${resp.status}` };
  return resp.json();
}

// ── Skills ─────────────────────────────────────────────

export interface SkillInfo {
  name: string;
  description: string;
  source_path: string;
  prompt: string;
  auto_evolve: boolean;
  evolve_min_confidence: number;
  evolve_max_revisions_per_session: number;
  evolve_reflection_threshold: number;
  always: boolean;
  has_paths: boolean;
  disabled: boolean;
}

export async function fetchSkills(): Promise<SkillInfo[]> {
  const resp = await fetch(D + '/skills', { headers: authHeaders() });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.skills || [];
}

// ── Evolution ──────────────────────────────────────────

export interface RevisionEntry {
  id: string;
  skill_name: string;
  session_id: string;
  diff_text: string;
  avg_confidence: number;
  pre_revision_error_rate: number | null;
  post_revision_error_rate: number | null;
  applied: boolean;
  created_at: number;
}

export interface ReflectionTypeBreakdown {
  reflection_type: string;
  count: number;
  avg_confidence: number;
}

export interface ReflectionSummary {
  skill_name: string;
  total_reflections: number;
  avg_confidence: number;
  by_type: ReflectionTypeBreakdown[];
}

export interface EvolutionData {
  recent_revisions: RevisionEntry[];
  reflection_summaries: ReflectionSummary[];
  confidence_trend: number[];
  total_revisions: number;
}

export async function fetchEvolution(): Promise<EvolutionData> {
  const resp = await fetch(D + '/skills/evolution', { headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function reloadSkills(): Promise<number> {
  const resp = await fetch(D + '/skills/reload', { method: 'POST', headers: authHeaders() });
  if (!resp.ok) return 0;
  const data = await resp.json();
  return data.loaded || 0;
}

export async function toggleSkill(name: string, disabled: boolean): Promise<{ ok: boolean; name: string; disabled: boolean }> {
  const resp = await fetch(D + '/skills/toggle', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ name, disabled }),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function toggleEvolve(name: string, auto_evolve: boolean): Promise<{ ok: boolean; name: string; auto_evolve: boolean }> {
  const resp = await fetch(D + '/skills/evolve-toggle', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ name, auto_evolve }),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

// ── Cron ───────────────────────────────────────────────

export interface CronJob {
  name: string;
  schedule: string;
  prompt: string;
  enabled: boolean;
  last_fired_at: number | null;
  last_response: string | null;
  cooldown_secs: number | null;
  trigger: string;
}

export async function fetchCronJobs(): Promise<CronJob[]> {
  const resp = await fetch(D + '/cron', { headers: authHeaders() });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.jobs || [];
}

// ── Vijnana ─────────────────────────────────────────────

export async function fetchVijnana(): Promise<VijnanaState> {
  const resp = await fetch(D + '/vijnana/state', { headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function fetchVijnanaSeeds(sessionId?: string): Promise<VijnanaSeedsResponse> {
  const params = sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : '';
  const resp = await fetch(`${D}/vijnana/seeds${params}`, { headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function manageCronJob(body: {
  action: string;
  name?: string;
  schedule?: string;
  prompt?: string;
  cooldown_secs?: number;
}): Promise<Record<string, unknown>> {
  const resp = await fetch(D + '/cron', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify(body),
  });
  return resp.json();
}

// ── Projects ────────────────────────────────────────────

export interface ProjectInfo {
  id: string;
  cwd: string;
  name: string;
  description: string;
  tags: string[];
  archived: boolean;
  createdAt: number;
  updatedAt: number;
  sessionCount: number;
}

export async function fetchProjects(filter?: string): Promise<ProjectInfo[]> {
  const qs = filter ? `?filter=${filter}` : '';
  const resp = await fetch(D + '/projects' + qs, { headers: authHeaders() });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.projects || [];
}

export async function createProject(name: string, cwd: string): Promise<{ id: string; cwd: string }> {
  const resp = await fetch(D + '/projects', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ name, cwd }),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function archiveProject(id: string): Promise<void> {
  await fetch(`${D}/projects/${encodeURIComponent(id)}/archive`, {
    method: 'POST',
    headers: authHeaders(),
  });
}

export async function fetchProject(id: string): Promise<any> {
  const resp = await fetch(`${D}/projects/${encodeURIComponent(id)}`, { headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function updateProject(id: string, data: { name?: string; description?: string; tags?: string[] }): Promise<any> {
  const resp = await fetch(`${D}/projects/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    headers: authHeaders(),
    body: JSON.stringify(data),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function unarchiveProject(id: string): Promise<void> {
  await fetch(`${D}/projects/${encodeURIComponent(id)}/unarchive`, {
    method: 'POST',
    headers: authHeaders(),
  });
}

// ── Monitor ────────────────────────────────────────────────

export interface MonitorData {
  context_window: { max_tokens: number };
  metrics: Record<string, unknown>;
  active_sessions: number;
}

export async function fetchMonitor(): Promise<MonitorData> {
  const resp = await fetch(D + '/monitor', { headers: authHeaders() });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

// ── Active Sessions ────────────────────────────────────────

export interface ActiveSession {
  id: string;
  provider: string;
  model: string;
  created_at: number;
}

export interface ActiveSessionsResponse {
  sessions: ActiveSession[];
}

export async function fetchActiveSessions(): Promise<ActiveSessionsResponse> {
  const resp = await fetch(D + '/sessions/active', { headers: authHeaders() });
  if (!resp.ok) return { sessions: [] };
  return resp.json();
}
