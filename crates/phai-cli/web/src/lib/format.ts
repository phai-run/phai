/**
 * Display formatters. Amounts arrive as decimal-as-string from the bridge
 * (rust_decimal). We NEVER do float math on them beyond what's needed to
 * render — the bridge is the source of truth for every computed total.
 */

/** Parse a decimal string to a number, for *display-only* purposes. */
const toNumber = (amount: string | null | undefined): number => {
  if (amount == null || amount === '') return 0
  const n = Number(amount)
  return Number.isFinite(n) ? n : 0
}

const brl = new Intl.NumberFormat('pt-BR', {
  style: 'currency',
  currency: 'BRL',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
})

/** Format a decimal-string amount as pt-BR R$. Display only. */
export const formatMoney = (amount: string | null | undefined): string =>
  brl.format(toNumber(amount))

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
