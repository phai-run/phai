import { useMemo, useState } from 'react'
import { formatMoney, numeric } from '../lib/format'

/**
 * Cash-evolution chart, rendered as hand-drawn SVG (no charting dependency —
 * keeps the bundle pure and the palette under our control per DESIGN.md).
 *
 * Per month: inflow bar (cyan), outflow bar (rose, drawn downward), and a
 * closing-balance line (purple). Future months render dashed/dimmer to
 * separate realized from forecast. All amounts are decimal strings; we parse
 * to numbers for geometry only — never for display totals.
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

const W = 720
const H = 320
const PAD = { top: 20, right: 16, bottom: 40, left: 16 }
const innerW = W - PAD.left - PAD.right
const innerH = H - PAD.top - PAD.bottom

export const CashflowChart = ({ months }: { months: ReadonlyArray<ChartMonthView> }) => {
  const [hover, setHover] = useState<number | null>(null)

  const model = useMemo(() => {
    const inflow = (m: ChartMonthView) =>
      numeric(m.inflows) + (m.isFuture ? numeric(m.forecastInflowsRemaining) : 0)
    const outflow = (m: ChartMonthView) =>
      Math.abs(numeric(m.outflows)) +
      (m.isFuture ? Math.abs(numeric(m.forecastOutflowsRemaining)) : 0)
    const balance = (m: ChartMonthView) =>
      m.isFuture ? numeric(m.projectedClosingBalance) : numeric(m.closingBalance)

    const inflows = months.map(inflow)
    const outflows = months.map(outflow)
    const balances = months.map(balance)

    const maxBar = Math.max(1, ...inflows, ...outflows)
    const minBal = Math.min(0, ...balances)
    const maxBal = Math.max(1, ...balances)
    const balSpan = maxBal - minBal || 1

    return { inflows, outflows, balances, maxBar, minBal, balSpan }
  }, [months])

  if (months.length === 0) return null

  const n = months.length
  const slot = innerW / n
  const barW = Math.min(28, slot * 0.32)
  const baseY = PAD.top + innerH / 2 // bars grow up (inflow) / down (outflow)
  const barH = (v: number) => (v / model.maxBar) * (innerH / 2 - 6)
  const balY = (v: number) =>
    PAD.top + innerH - ((v - model.minBal) / model.balSpan) * innerH

  const xCenter = (i: number) => PAD.left + slot * i + slot / 2

  const linePath = model.balances
    .map((b, i) => `${i === 0 ? 'M' : 'L'} ${xCenter(i).toFixed(1)} ${balY(b).toFixed(1)}`)
    .join(' ')

  // Split the balance line into realized (solid) and forecast (dashed).
  const firstFuture = months.findIndex((m) => m.isFuture === 1)

  return (
    <div style={{ position: 'relative' }}>
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label="evolução de caixa">
        {/* zero baseline */}
        <line
          x1={PAD.left}
          x2={W - PAD.right}
          y1={baseY}
          y2={baseY}
          stroke="var(--border)"
          strokeWidth={1}
        />

        {months.map((m, i) => {
          const x = xCenter(i)
          const inH = barH(model.inflows[i])
          const outH = barH(model.outflows[i])
          const dim = m.isFuture ? 0.45 : 1
          return (
            <g
              key={m.label}
              onMouseEnter={() => setHover(i)}
              onMouseLeave={() => setHover(null)}
            >
              {/* hover hit area */}
              <rect
                x={PAD.left + slot * i}
                y={PAD.top}
                width={slot}
                height={innerH}
                fill={hover === i ? 'rgba(255,255,255,0.03)' : 'transparent'}
              />
              {/* inflow (cyan, up) */}
              <rect
                x={x - barW - 1}
                y={baseY - inH}
                width={barW}
                height={inH}
                rx={2}
                fill="var(--cyan)"
                opacity={dim}
              />
              {/* outflow (rose, down) */}
              <rect
                x={x + 1}
                y={baseY}
                width={barW}
                height={outH}
                rx={2}
                fill="var(--rose)"
                opacity={dim}
              />
              {/* x label */}
              <text
                x={x}
                y={H - 14}
                textAnchor="middle"
                fontSize={10}
                fontFamily="var(--font-mono)"
                fill="var(--muted)"
              >
                {m.label.slice(2)}
              </text>
            </g>
          )
        })}

        {/* balance line — realized solid, forecast dashed */}
        {firstFuture === -1 ? (
          <path d={linePath} fill="none" stroke="var(--purple)" strokeWidth={2} />
        ) : (
          <>
            <path
              d={model.balances
                .slice(0, firstFuture + 1)
                .map((b, i) => `${i === 0 ? 'M' : 'L'} ${xCenter(i)} ${balY(b)}`)
                .join(' ')}
              fill="none"
              stroke="var(--purple)"
              strokeWidth={2}
            />
            <path
              d={model.balances
                .slice(Math.max(0, firstFuture))
                .map((b, k) => {
                  const i = Math.max(0, firstFuture) + k
                  return `${k === 0 ? 'M' : 'L'} ${xCenter(i)} ${balY(b)}`
                })
                .join(' ')}
              fill="none"
              stroke="var(--purple)"
              strokeWidth={2}
              strokeDasharray="4 4"
              opacity={0.6}
            />
          </>
        )}

        {/* balance dots */}
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

      {hover != null && <Tooltip month={months[hover]} model={model} index={hover} />}

      <Legend />
    </div>
  )
}

const Tooltip = ({
  month,
  model,
  index,
}: {
  month: ChartMonthView
  model: { inflows: number[]; outflows: number[]; balances: number[] }
  index: number
}) => (
  <div
    className="mono"
    style={{
      position: 'absolute',
      top: 0,
      right: 0,
      background: 'var(--surface)',
      border: '1px solid var(--border)',
      borderRadius: 'var(--radius-sm)',
      padding: '10px 12px',
      fontSize: 11,
      lineHeight: 1.8,
      pointerEvents: 'none',
    }}
  >
    <div style={{ color: 'var(--white)', marginBottom: 4 }}>
      {month.label}
      {month.isFuture ? ' · previsto' : ''}
    </div>
    <div style={{ color: 'var(--cyan)' }}>entradas {formatMoney(String(model.inflows[index]))}</div>
    <div style={{ color: 'var(--rose)' }}>saídas {formatMoney(String(model.outflows[index]))}</div>
    <div style={{ color: 'var(--purple)' }}>
      saldo {formatMoney(String(model.balances[index]))}
    </div>
  </div>
)

const Legend = () => (
  <div
    className="mono"
    style={{ display: 'flex', gap: 18, fontSize: 11, color: 'var(--muted)', marginTop: 12 }}
  >
    <Swatch color="var(--cyan)" label="entradas" />
    <Swatch color="var(--rose)" label="saídas" />
    <Swatch color="var(--purple)" label="saldo projetado" />
    <span>· · · = previsto</span>
  </div>
)

const Swatch = ({ color, label }: { color: string; label: string }) => (
  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
    <span style={{ width: 10, height: 10, borderRadius: 2, background: color }} />
    {label}
  </span>
)
