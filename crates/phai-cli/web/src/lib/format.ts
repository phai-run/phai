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
 * two fractional digits; extra precision is truncated (amounts are money).
 */
export const toCents = (amount: string | null | undefined): number => {
  if (amount == null || amount === '') return 0
  const s = amount.trim()
  const neg = s.startsWith('-')
  const digits = s.replace(/^[+-]/, '')
  const [whole, frac = ''] = digits.split('.')
  const w = Number.parseInt(whole || '0', 10)
  if (!Number.isFinite(w)) return 0
  const cents = Number.parseInt((frac + '00').slice(0, 2), 10) || 0
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
