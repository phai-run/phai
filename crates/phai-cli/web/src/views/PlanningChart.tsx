import { useEffect, useMemo, useRef, useState } from 'react'
import { formatMoney, numeric } from '../lib/format'
import { useDnd } from '../lib/dnd'
import type { ForecastView } from './Planning'

/**
 * The cash-evolution chart — the spine of Planejamento. Hand-drawn SVG (no
 * charting dep) so the palette stays under our control (DESIGN.md).
 *
 * Per month:
 *  - inflow bar (cyan) up, outflow bar (rose) down, each STACKING realized over
 *    forecast: realized = solid accent, forecast-remaining = same hue at a lighter
 *    tint + diagonal hatch.
 *  - a projected closing-balance line (purple); realized solid, future dashed.
 *
 * An HTML column grid overlays the SVG: each column is the click target (selects
 * the month — instant client state), the hover target (opens a forecast popover),
 * and a drag drop target (re-date a dragged forecast to that month).
 */
export interface ChartMonthView {
  label: string
  inflows: string
  outflows: string
  forecastInflowsRemaining: string
  forecastOutflowsRemaining: string
  closingBalance: string
  projectedClosingBalance: string
  isFuture: number
}

const W = 960
const H = 340
const PAD = { top: 20, right: 16, bottom: 40, left: 16 }
const innerW = W - PAD.left - PAD.right
const innerH = H - PAD.top - PAD.bottom

