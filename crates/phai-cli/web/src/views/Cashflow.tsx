import { queryDb } from '@livestore/livestore'
import { useQuery, useClientDocument } from '@livestore/react'
import { tables } from '../livestore/schema'
import { useChartSeed } from '../bridge/sync'
import {
  Card,
  EmptyState,
  ErrorNote,
  FilterBar,
  Label,
  LoadingNote,
  Pill,
  ViewHeader,
} from '../components/ui'
import { CashflowChart } from './CashflowChart'

const ACCENT = 'var(--cyan)'

const chart$ = queryDb(tables.chartMonths.orderBy('ordinal', 'asc'))

const RANGE = [3, 6, 12, 18, 24]

/**
 * Cashflow view — the cash-evolution chart (replaces the old Chart.js dashboard
 * tab). Per-month inflows/outflows with a projected closing-balance line;
 * future months render dashed. Controls drive a bridge re-seed.
 */
export const Cashflow = () => {
  const [ui, setUi] = useClientDocument(tables.ui)
  const months = useQuery(chart$)
  const seed = useChartSeed(ui.monthsBack, ui.monthsAhead)

  return (
    <div>
      <ViewHeader title="Caixa" accent={ACCENT} />

      <FilterBar>
        <Label>histórico</Label>
        {RANGE.map((r) => (
          <Pill
            key={`back-${r}`}
            accent={ACCENT}
            active={ui.monthsBack === r}
            onClick={() => setUi({ monthsBack: r })}
          >
            {r}m
          </Pill>
        ))}
        <span style={{ width: 12 }} />
        <Label>projeção</Label>
        {RANGE.map((r) => (
          <Pill
            key={`ahead-${r}`}
            accent={ACCENT}
            active={ui.monthsAhead === r}
            onClick={() => setUi({ monthsAhead: r })}
          >
            {r}m
          </Pill>
        ))}
        <Pill accent={ACCENT} onClick={() => seed.reload()}>
          ↻ atualizar
        </Pill>
      </FilterBar>

      {seed.error && <ErrorNote error={seed.error} />}
      {seed.loading && months.length === 0 && <LoadingNote />}

      {months.length === 0 && !seed.loading ? (
        <EmptyState message="Sem dados de caixa." />
      ) : (
        <Card accent={ACCENT} style={{ padding: 24 }}>
          <CashflowChart months={months} />
        </Card>
      )}
    </div>
  )
}
