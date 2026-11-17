import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig(() => {
  const env = loadEnv('development', fileURLToPath(new URL('../..', import.meta.url)), '')
  const api = `http://${env.FERRIX_BIND || '127.0.0.1:8080'}`
  return {
  plugins: [react(), tailwindcss()],
  resolve: { alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) } },
  server: { port: Number(env.FERRIX_WEB_PORT || 5173), strictPort: true, proxy: { '/api': api, '/health': api } },
  }
})