export const PlanningChart = ({
  months,
  forecastsByMonth,
  selectedMonth,
  onSelectMonth,
  onDropForecast,
}: {
  months: ReadonlyArray<ChartMonthView>
  forecastsByMonth: Map<string, ForecastView[]>
  selectedMonth: string | null
  onSelectMonth: (label: string) => void
  onDropForecast: (forecastId: string, targetMonth: string) => void
}) => {
  const [hover, setHover] = useState<number | null>(null)

  const model = useMemo(() => {
    const realIn = (m: ChartMonthView) => Math.max(0, numeric(m.inflows))
    const fcIn = (m: ChartMonthView) => Math.max(0, numeric(m.forecastInflowsRemaining))
    const realOut = (m: ChartMonthView) => Math.abs(numeric(m.outflows))
    const fcOut = (m: ChartMonthView) => Math.abs(numeric(m.forecastOutflowsRemaining))
    const balance = (m: ChartMonthView) =>
      m.isFuture ? numeric(m.projectedClosingBalance) : numeric(m.closingBalance)

    const realIns = months.map(realIn)
    const fcIns = months.map(fcIn)
    const realOuts = months.map(realOut)
    const fcOuts = months.map(fcOut)
    const balances = months.map(balance)

    const maxBar = Math.max(
      1,
      ...months.map((_, i) => realIns[i] + fcIns[i]),
      ...months.map((_, i) => realOuts[i] + fcOuts[i]),
    )
    const minBal = Math.min(0, ...balances)
    const maxBal = Math.max(1, ...balances)
    const balSpan = maxBal - minBal || 1

    return { realIns, fcIns, realOuts, fcOuts, balances, maxBar, minBal, balSpan }
  }, [months])

  if (months.length === 0) return null

  const n = months.length
  const slot = innerW / n
  const barW = Math.min(26, slot * 0.3)
  const baseY = PAD.top + innerH / 2
  const barH = (v: number) => (v / model.maxBar) * (innerH / 2 - 6)
  const balY = (v: number) =>
    PAD.top + innerH - ((v - model.minBal) / model.balSpan) * innerH
  const xCenter = (i: number) => PAD.left + slot * i + slot / 2

  const firstFuture = months.findIndex((m) => m.isFuture === 1)

  const realizedLine =
    firstFuture === -1
      ? model.balances
      : model.balances.slice(0, firstFuture + 1)
  const futureLine = firstFuture === -1 ? [] : model.balances.slice(firstFuture)

  const linePath = (vals: number[], offset = 0) =>
    vals.map((b, k) => `${k === 0 ? 'M' : 'L'} ${xCenter(offset + k)} ${balY(b)}`).join(' ')

  return (
    <div style={{ position: 'relative' }}>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        width="100%"
        role="img"
        aria-label="evolução de caixa"
        style={{ display: 'block' }}
      >
        <defs>
          <pattern id="hatch-cyan" width="5" height="5" patternTransform="rotate(45)" patternUnits="userSpaceOnUse">
            <rect width="5" height="5" fill="var(--cyan)" opacity={0.18} />
            <line x1="0" y1="0" x2="0" y2="5" stroke="var(--cyan)" strokeWidth="1.4" opacity={0.6} />
          </pattern>
          <pattern id="hatch-rose" width="5" height="5" patternTransform="rotate(45)" patternUnits="userSpaceOnUse">
            <rect width="5" height="5" fill="var(--rose)" opacity={0.18} />
            <line x1="0" y1="0" x2="0" y2="5" stroke="var(--rose)" strokeWidth="1.4" opacity={0.6} />
          </pattern>
        </defs>

        {/* zero baseline */}
        <line x1={PAD.left} x2={W - PAD.right} y1={baseY} y2={baseY} stroke="var(--border)" strokeWidth={1} />

        {months.map((m, i) => {
          const x = xCenter(i)
          const realInH = barH(model.realIns[i])
          const fcInH = barH(model.fcIns[i])
          const realOutH = barH(model.realOuts[i])
          const fcOutH = barH(model.fcOuts[i])
          const isSel = m.label === selectedMonth
          return (
            <g key={m.label}>
              {(hover === i || isSel) && (
                <rect
                  x={PAD.left + slot * i}
                  y={PAD.top}
                  width={slot}
                  height={innerH}
                  fill={isSel ? 'rgba(13,148,136,0.08)' : 'rgba(0,0,0,0.04)'}
                />
              )}

              {/* inflow: realized (solid) then forecast (hatch) stacked above */}
              <rect x={x - barW - 1} y={baseY - realInH} width={barW} height={realInH} rx={2} fill="var(--cyan)" />
              {fcInH > 0 && (
                <rect
                  x={x - barW - 1}
                  y={baseY - realInH - fcInH}
                  width={barW}
                  height={fcInH}
                  rx={2}
                  fill="url(#hatch-cyan)"
                  stroke="var(--cyan)"
                  strokeOpacity={0.4}
                  strokeWidth={0.5}
                />
              )}

              {/* outflow: realized (solid) then forecast (hatch) stacked below */}
              <rect x={x + 1} y={baseY} width={barW} height={realOutH} rx={2} fill="var(--rose)" />
              {fcOutH > 0 && (
                <rect
                  x={x + 1}
                  y={baseY + realOutH}
                  width={barW}
                  height={fcOutH}
                  rx={2}
                  fill="url(#hatch-rose)"
                  stroke="var(--rose)"
                  strokeOpacity={0.4}
                  strokeWidth={0.5}
                />
              )}

              <text x={x} y={H - 14} textAnchor="middle" fontSize={10} fontFamily="var(--font-mono)" fill="var(--muted)">
                {m.label.slice(2)}
              </text>
            </g>
          )
        })}

        {/* balance line — realized solid, forecast dashed */}
        <path d={linePath(realizedLine)} fill="none" stroke="var(--purple)" strokeWidth={2} />
        {futureLine.length > 1 && (
          <path
            d={linePath(futureLine, Math.max(0, firstFuture))}
            fill="none"
            stroke="var(--purple)"
            strokeWidth={2}
            strokeDasharray="4 4"
            opacity={0.6}
          />
        )}
        {model.balances.map((b, i) => (
          <circle
            key={months[i].label}
            cx={xCenter(i)}
            cy={balY(b)}
            r={2.5}
            fill="var(--purple)"
            opacity={months[i].isFuture ? 0.6 : 1}
          />
        ))}
      </svg>

      {/* HTML column overlay: click / hover / drop-target per month */}
      <ColumnOverlay
        months={months}
        selectedMonth={selectedMonth}
        onSelectMonth={onSelectMonth}
        onHover={setHover}
        onDropForecast={onDropForecast}
      />

      {hover != null && (
        <BarPopover
          month={months[hover]}
          forecasts={forecastsByMonth.get(months[hover].label) ?? []}
          leftPct={((hover + 0.5) / n) * 100}
        />
      )}

      <Legend />
    </div>
  )
}

const ColumnOverlay = ({
  months,
  selectedMonth,
  onSelectMonth,
  onHover,
  onDropForecast,
}: {
  months: ReadonlyArray<ChartMonthView>
  selectedMonth: string | null
  onSelectMonth: (label: string) => void
  onHover: (i: number | null) => void
  onDropForecast: (forecastId: string, targetMonth: string) => void
}) => (
  <div
    style={{
      position: 'absolute',
      inset: 0,
      display: 'grid',
      gridTemplateColumns: `repeat(${months.length}, 1fr)`,
      // leave room for the x-axis labels (bottom band) so clicks land on bars
      paddingBottom: `${(PAD.bottom / H) * 100}%`,
    }}
  >
    {months.map((m, i) => (
      <MonthColumn
        key={m.label}
        month={m.label}
        index={i}
        selected={m.label === selectedMonth}
        onSelect={() => onSelectMonth(m.label)}
        onHover={onHover}
        onDropForecast={onDropForecast}
      />
    ))}
  </div>
)

