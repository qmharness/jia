import type { ChatEntry } from '../types';

export const chatStore = $state({
  entries: [] as ChatEntry[],
  streamStates: {} as Record<string, { status: 'queued' | 'streaming' | 'done'; text: string }>,
  cancels: {} as Record<string, () => void>,
  streamSessions: {} as Record<string, string>,
  scrollTick: 0,
  searchQuery: '',
  loadingMessages: false,
});

export function resetChatSession() {
  chatStore.entries = [];
  chatStore.streamStates = {};
  chatStore.cancels = {};
  chatStore.scrollTick = 0;
  chatStore.searchQuery = '';
  chatStore.loadingMessages = false;
}

export function updateLastToolResult(
  output: string | null,
  error: string | null,
  geju: string | null,
  executionMode: string | null
) {
  const entries = chatStore.entries;
  let idx = -1;
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i] as any;
    if (e.role === 'tool_call' && e.status === 'running') { idx = i; break; }
  }
  if (idx < 0) return;
  const tc = { ...entries[idx] } as any;
  tc.status = error ? 'error' : 'success';
  if (output !== null) tc.output = output;
  if (error !== null) tc.error = error;
  if (geju !== null) tc.geju = geju;
  if (executionMode !== null) tc.executionMode = executionMode;
  const updated = [...entries];
  updated[idx] = tc;
  chatStore.entries = updated;
}
