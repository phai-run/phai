/**
 * Display formatters + sums. Amounts arrive as decimal-as-string from the bridge
 * (rust_decimal). The bridge is the source of truth for server-computed totals;
 * the running sums the UI derives from filtered LiveStore rows are computed in
 * integer cents (never float math) so they never drift.
 */

/** Parse a decimal string to a number, for *display-only* purposes. */
const toNumber = (amount: string | null | undefined): number => {
  if (amount == null || amount === '') return 0
  const n = Number(amount)
  return Number.isFinite(n) ? n : 0
}

/**
 * Parse a decimal string to integer cents. Used for client-side running totals
 * so addition stays exact (no float drift). Handles a leading sign and at most
 * two fractional digits; extra precision is rounded to the nearest cent.
 */
export const toCents = (amount: string | null | undefined): number => {
  if (amount == null || amount === '') return 0
  const s = amount.trim()
  const match = /^([+-])?(\d*)(?:\.(\d*))?$/.exec(s)
  if (!match || (match[2] === '' && (match[3] ?? '') === '')) return 0
  const neg = match[1] === '-'
  const whole = match[2] || '0'
  const frac = match[3] ?? ''
  const w = Number.parseInt(whole, 10)
  if (!Number.isFinite(w)) return 0
  let cents = Number.parseInt((frac + '00').slice(0, 2), 10) || 0
  if (Number.parseInt((frac + '000')[2] ?? '0', 10) >= 5) cents += 1
  const total = w * 100 + cents
  return neg ? -total : total
}

/** Sum decimal-string amounts exactly (integer cents), returning a number. */
export const sumAmounts = (amounts: ReadonlyArray<string | null | undefined>): number =>
  amounts.reduce<number>((acc, a) => acc + toCents(a), 0) / 100

const brl = new Intl.NumberFormat('pt-BR', {
  style: 'currency',
  currency: 'BRL',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
})

/** Format a decimal-string amount as pt-BR R$. Display only. */
export const formatMoney = (amount: string | null | undefined): string =>
  brl.format(toNumber(amount))

/** Format an already-parsed number as pt-BR R$. Display only. */
export const formatMoneyNumber = (n: number): string => brl.format(n)

const brlCompact = new Intl.NumberFormat('pt-BR', {
  style: 'currency',
  currency: 'BRL',
  notation: 'compact',
  maximumFractionDigits: 1,
})

/** Compact pt-BR R$ for tight UI (e.g. "R$ 8,2 mil"). Display only. */
export const formatMoneyCompact = (n: number): string => brlCompact.format(n)

/** True when the amount is negative (an expense / saída). */
export const isNegative = (amount: string | null | undefined): boolean => {
  if (amount == null) return false
  return amount.trim().startsWith('-')
}

/** Palette colour for an amount by sign: rose for expense, green for income. */
export const amountColor = (amount: string | null | undefined): string =>
  isNegative(amount) ? 'var(--rose)' : 'var(--green)'

/** Numeric value of a decimal string for charting geometry. Display only. */
export const numeric = toNumber
