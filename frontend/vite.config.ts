import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'

const DAEMON = 'http://127.0.0.1:3000'
// 浏览器直连 vite(无 Tauri init script 注入 token)时,由代理补上 daemon 的 api_key,否则 401。仅 dev 用。
const authHeaders = { Authorization: 'Bearer jia-dev-token' }
const proxyRoutes = ['/config','/providers','/sessions','/agent','/tools','/skills','/cron','/monitor','/vijnana','/files','/events','/confirm','/projects']

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  base: './',
  server: {
    proxy: Object.fromEntries(proxyRoutes.map(p => [p, { target: DAEMON, headers: authHeaders }])),
  },
})
