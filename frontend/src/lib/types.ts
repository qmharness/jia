// ── Provider ──────────────────────────────────────────────

export interface Provider {
  name: string;
  kind: string;
  models: string[];
  default_model: string;
}

// ── Messages ──────────────────────────────────────────────

export type Role = 'user' | 'assistant' | 'system';

export interface ImageContent {
  data: string;
  media_type: string;
}

export interface Message {
  id: string;
  role: Role;
  content: string;
  images?: ImageContent[];
  createdAt?: number;
}

// 流式助手条目:进行中的流携带 _streamId(content 可能是读 store.streamStates 的响应式 getter)。
// 完成的消息不带 _streamId。
export type StreamingMessage = Message & { _streamId?: string };

// ── Tool Cards ────────────────────────────────────────────

export type ExecutionMode = 'direct' | 'guarded' | 'sandbox' | 'denied';
export type ToolStatus = 'running' | 'success' | 'error';

export interface ToolCallEntry {
  id: string;
  role: 'tool_call';
  tool: string;
  input: unknown;
  status: ToolStatus;
  output?: string;
  error?: string;
  geju?: string;
  executionMode?: ExecutionMode;
  createdAt?: number;
}

export type ChatEntry = Message | ToolCallEntry;

// ── SSE Events ────────────────────────────────────────────

export type SSEEvent =
  | { type: 'session'; session_id: string }
  | { type: 'delta'; content: string }
  | { type: 'stream_end' }
  | { type: 'tool_batch_start' }
  | { type: 'tool_call'; tool: string; input: unknown }
  | { type: 'tool_result'; tool: string; output: string | null; error: string | null; geju: string | null; execution_mode: string | null }
  | { type: 'confirm_request'; id: string; tool: string; reason: string; timeout_secs: number; token: string }
  | { type: 'done' }
  | { type: 'error'; message: string }
  | { type: 'cron_notification'; job_name: string; prompt: string; response: string; timestamp: number };

// ── Confirm Dialog ────────────────────────────────────────

export interface ConfirmState {
  id: string;
  tool: string;
  reason: string;
  timeoutSecs: number;
  token: string;
}

// ── Sessions ──────────────────────────────────────────────

export interface SessionMeta {
  id: string;
  title: string;
  messageCount: number;
  updatedAt: number;
  cwd?: string;
  projectId?: string;
  projectName?: string;
  status?: 'active' | 'error' | 'idle';
  pinned?: boolean;
}

// ── Files ─────────────────────────────────────────────────

export interface FileNode {
  name: string;
  path: string;
  isDir: boolean;
  children?: FileNode[];
}

// ── Navigation ────────────────────────────────────────────

export type PageId = 'chat' | 'session' | 'sessions' | 'projects' | 'project' | 'tools' | 'skills' | 'cron' | 'monitor' | 'settings' | 'vijnana';

// ── Vijnana ────────────────────────────────────────────────

export interface VijnanaState {
  manas: VijnanaManas;
  manas_history: ManasSnapshot[];
  entropy: VijnanaEntropy;
}

export interface ManasSnapshot {
  atma_graha: number;
  entropy_total: number;
  seed_count: number;
  created_at: number;
}

export interface VijnanaManas {
  atma_graha: number;
  total_turns: number;
  consolidation_count: number;
  stable_pattern_count: number;
  last_consolidation_at: number;
  stable_epochs: number;
  is_stable: boolean;
  total_seeds: number;
}

export interface VijnanaEntropyDim {
  staleness: number;
  contradiction: number;
  redundancy: number;
  access_decay: number;
  total: number;
}

export interface SeedDigest {
  nature: string;
  source: string;
  primary_dim: string;
}

export interface DissolutionEvent {
  timestamp: number;
  examined: number;
  dissolved: number;
  weakened: number;
  entropy_before: number;
  entropy_after: number;
  kept: number;
  protected: number;
  dissolved_sample: SeedDigest[];
}

export interface VijnanaEntropy {
  current: VijnanaEntropyDim;
  dissolution_history: DissolutionEvent[];
}

export interface VijnanaSeed {
  id: string;
  nature: string;
  source: string;
  content: { type: string } & Record<string, string>;
  palace: string;
  intent_stem: string;
  geju_key: string;
  strength: number;
  created_at: number;
}

export interface VijnanaSeedsResponse {
  seeds: VijnanaSeed[];
  count: number;
}

// ── Stream Agent Params ───────────────────────────────────

export interface StreamAgentParams {
  provider: string;
  model?: string;
  auxProvider?: string;
  auxModel?: string;
  messages: Array<{
    role: string;
    content: string;
    images?: Array<{ data: string; media_type: string }>;
  }>;
  sessionId: string | null;
  cwd?: string;
  projectId?: string;
  _tempSessionId?: string;
}
