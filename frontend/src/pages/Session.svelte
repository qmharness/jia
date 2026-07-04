<script lang="ts">
  import { store } from '../lib/store.svelte';
  import ChatHeader from '../components/ChatHeader.svelte';
  import ChatBody from '../components/ChatBody.svelte';
  import InputArea from '../components/InputArea.svelte';
</script>

<div class="session-page">
  <div class="session-main">
    <ChatHeader />
    <ChatBody />
    <InputArea />
  </div>
  <aside class="right-panel">
  </aside>
</div>

<style>
  .session-page { display: flex; height: 100%; overflow: hidden; }
  .session-main { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .right-panel { width: 240px; min-width: 200px; flex-shrink: 0; border-left: 1px solid var(--border); background: var(--bg-secondary); overflow-y: auto; }
</style>
