import { makePersistedAdapter } from '@livestore/adapter-web'
import LiveStoreSharedWorker from '@livestore/adapter-web/shared-worker?sharedworker'
import { LiveStoreProvider } from '@livestore/react'
import { StrictMode, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { unstable_batchedUpdates as batchUpdates } from 'react-dom'
import { App } from './App'
import { api } from './bridge/api'
// Self-hosted fonts (bundled woff2, same-origin). Required so the strict
// cross-origin-isolation policy (COEP: require-corp) the desktop shell needs
// does not have to reach fonts.gstatic.com. See ADR-0039 + design/tokens.css.
import '@fontsource/inter/400.css'
import '@fontsource/inter/500.css'
import '@fontsource/inter/600.css'
import '@fontsource/jetbrains-mono/400.css'
import '@fontsource/space-grotesk/500.css'
import '@fontsource/space-grotesk/700.css'
import './design/tokens.css'
import LiveStoreWorker from './livestore/livestore.worker?worker'
import { schema, STORE_ID } from './livestore/schema'
import { Onboarding } from './views/Onboarding'

const adapter = makePersistedAdapter({
  storage: { type: 'opfs' },
  worker: LiveStoreWorker,
  sharedWorker: LiveStoreSharedWorker,
})

const Loading = ({ stage }: { stage: string }) => (
  <div
    style={{
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      justifyContent: 'center',
      minHeight: '100vh',
      gap: 24,
      background: 'var(--bg)',
    }}
  >
    {/* Glow halo + orbital particles + φ symbol */}
    <div
      style={{
        position: 'relative',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: 160,
        height: 160,
      }}
    >
      {/* Glow disc behind the φ */}
      <div
        className="loading-glow"
        style={{
          position: 'absolute',
          width: 100,
          height: 100,
          top: '50%',
          left: '50%',
          transform: 'translate(-50%, -50%)',
        }}
      />
      {/* Orbital particles */}
      <div
        style={{
          position: 'absolute',
          width: 0,
          height: 0,
          top: '50%',
          left: '50%',
        }}
      >
        <span className="loading-particle" />
        <span className="loading-particle" />
        <span className="loading-particle" />
      </div>
      {/* The φ with animated gradient */}
      <span
        className="phi phi-loading"
        style={{
          fontSize: '5rem',
          position: 'relative',
          zIndex: 1,
          lineHeight: 1,
        }}
      >
        φ
      </span>
    </div>
    {/* Stage text */}
    <span
      className="mono loading-stage"
      style={{ color: 'var(--muted)', fontSize: '0.8rem', letterSpacing: '0.05em' }}
    >
      {stage}
    </span>
    {/* Indeterminate progress bar */}
    <div className="loading-progress" />
  </div>
)

/** The full app once the machine is activated: boot LiveStore, render App. */
const Ready = () => (
  <LiveStoreProvider
    schema={schema}
    adapter={adapter}
    batchUpdates={batchUpdates}
    // Versioned in livestore/schema.ts (STORE_VERSION) — see the comment there.
    storeId={STORE_ID}
    renderLoading={(status) => <Loading stage={status.stage} />}
    renderError={(error) => (
      <div
        className="mono"
        style={{ padding: 32, color: 'var(--rose)', whiteSpace: 'pre-wrap', maxWidth: 900, margin: '0 auto' }}
      >
        <strong>Erro ao iniciar o LiveStore</strong>
        {'\n\n'}
        {String((error as { message?: string })?.message ?? error)}
      </div>
    )}
  >
    <App />
  </LiveStoreProvider>
)

/**
 * Root gate. Asks the bridge whether this machine is activated yet. A fresh
 * install shows onboarding (attach key + passphrase); an activated machine boots
 * straight into the dashboard. If the bridge can't be reached we fail open to
 * the app so `vite dev` and offline reloads still work.
 */
const Root = () => {
  const [phase, setPhase] = useState<'loading' | 'onboarding' | 'ready'>('loading')

  useEffect(() => {
    let cancelled = false
    api
      .status()
      .then((s) => {
        if (!cancelled) setPhase(s.activated ? 'ready' : 'onboarding')
      })
      .catch(() => {
        if (!cancelled) setPhase('ready')
      })
    return () => {
      cancelled = true
    }
  }, [])

  if (phase === 'loading') return <Loading stage="status" />
  if (phase === 'onboarding')
    // Full reload: reinitialise LiveStore/OPFS against the freshly activated
    // BigQuery backend (the bridge identity changes local→bigquery, so sync
    // reseeds from scratch). Bulletproof for a non-technical first run.
    return <Onboarding onActivated={() => window.location.reload()} />
  return <Ready />
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Root />
  </StrictMode>,
)
