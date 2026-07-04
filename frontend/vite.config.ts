import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'

const DAEMON = 'http://127.0.0.1:3000'
// Token is now obtained by the frontend via POST /auth/session (localhost-gated),
// so the proxy no longer needs to inject an Authorization header. Removing it
// also avoids sending duplicate Authorization headers when the frontend has a token.
const proxyRoutes = [
  '/config', '/providers', '/sessions', '/agent', '/tools', '/skills', '/cron',
  '/monitor', '/vijnana', '/principles', '/files', '/events', '/confirm', '/projects',
  '/auth/session',
]

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  base: './',
  server: {
    proxy: Object.fromEntries(proxyRoutes.map(p => [p, { target: DAEMON }])),
  },
})
