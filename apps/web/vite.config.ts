import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import { localOriginRedirect } from './src/lib/dev-origin'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig(() => {
  const env = loadEnv('development', fileURLToPath(new URL('../..', import.meta.url)), '')
  const api = `http://${env.FERRIX_BIND || '127.0.0.1:8080'}`
  return {
  plugins: [{
    name: 'ferrix-local-auth-origin',
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const location = localOriginRedirect(env.FERRIX_AUTH_ORIGIN ?? '', req.headers.host, req.method, req.headers.accept, req.url)
        if (!location) { next(); return }
        res.writeHead(307, { Location: location, 'Cache-Control': 'no-store' })
        res.end()
      })
    },
  }, react(), tailwindcss()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  build: {
    manifest: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes('node_modules')) return
          if (id.includes('@tanstack/')) return 'router'
          if (id.includes('radix-ui') || id.includes('@radix-ui/') || id.includes('@base-ui/')) return 'ui'
          if (id.includes('/react/') || id.includes('/react-dom/') || id.includes('/scheduler/')) return 'react'
        },
      },
    },
  },
  server: { port: Number(env.FERRIX_WEB_PORT || 5173), strictPort: true, proxy: { '/api/': { target: api, ws: true, rewriteWsOrigin: false }, '/health': api } },
  }
})
