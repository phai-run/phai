import { queryDb } from '@livestore/livestore'
import { useStore, useQuery, useClientDocument } from '@livestore/react'
import { useMemo, useState } from 'react'
import { events, tables } from '../livestore/schema'
import { api } from '../bridge/api'
import { useChartSeed, useForecastsSeed, useTransactionsSeed } from '../bridge/sync'
import { amountColor, formatMoney, formatMoneyNumber, isNegative, sumAmounts } from '../lib/format'
import { useDnd } from '../lib/dnd'
import {
  Card,
  EmptyState,
  ErrorNote,
  FilterBar,
  Label,
  LoadingNote,
  Pill,
  TextInput,
  ViewHeader,
} from '../components/ui'
import { PlanningChart, type ChartMonthView } from './PlanningChart'

const ACCENT = 'var(--cyan)'

const chart$ = queryDb(tables.chartMonths.orderBy('ordinal', 'asc'))
const forecasts$ = queryDb(tables.forecasts.orderBy('dueDate', 'asc'))
const forecastOverlay$ = queryDb(tables.forecastOverlay)
const txAll$ = queryDb(tables.transactions.orderBy('postedAt', 'desc'))

const RANGE = [3, 6, 12, 18, 24]

const monthOf = (date: string | null): string | null => (date ? date.slice(0, 7) : null)

/** A forecast as the view sees it: overlay-redated `dueDate`, derived `month`. */
export interface ForecastView {
  forecastId: string
  dueDate: string | null
  description: string
  amount: string
  categoryId: string | null
  accountId: string | null
  status: string
  kind: string
  draggable: number
  month: string | null
}

interface TxLite {
  id: string
  postedAt: string
  amount: string
  description: string | null
  merchantName: string | null
  rawDescription: string
  categoryId: string | null
  month: string
}

/**
 * Planejamento — Caixa + Previsões unified. The cash-evolution bar chart is the
 * spine: clicking a month's bar selects it (pure client state, instant), and the
 * panel below shows that month's transactions and forecasts with inflow/outflow/
 * projected-close totals. Manual forecasts are draggable between months; dropping
 * commits an optimistic `forecastMoved` (re-dates locally, bars + totals recompute
 * in the same frame) and queues a background flush to /api/forecast/move.
 */
export const Planning = () => {
  const { store } = useStore()
  const [ui, setUi] = useClientDocument(tables.ui)
  const chartRows = useQuery(chart$)
  const forecastsRaw = useQuery(forecasts$)
  const overlay = useQuery(forecastOverlay$)
  const txRows = useQuery(txAll$) as ReadonlyArray<TxLite>

  // Window drives both the chart and the transaction window (shared with Revisão).
  const chartSeed = useChartSeed(ui.monthsBack, ui.monthsAhead)
  const forecastSeed = useForecastsSeed(null)
  useTransactionsSeed(ui.monthsBack, ui.monthsAhead)

  // Apply the optimistic re-dating overlay over seeded forecasts.
  const overlayById = useMemo(
    () => new Map(overlay.map((o) => [o.forecastId, o.dueDate])),
    [overlay],
  )
  const forecasts: ForecastView[] = useMemo(
    () =>
      forecastsRaw.map((f) => {
        const dueDate = overlayById.has(f.forecastId)
          ? (overlayById.get(f.forecastId) ?? f.dueDate)
          : f.dueDate
        return { ...f, dueDate, month: monthOf(dueDate) }
      }),
    [forecastsRaw, overlayById],
  )

  // Group forecasts by month for the bars' popover + the panel.
  const forecastsByMonth = useMemo(() => {
    const map = new Map<string, ForecastView[]>()
    for (const f of forecasts) {
      if (!f.month) continue
      const list = map.get(f.month) ?? []
      list.push(f)
      map.set(f.month, list)
    }
    return map
  }, [forecasts])

  const months: ReadonlyArray<ChartMonthView> = chartRows
  const selected = ui.selectedMonth ?? months[months.length - 1]?.month ?? null

  const moveForecast = (forecastId: string, targetMonth: string) => {
    const f = forecasts.find((x) => x.forecastId === forecastId)
    if (!f || !f.draggable) return
    // Preserve the day-of-month; default to the 1st if unknown.
    const day = f.dueDate ? f.dueDate.slice(8, 10) || '01' : '01'
    const dueDate = `${targetMonth}-${day}`
    if (dueDate === f.dueDate) return
    store.commit(
      events.forecastMoved({
        writeId: crypto.randomUUID(),
        forecastId,
        dueDate,
        movedAt: Date.now(),
      }),
    )
  }

  const error = chartSeed.error ?? forecastSeed.error
  const loading = chartSeed.loading && months.length === 0

  return (
    <div>
      <ViewHeader title="Planejamento" accent={ACCENT} />

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
        <Pill
          accent={ACCENT}
          onClick={() => {
            chartSeed.reload()
            forecastSeed.reload()
          }}
        >
          ↻ atualizar
        </Pill>
      </FilterBar>

      {error && <ErrorNote error={error} />}
      {loading && <LoadingNote />}

      {months.length === 0 && !loading ? (
        <EmptyState message="Sem dados de caixa." />
      ) : (
        <div className="planning-grid" style={{ display: 'grid', gap: 24, alignItems: 'start' }}>
          <Card accent={ACCENT} style={{ padding: 24, minWidth: 0 }}>
            <PlanningChart
              months={months}
              forecastsByMonth={forecastsByMonth}
              selectedMonth={selected}
              onSelectMonth={(m) => setUi({ selectedMonth: m })}
              onDropForecast={moveForecast}
            />
          </Card>

          <aside style={{ minWidth: 0 }}>
            <MonthPanel
              month={selected}
              chart={months.find((m) => m.month === selected) ?? null}
              forecasts={selected ? (forecastsByMonth.get(selected) ?? []) : []}
              transactions={
                selected ? txRows.filter((t) => t.month === selected) : []
              }
              onForecastAdded={() => forecastSeed.reload()}
            />
          </aside>
        </div>
      )}

      <style>{planningGridCss}</style>
    </div>
  )
}

