import { makePersistedAdapter } from '@livestore/adapter-web'
import LiveStoreSharedWorker from '@livestore/adapter-web/shared-worker?sharedworker'
import { LiveStoreProvider } from '@livestore/react'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { unstable_batchedUpdates as batchUpdates } from 'react-dom'
import { App } from './App'
import './design/tokens.css'
import LiveStoreWorker from './livestore/livestore.worker?worker'
import { schema } from './livestore/schema'

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
      carregando… ({stage})
    </span>
  </div>
)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <LiveStoreProvider
      schema={schema}
      adapter={adapter}
      batchUpdates={batchUpdates}
      storeId="phai"
      renderLoading={(status) => <Loading stage={status.stage} />}
    >
      <App />
    </LiveStoreProvider>
  </StrictMode>,
)
