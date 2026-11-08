import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const api = 'http://127.0.0.1:8191'

const proxy = {
  '/api': api,
  '/mcp': api,
  '/healthz': api,
  // Regex keys so /favicon… or /usr… never hit the API.
  '^/f/': api,
  '^/u/': api,
}

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5191,
    strictPort: true,
    proxy,
  },
  preview: {
    port: 5191,
    strictPort: true,
    proxy,
  },
})