const MonthColumn = ({
  month,
  index,
  selected,
  onSelect,
  onHover,
  onDropForecast,
}: {
  month: string
  index: number
  selected: boolean
  onSelect: () => void
  onHover: (i: number | null) => void
  onDropForecast: (forecastId: string, targetMonth: string) => void
}) => {
  const { dragging, hoverTargetId, registerTarget } = useDnd()
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    return registerTarget({
      id: `month:${month}`,
      getRect: () => ref.current?.getBoundingClientRect() ?? null,
      onDrop: (payload) => onDropForecast(payload.forecastId, month),
    })
  }, [month, registerTarget, onDropForecast])

  const isDropHover = dragging != null && hoverTargetId === `month:${month}`

  return (
    <div
      ref={ref}
      onClick={onSelect}
      onMouseEnter={() => onHover(index)}
      onMouseLeave={() => onHover(null)}
      style={{
        cursor: 'pointer',
        borderRadius: 'var(--radius-sm)',
        outline: isDropHover ? '2px solid var(--purple)' : 'none',
        outlineOffset: -2,
        background: isDropHover
          ? 'rgba(109,74,255,0.10)'
          : selected
            ? 'transparent'
            : 'transparent',
        transition: 'outline-color 120ms',
      }}
      aria-label={`mês ${month}`}
    />
  )
}

const BarPopover = ({
  month,
  forecasts,
  leftPct,
}: {
  month: ChartMonthView
  forecasts: ForecastView[]
  leftPct: number
}) => {
  const inflow = numeric(month.inflows) + numeric(month.forecastInflowsRemaining)
  const outflow = Math.abs(numeric(month.outflows)) + Math.abs(numeric(month.forecastOutflowsRemaining))
  const close = month.isFuture ? month.projectedClosingBalance : month.closingBalance
  const onRight = leftPct > 60
  return (
    <div
      className="mono"
      style={{
        position: 'absolute',
        top: 8,
        [onRight ? 'right' : 'left']: onRight ? `${100 - leftPct + 2}%` : `${leftPct + 2}%`,
        background: 'var(--surface)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius-sm)',
        padding: '10px 12px',
        fontSize: 11,
        lineHeight: 1.7,
        pointerEvents: 'none',
        minWidth: 180,
        maxWidth: 240,
        zIndex: 5,
      } as React.CSSProperties}
    >
      <div style={{ color: 'var(--white)', marginBottom: 4 }}>
        {month.label}
        {month.isFuture ? ' · previsto' : ''}
      </div>
      <div style={{ color: 'var(--cyan)' }}>entradas {formatMoney(String(inflow))}</div>
      <div style={{ color: 'var(--rose)' }}>saídas {formatMoney(String(-outflow))}</div>
      <div style={{ color: 'var(--purple)' }}>saldo {formatMoney(close)}</div>
      {forecasts.length > 0 && (
        <div style={{ marginTop: 6, paddingTop: 6, borderTop: '1px solid var(--border)' }}>
          <div style={{ color: 'var(--muted)', marginBottom: 2 }}>previsões</div>
          {forecasts.slice(0, 6).map((f) => (
            <div key={f.forecastId} style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {f.description}
              </span>
              <span style={{ color: 'var(--muted)', whiteSpace: 'nowrap' }}>{formatMoney(f.amount)}</span>
            </div>
          ))}
          {forecasts.length > 6 && <div style={{ color: 'var(--muted)' }}>+{forecasts.length - 6}</div>}
        </div>
      )}
    </div>
  )
}

const Legend = () => (
  <div className="mono" style={{ display: 'flex', flexWrap: 'wrap', gap: 16, fontSize: 11, color: 'var(--muted)', marginTop: 12 }}>
    <Swatch color="var(--cyan)" label="entradas" />
    <Swatch color="var(--rose)" label="saídas" />
    <Swatch hatch label="previsto" />
    <Swatch color="var(--purple)" label="saldo projetado" />
  </div>
)

const Swatch = ({ color, label, hatch }: { color?: string; label: string; hatch?: boolean }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
    <span
      style={{
        width: 10,
        height: 10,
        borderRadius: 2,
        background: hatch
          ? 'repeating-linear-gradient(45deg, var(--muted2) 0 1.4px, transparent 1.4px 4px)'
          : color,
        border: hatch ? '1px solid var(--muted2)' : 'none',
      }}
    />
    {label}
  </span>
)
