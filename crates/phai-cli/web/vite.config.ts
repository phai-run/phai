import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// Built bundle is embedded into the `phai` binary and served from `phai serve`
// at the site root. Relative asset URLs keep it mount-point agnostic.
// Cross-origin isolation headers — required for OPFS + SharedWorker (LiveStore)
// during local `vite dev` / `vite preview`. Production is served by `phai serve`
// with its own COEP-credentialless headers (this only affects the dev server).
const crossOriginIsolation = {
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Embedder-Policy': 'credentialless',
}

export default defineConfig({
  base: './',
  worker: { format: 'es' },
  server: { headers: crossOriginIsolation },
  preview: { headers: crossOriginIsolation },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
  plugins: [react()],
})
