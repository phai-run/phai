import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Built bundle is embedded into the `phai` binary and served from `phai serve`
// at the site root. Relative asset URLs keep it mount-point agnostic.
export default defineConfig({
  base: './',
  worker: { format: 'es' },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
  plugins: [react()],
})
