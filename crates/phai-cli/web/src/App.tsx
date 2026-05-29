import { useClientDocument } from '@livestore/react'
import { tables } from './livestore/schema'
import { useBridgeSync } from './bridge/sync'
import { DndProvider } from './lib/dnd'
import { Planning } from './views/Planning'
import { Review } from './views/Review'

type View = 'review' | 'planning'

const VIEWS: { id: View; label: string }[] = [
  { id: 'review', label: 'Revisão' },
  { id: 'planning', label: 'Planejamento' },
]

/**
 * App shell — a full-width responsive workspace (DESIGN.md "Layout"). The shell
 * caps at `min(1680px, 96vw)` with 24–32px gutters; the views own their own
 * responsive grids. Two views: Revisão (the transaction list + live-sum filters)
 * and Planejamento (the cash-evolution chart spine + the selected month's plan,
 * with drag-and-drop forecast re-dating).
 */
export const App = () => {
  const [{ view }, setUi] = useClientDocument(tables.ui)
  const sync = useBridgeSync()

  return (
    <div
      style={{
        maxWidth: 'var(--container)',
        margin: '0 auto',
        padding: '0 clamp(24px, 3vw, 32px)',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 16,
          padding: '28px 0 20px',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <span className="phi" style={{ fontSize: '2rem' }}>
          φ
        </span>
        <strong
          style={{ fontFamily: 'var(--font-display)', fontSize: '1.4rem', letterSpacing: '-0.02em' }}
        >
          phai
        </strong>
        <nav style={{ display: 'flex', gap: 8, marginLeft: 'auto' }}>
          {VIEWS.map((v) => (
            <button
              key={v.id}
              onClick={() => setUi({ view: v.id })}
              className="mono"
              style={{
                background: view === v.id ? 'rgba(109,74,255,0.08)' : 'transparent',
                color: view === v.id ? 'var(--purple)' : 'var(--muted)',
                border: `1px solid ${view === v.id ? 'rgba(109,74,255,0.25)' : 'var(--border)'}`,
                borderRadius: 'var(--radius-full)',
                padding: '6px 18px',
                cursor: 'pointer',
                fontSize: 13,
                transition: 'border-color 150ms, color 150ms',
              }}
            >
              {v.label}
            </button>
          ))}
        </nav>
      </header>

      <SyncChip pending={sync.pending} error={sync.error} />

      <DndProvider>
        <main style={{ padding: '20px 0 80px' }}>
          {view === 'review' && <Review />}
          {view === 'planning' && <Planning />}
        </main>
      </DndProvider>
    </div>
  )
}

const SyncChip = ({ pending, error }: { pending: number; error: string | null }) => {
  const color = error ? 'var(--rose)' : pending > 0 ? 'var(--amber)' : 'var(--green)'
  const label = error
    ? `sync com erro — ${error}`
    : pending > 0
      ? `${pending} pendente${pending === 1 ? '' : 's'} de sync`
      : 'sincronizado'
  return (
    <div
      className="mono"
      style={{ fontSize: 12, color, padding: '12px 0', display: 'flex', alignItems: 'center', gap: 8 }}
    >
      <span style={{ width: 7, height: 7, borderRadius: '50%', background: color }} />
      {label}
    </div>
  )
}
