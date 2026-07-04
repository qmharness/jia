import type { ChatEntry, ToolCallEntry, Message } from './types';

export function stripToolCalls(text: unknown): string {
  if (typeof text !== 'string') return '';
  return text.replace(/<tool_call>[\s\S]*?<\/tool_call>/g, '').trim();
}

export function isToolResultMsg(content: unknown): boolean {
  if (typeof content !== 'string') return false;
  return /^Tool \S+ (result|error):/.test(content);
}

/** Parse backend HistoryEntry[] JSON into frontend ChatEntry[], with optional createdAt timestamps. */
export function parseEntries(data: { entries: Array<Record<string, unknown>> }, opts?: { createdAt?: boolean }): ChatEntry[] {
  const results: ChatEntry[] = [];
  for (const item of data.entries) {
    if (item.role === 'tool_call' || item.tool) {
      results.push({
        id: crypto.randomUUID(),
        role: 'tool_call',
        tool: item.tool as string,
        input: item.input,
        status: (item.status as string) || (item.error ? 'error' : 'success'),
        output: item.output as string | undefined,
        error: item.error as string | undefined,
        geju: item.geju as string | undefined,
        executionMode: item.executionMode as ToolCallEntry['executionMode'] | undefined,
        ...(opts?.createdAt ? { createdAt: Date.now() } : {}),
      } as ChatEntry);
    } else if (item.role) {
      if (item.role === 'user' && isToolResultMsg(item.content)) continue;
      const content = item.role === 'assistant' ? stripToolCalls(item.content) : item.content;
      if (typeof content === 'string' && content.trim().length === 0) continue;
      results.push({
        id: crypto.randomUUID(),
        role: item.role as ChatEntry['role'],
        content: content as string,
        images: item.images as Message['images'] | undefined,
        ...(opts?.createdAt ? { createdAt: Date.now() } : {}),
      } as ChatEntry);
    }
  }
  return results;
}