const planningGridCss = `
.planning-grid { grid-template-columns: 1fr; }
@media (min-width: 1100px) {
  .planning-grid { grid-template-columns: minmax(0, 1.6fr) clamp(320px, 32vw, 460px); }
}
`

const MonthPanel = ({
  month,
  chart,
  forecasts,
  transactions,
  onForecastAdded,
}: {
  month: string | null
  chart: ChartMonthView | null
  forecasts: ForecastView[]
  transactions: ReadonlyArray<TxLite>
  onForecastAdded: () => void
}) => {
  if (!month) return <EmptyState message="Clique em um mês no gráfico." />

  const realizedIn = sumAmounts(transactions.filter((t) => !isNegative(t.amount)).map((t) => t.amount))
  const realizedOut = Math.abs(
    sumAmounts(transactions.filter((t) => isNegative(t.amount)).map((t) => t.amount)),
  )
  const projectedClose = chart
    ? chart.isFuture
      ? Number(chart.projectedClosingBalance)
      : Number(chart.closingBalance)
    : 0

  return (
    <Card accent={ACCENT} style={{ position: 'sticky', top: 16 }}>
      <h3 style={{ fontFamily: 'var(--font-display)', fontSize: '1.15rem', margin: '0 0 4px' }}>
        {month}
        {chart?.isFuture ? (
          <span className="mono" style={{ color: 'var(--muted)', fontSize: 12, marginLeft: 8 }}>
            previsto
          </span>
        ) : null}
      </h3>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(3, 1fr)',
          gap: 8,
          margin: '12px 0 16px',
        }}
      >
        <Stat label="entradas" value={formatMoneyNumber(realizedIn)} color="var(--green)" />
        <Stat label="saídas" value={formatMoneyNumber(-realizedOut)} color="var(--rose)" />
        <Stat label="saldo proj." value={formatMoneyNumber(projectedClose)} color="var(--purple)" />
      </div>

      <Section title={`previsões · ${forecasts.length}`}>
        {forecasts.length === 0 ? (
          <Muted>sem previsões neste mês.</Muted>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {forecasts.map((f) => (
              <ForecastChip key={f.forecastId} forecast={f} />
            ))}
          </div>
        )}
        <AddForecast month={month} onAdded={onForecastAdded} />
      </Section>

      <Section title={`transações · ${transactions.length}`}>
        {transactions.length === 0 ? (
          <Muted>sem transações neste mês.</Muted>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {transactions.slice(0, 60).map((t) => (
              <Line
                key={t.id}
                left={t.description || t.merchantName || t.rawDescription}
                sub={t.categoryId ?? '—'}
                amount={t.amount}
              />
            ))}
          </div>
        )}
      </Section>
    </Card>
  )
}

const Stat = ({ label, value, color }: { label: string; value: string; color: string }) => (
  <div
    style={{
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius-sm)',
      padding: '8px 10px',
    }}
  >
    <div className="mono" style={{ color: 'var(--muted)', fontSize: 10 }}>
      {label}
    </div>
    <div className="mono" style={{ color, fontWeight: 600, fontSize: 13, marginTop: 2 }}>
      {value}
    </div>
  </div>
)

const Section = ({ title, children }: { title: string; children: React.ReactNode }) => (
  <div style={{ marginTop: 14, paddingTop: 14, borderTop: '1px solid var(--border)' }}>
    <Label>{title}</Label>
    <div style={{ marginTop: 8 }}>{children}</div>
  </div>
)

