import { makePersistedAdapter } from '@livestore/adapter-web'
import LiveStoreSharedWorker from '@livestore/adapter-web/shared-worker?sharedworker'
import { LiveStoreProvider } from '@livestore/react'
import { StrictMode, useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { unstable_batchedUpdates as batchUpdates } from 'react-dom'
import { App } from './App'
import { api } from './bridge/api'
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
      gap: 16,
    }}
  >
    <span className="phi phi-hero" style={{ fontSize: '5rem' }}>
      φ
    </span>
    <span className="mono" style={{ color: 'var(--muted)' }}>
      loading… ({stage})
    </span>
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
