<script lang="ts">
  import { onMount, onDestroy } from 'svelte';

  interface SpiritEvent {
    type: string;
    timestamp: number;
  }

  let events = $state<SpiritEvent[]>([]);
  let connected = $state(false);
  let eventSource: EventSource | null = null;
  let expanded = $state<Record<string, boolean>>({});

  const spiritNames: Record<string, string> = {
    certainty_trace: '太阴 · 确定度',
    seed_dynamics: '太阴 · 种子激活',
    behavioral_alert: '白虎 · 异常告警',
    memory_loss: '玄武 · 记忆损失',
    strategy_insight: '九天 · 策略涌现',
    stability_transition: '九地 · 稳定性',
    turn_start: '值符 · 轮次开始',
    turn_end: '值符 · 轮次结束',
    llm_usage: '螣蛇 · LLM用量',
    tool_call: '值符 · 工具调用',
    tool_result: '值符 · 工具结果',
    geju_result: '值符 · 格局',
    cron_notification: '六合 · 定时任务',
    session_end: '六合 · 会话结束',
  };

  onMount(() => {
    connect();
    return () => eventSource?.close();
  });

  function connect() {
    const url = `${window.location.origin}/events`;
    eventSource = new EventSource(url);
    eventSource.onopen = () => connected = true;
    eventSource.onerror = () => connected = false;
    eventSource.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data);
        events = [...events.slice(-99), { type: data.type, timestamp: Date.now() }];
      } catch {}
    };
  }

  function toggle(id: string) {
    expanded = { ...expanded, [id]: !expanded[id] };
  }

  // Group by spirit
  let grouped = $derived.by(() => {
    const groups: Record<string, SpiritEvent[]> = {};
    for (const ev of events) {
      const spirit = spiritNames[ev.type] || ev.type;
      if (!groups[spirit]) groups[spirit] = [];
      groups[spirit].push(ev);
    }
    return groups;
  });
</script>

<div class="observer">
  <div class="status-bar">
    <span class="dot" class:live={connected}></span>
    {connected ? 'SSE 连接中' : 'SSE 断开'} · {events.length} 事件
  </div>

  {#if Object.keys(grouped).length === 0}
    <div class="empty">等待八神观测事件…</div>
  {:else}
    <div class="groups">
      {#each Object.entries(grouped) as [spirit, evs]}
        <div class="group" onclick={() => toggle(spirit)}>
          <div class="group-header">
            <span class="spirit-name">{spirit}</span>
            <span class="count">{evs.length}</span>
          </div>
          {#if expanded[spirit]}
            <div class="events">
              {#each evs.slice(-5).reverse() as ev}
                <div class="event-row">
                  <code>{ev.type}</code>
                  <span class="time">{new Date(ev.timestamp).toLocaleTimeString()}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .observer { height: 100%; display: flex; flex-direction: column; }
  .status-bar {
    padding: 8px 16px; font-size: 12px; color: var(--text-secondary);
    border-bottom: 1px solid var(--border); display: flex; align-items: center; gap: 8px;
  }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--text-secondary); }
  .dot.live { background: #22c55e; }
  .empty { padding: 40px; text-align: center; color: var(--text-secondary); font-size: 14px; }
  .groups { flex: 1; overflow-y: auto; }
  .group { border-bottom: 1px solid var(--border); cursor: pointer; }
  .group-header {
    display: flex; justify-content: space-between; align-items: center;
    padding: 10px 16px; font-size: 13px;
  }
  .group:hover { background: var(--hover); }
  .spirit-name { font-weight: 500; }
  .count { font-size: 11px; color: var(--text-secondary); background: var(--bg-secondary); padding: 2px 8px; border-radius: 10px; }
  .events { padding: 4px 16px 10px; }
  .event-row { display: flex; justify-content: space-between; padding: 3px 0; font-size: 11px; }
  .event-row code { font-family: monospace; color: var(--accent); }
  .time { color: var(--text-secondary); }
</style>
