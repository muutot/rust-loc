import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// The built page is embedded in the Rust binary and served from its root, so
// every asset reference has to stay relative.
export default defineConfig({
  plugins: [react()],
  base: './',
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:8422',
    },
  },
})