const Muted = ({ children }: { children: React.ReactNode }) => (
  <p className="mono" style={{ color: 'var(--muted)', fontSize: 12, margin: 0 }}>
    {children}
  </p>
)

const Line = ({ left, sub, amount }: { left: string; sub: string; amount: string }) => (
  <div style={{ display: 'flex', justifyContent: 'space-between', gap: 10, alignItems: 'baseline' }}>
    <div style={{ minWidth: 0 }}>
      <div style={{ fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {left}
      </div>
      <div className="mono" style={{ color: 'var(--cyan)', fontSize: 10 }}>
        {sub}
      </div>
    </div>
    <span className="mono" style={{ color: amountColor(amount), fontSize: 13, whiteSpace: 'nowrap' }}>
      {formatMoney(amount)}
    </span>
  </div>
)

/**
 * Compact "nova previsão" affordance (the thin Previsões affordance inside
 * Planejamento). Defaults the due date to the selected month; posts straight to
 * /api/forecast (returns the id synchronously) then reloads the forecast seed so
 * the new chip + bars appear.
 */
const AddForecast = ({ month, onAdded }: { month: string; onAdded: () => void }) => {
  const [open, setOpen] = useState(false)
  const [description, setDescription] = useState('')
  const [amount, setAmount] = useState('')
  const [outflow, setOutflow] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const canSubmit = description.trim() !== '' && amount.trim() !== '' && !busy

  const submit = async () => {
    if (!canSubmit) return
    setBusy(true)
    setError(null)
    try {
      const magnitude = amount.replace(/^-/, '').trim()
      await api.createForecast({
        description: description.trim(),
        amount: outflow ? `-${magnitude}` : magnitude,
        due_date: `${month}-01`, // due_date required; default to the 1st
      })
      setDescription('')
      setAmount('')
      setOpen(false)
      onAdded()
    } catch (e: unknown) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="mono"
        style={{
          marginTop: 8,
          background: 'transparent',
          border: '1px dashed var(--border)',
          borderRadius: 'var(--radius-sm)',
          color: 'var(--muted)',
          fontSize: 12,
          padding: '6px 10px',
          cursor: 'pointer',
          width: '100%',
          textAlign: 'left',
        }}
      >
        + nova previsão em {month}
      </button>
    )
  }

  return (
    <div
      style={{
        marginTop: 8,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius-sm)',
        padding: 10,
      }}
    >
      <TextInput
        placeholder="descrição"
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        style={{ width: '100%' }}
      />
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <Pill accent={ACCENT} active={outflow} onClick={() => setOutflow(true)}>
          saída
        </Pill>
        <Pill accent="var(--green)" active={!outflow} onClick={() => setOutflow(false)}>
          entrada
        </Pill>
        <TextInput
          inputMode="decimal"
          placeholder="0,00"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          style={{ width: 100 }}
        />
      </div>
      {error && <ErrorNote error={error} />}
      <div style={{ display: 'flex', gap: 8 }}>
        <Pill accent={ACCENT} active={canSubmit} onClick={submit}>
          {busy ? '…' : 'adicionar →'}
        </Pill>
        <Pill onClick={() => setOpen(false)}>cancelar</Pill>
      </div>
    </div>
  )
}

/**
 * A draggable forecast chip. Manual forecasts (draggable=1) get a grab handle and
 * start a hand-rolled pointer drag (1:1 ghost + sanctioned shadow, see lib/dnd);
 * installments/subscriptions render locked. The chart's month columns are the
 * drop targets — dropping re-dates the forecast via `onDropForecast`.
 */
const ForecastChip = ({ forecast: f }: { forecast: ForecastView }) => {
  const { startDrag, dragging } = useDnd()
  const locked = f.draggable !== 1
  const isDragging = dragging?.forecastId === f.forecastId
  return (
    <div
      onPointerDown={(e) => {
        if (locked || e.button !== 0) return
        startDrag({ forecastId: f.forecastId, label: f.description, amount: formatMoney(f.amount) }, e)
      }}
      title={locked ? 'parcela/assinatura — bloqueada' : 'arraste para outro mês'}
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        gap: 8,
        alignItems: 'baseline',
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius-sm)',
        padding: '6px 10px',
        cursor: locked ? 'default' : 'grab',
        opacity: locked ? 0.7 : isDragging ? 0.4 : 1,
        touchAction: 'none',
        userSelect: 'none',
      }}
    >
      <span style={{ minWidth: 0, display: 'flex', gap: 6, alignItems: 'center' }}>
        <span className="mono" style={{ color: 'var(--muted)', fontSize: 12 }}>
          {locked ? '⊘' : '⠿'}
        </span>
        <span style={{ fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {f.description}
        </span>
      </span>
      <span className="mono" style={{ color: amountColor(f.amount), fontSize: 13, whiteSpace: 'nowrap' }}>
        {formatMoney(f.amount)}
      </span>
    </div>
  )
}
