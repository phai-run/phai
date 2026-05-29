import { useClientDocument } from '@livestore/react'
import { tables } from './livestore/schema'
import { useBridgeSync } from './bridge/sync'
import { Cashflow } from './views/Cashflow'
import { Forecasts } from './views/Forecasts'
import { Review } from './views/Review'

type Tab = 'review' | 'cashflow' | 'forecasts'

const TABS: { id: Tab; label: string }[] = [
  { id: 'review', label: 'Revisão' },
  { id: 'cashflow', label: 'Caixa' },
  { id: 'forecasts', label: 'Previsões' },
]

export const App = () => {
  const [{ tab }, setUi] = useClientDocument(tables.ui)
  const sync = useBridgeSync()

  return (
    <div style={{ maxWidth: 'var(--container)', margin: '0 auto', padding: '0 24px' }}>
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
          {TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setUi({ tab: t.id })}
              className="mono"
              style={{
                background: tab === t.id ? 'rgba(167,139,250,0.08)' : 'transparent',
                color: tab === t.id ? 'var(--purple)' : 'var(--muted)',
                border: `1px solid ${tab === t.id ? 'rgba(167,139,250,0.2)' : 'var(--border)'}`,
                borderRadius: 'var(--radius-full)',
                padding: '6px 18px',
                cursor: 'pointer',
                fontSize: 13,
              }}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      <SyncChip pending={sync.pending} error={sync.error} />

      <main style={{ padding: '24px 0 80px' }}>
        {tab === 'review' && <Review />}
        {tab === 'cashflow' && <Cashflow />}
        {tab === 'forecasts' && <Forecasts />}
      </main>
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
