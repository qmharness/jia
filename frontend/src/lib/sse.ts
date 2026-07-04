import { streamAgent, API_BASE, authHeaders } from './api';
import { store, showToast, updateLastToolResult, setPage } from './store.svelte';
import { uiStore } from './stores/ui.svelte';
import type { ChatEntry, SSEEvent, StreamAgentParams } from './types';

function createStreamingEntry(streamId: string): ChatEntry {
  const id = crypto.randomUUID();
  return {
    id,
    role: 'assistant' as const,
    get content() { return store.streamStates[streamId]?.text ?? ''; },
    _streamId: streamId,
    createdAt: Date.now(),
  } as any;
}

function freezeStreamingEntry(streamId: string) {
  const entries = store.entries;
  const idx = entries.findLastIndex((e: any) => e._streamId === streamId);
  if (idx >= 0) {
    const entry = entries[idx] as any;
    const { _streamId, ...rest } = entry;
    rest.content = store.streamStates[streamId]?.text ?? entry.content ?? '';
    const updated = [...entries];
    updated[idx] = rest;
    store.entries = updated;
  }
}

function finalizeStreamingEntry(streamId: string) {
  // Snapshot content before cleanup — entry.content is a reactive getter
  // that reads from streamStates, so we must capture it first.
  const entries = store.entries;
  const idx = entries.findLastIndex((e: any) => e._streamId === streamId);
  let snapshot = '';
  if (idx >= 0) {
    snapshot = (entries[idx] as any).content ?? '';
  }

  delete store.cancels[streamId];
  delete store.streamStates[streamId];
  delete store.streamSessions[streamId];

  if (idx >= 0) {
    const entry = entries[idx] as any;
    const { _streamId, ...rest } = entry;
    rest.content = snapshot;
    const updated = [...entries];
    updated[idx] = rest;
    store.entries = updated;
  }
}

export function sendMessage(params: StreamAgentParams) {
  const ac = new AbortController();
  const streamId = crypto.randomUUID();
  let sessionId = params.sessionId;
  const tempSessionId = params._tempSessionId;

  // Initialize stream state as queued
  store.streamStates[streamId] = { status: 'queued', text: '' };

  // Insert assistant placeholder bubble (queued)
  const streamEntry = createStreamingEntry(streamId);
  store.entries = [...store.entries, streamEntry];

  // ── RAF batching: accumulate deltas, flush once per frame ──
  let pendingText = '';
  let rafId: number | null = null;

  function flushDelta() {
    rafId = null;
    if (!pendingText) return;
    const st = store.streamStates[streamId];
    if (st) {
      if (st.status === 'queued') st.status = 'streaming';
      st.text += pendingText;
      pendingText = '';
      // Spread to new object to trigger Svelte proxy reactivity
      store.streamStates[streamId] = { ...st };
      store.scrollTick++;
    }
  }

  (async () => {
    try {
      const { reader, decoder } = await streamAgent(params, ac.signal);
      let buffer = '';

      // Only update UI if this stream belongs to the currently active session
      function isActiveSession(): boolean {
        if (!sessionId) return true; // before session event, always allow
        return sessionId === store.activeSessionId;
      }

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed.startsWith('data: ')) continue;
          const jsonStr = trimmed.slice(6);
          if (!jsonStr) continue;

          let event: SSEEvent;
          try { event = JSON.parse(jsonStr); } catch { continue; }

          switch (event.type) {
            case 'delta': {
              // Always process deltas — they only update streamStates, not entries
              pendingText += event.content;
              if (rafId === null) {
                rafId = requestAnimationFrame(flushDelta);
              }
              break;
            }
            case 'stream_end': {
              if (!isActiveSession()) continue;
              if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
              freezeStreamingEntry(streamId);
              break;
            }
            case 'tool_batch_start': {
              if (!isActiveSession()) continue;
              if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
              store.streamStates[streamId] = { status: 'streaming', text: '' };
              store.entries = [...store.entries, createStreamingEntry(streamId)];
              await new Promise(r => requestAnimationFrame(r));
              break;
            }
            case 'tool_call': {
              if (!isActiveSession()) continue;
              if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
              const entries = store.entries;
              const last = entries[entries.length - 1];
              store.entries = [...entries.slice(0, -1), {
                id: crypto.randomUUID(),
                role: 'tool_call' as const,
                tool: event.tool,
                input: event.input,
                status: 'running' as const,
                createdAt: Date.now(),
              }, last];
              break;
            }
            case 'tool_result': {
              if (!isActiveSession()) continue;
              updateLastToolResult(event.output, event.error, event.geju, event.execution_mode);
              break;
            }
            case 'confirm_request': {
              store.confirmState = {
                id: event.id,
                tool: event.tool,
                reason: event.reason,
                timeoutSecs: event.timeout_secs,
                token: event.token,
              };
              break;
            }
            case 'session': {
              sessionId = event.session_id;
              store.activeSessionId = event.session_id;
              uiStore.activeSessionId = event.session_id;
              store.streamSessions[streamId] = sessionId;
              // Replace temp session entry with real ID in both uiStore and sidebar
              if (tempSessionId) {
                uiStore.sessions = uiStore.sessions.map(s =>
                  s.id === tempSessionId ? { ...s, id: event.session_id } : s
                );
                window.dispatchEvent(new CustomEvent('jia:session-id-updated', {
                  detail: { tempId: tempSessionId, realId: event.session_id }
                }));
                // Navigate to session page now that we have the real ID
                window.location.hash = 'session/' + event.session_id;
              }
              // Track streaming state locally so sidebar shows green dot immediately
              uiStore.streamingSessionIds[event.session_id] = true;
              // Refresh sidebar so it picks up the "active" status from the API
              window.dispatchEvent(new CustomEvent('jia:refresh-sessions'));
              break;
            }
            case 'done': {
              if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
              const st = store.streamStates[streamId];
              if (st) st.status = 'done';
              finalizeStreamingEntry(streamId);
              if (sessionId) delete uiStore.streamingSessionIds[sessionId];
              break;
            }
            case 'error': {
              if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
              showToast(event.message, 'error');
              finalizeStreamingEntry(streamId);
              if (sessionId) delete uiStore.streamingSessionIds[sessionId];
              break;
            }
          }
        }
      }
      // Stream ended — flush any remaining pending text
      if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
    } catch (err: any) {
      if (rafId !== null) { cancelAnimationFrame(rafId); flushDelta(); }
      if (err.name !== 'AbortError') {
        showToast(err.message || 'Stream failed', 'error');
      }
      finalizeStreamingEntry(streamId);
      if (sessionId) delete uiStore.streamingSessionIds[sessionId];
    }
  })();

  // Register per-stream cancel
  store.cancels[streamId] = () => {
    ac.abort();
    if (sessionId) {
      fetch(API_BASE + '/agent/cancel', {
        method: 'POST',
        headers: authHeaders(),
        body: JSON.stringify({ session_id: sessionId }),
      }).catch(() => {});
    }
  };
}
