import { queryDb } from '@livestore/livestore'
import { useStore } from '@livestore/react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { events, tables } from '../livestore/schema'
import {
  api,
  type ChartData,
  type ForecastRecord,
  type ForecastTemplateRecord,
  type ReviewFlushItem,
  type TxRow,
} from './api'

const pendingWrites$ = queryDb(tables.pendingWrites)

export interface SyncStatus {
  pending: number
  error: string | null
  seeded: boolean
}

interface PendingRow {
  writeId: string
  type: string
  transactionId: string
  forecastId: string
  payload: unknown
}

const bool = (v: unknown): number => (v ? 1 : 0)

/**
 * Wires LiveStore to the Rust bridge:
 *  1. On mount, seed reference data (categories, accounts) from the bridge.
 *  2. Continuously drain `pendingWrites`, routing each row to its endpoint by
 *     `type` (review → /api/events, forecastMove → /api/forecast/move). On
 *     success, commit `writeAcked`; on failure, `writeFailed`. Retries on the
 *     next tick.
 *
 * The per-view re-seed of the transaction window, chart, forecasts and templates
 * is handled by the dedicated hooks below, which the views call so a seed only
 * fires when that view is mounted.
 */
export const useBridgeSync = (): SyncStatus => {
  const { store } = useStore()
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(0)
  const [seeded, setSeeded] = useState(false)
  const flushing = useRef(false)

  // 1. Seed reference data once.
  useEffect(() => {
    let cancelled = false
    Promise.all([api.categories(), api.accounts()])
      .then(([cats, accs]) => {
        if (cancelled) return
        store.commit(events.categoriesSeeded({ ids: cats.ids }))
        store.commit(events.accountsSeeded({ rows: accs.rows }))
        setSeeded(true)
      })
      .catch((e: unknown) => setError(String(e)))
    return () => {
      cancelled = true
    }
  }, [store])

  // 2. Drain the typed pending-write queue.
  useEffect(() => {
    const flush = async () => {
      if (flushing.current) return
      const rows = store.query(pendingWrites$) as ReadonlyArray<PendingRow>
      setPending(rows.length)
      if (rows.length === 0) return
      flushing.current = true
      try {
        const failures = await drainQueue(store, rows)
        setError(failures.length > 0 ? failures[0] : null)
      } catch (e: unknown) {
        setError(String(e))
      } finally {
        flushing.current = false
      }
    }

    const sub = store.subscribe(pendingWrites$, { onUpdate: () => void flush() })
    void flush()
    const timer = setInterval(() => void flush(), 5000)
    return () => {
      sub()
      clearInterval(timer)
    }
  }, [store])

  return { pending, error, seeded }
}

type StoreApi = ReturnType<typeof useStore>['store']

/**
 * Routes each pending write to the right endpoint by `type`. Reviews flush as a
 * single batch (the bridge accepts `{ writes }`); forecast moves flush one at a
 * time. Returns the error strings of any failures (for the status chip).
 */
const drainQueue = async (
  store: StoreApi,
  rows: ReadonlyArray<PendingRow>,
): Promise<string[]> => {
  const errors: string[] = []

  const reviews = rows.filter((r) => r.type === 'review')
  if (reviews.length > 0) {
    const items: ReviewFlushItem[] = reviews.map((r) => ({
      writeId: r.writeId,
      transactionId: r.transactionId,
      patch: r.payload as ReviewFlushItem['patch'],
    }))
    try {
      const res = await api.flushReviews(items)
      store.commit(...res.acked.map((writeId) => events.writeAcked({ writeId })))
      store.commit(
        ...res.failed.map((f) => events.writeFailed({ writeId: f.writeId, error: f.error })),
      )
      for (const f of res.failed) errors.push(f.error)
    } catch (e: unknown) {
      // Whole batch failed (network) — mark each so the chip surfaces it.
      const msg = String(e)
      store.commit(...reviews.map((r) => events.writeFailed({ writeId: r.writeId, error: msg })))
      errors.push(msg)
    }
  }

  for (const r of rows) {
    if (r.type !== 'forecastMove') continue
    const dueDate = (r.payload as { dueDate: string }).dueDate
    try {
      await api.moveForecast(r.forecastId, dueDate)
      store.commit(events.writeAcked({ writeId: r.writeId }))
    } catch (e: unknown) {
      const msg = String(e)
      store.commit(events.writeFailed({ writeId: r.writeId, error: msg }))
      errors.push(msg)
    }
  }

  return errors
}

export interface SeedState {
  loading: boolean
  error: string | null
  reload: () => void
}

/**
 * Generic "fetch from bridge → commit a seed event" hook. Re-runs whenever
 * `deps` change (e.g. window controls) and exposes a manual `reload`.
 */
const useSeed = (
  fetcher: () => Promise<void>,
  deps: ReadonlyArray<unknown>,
): SeedState => {
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [nonce, setNonce] = useState(0)
  const reload = useCallback(() => setNonce((n) => n + 1), [])

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setError(null)
    fetcher()
      .then(() => {
        if (!cancelled) setError(null)
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(String(e))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
    // fetcher is recreated by the caller when deps change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, nonce])

  return { loading, error, reload }
}

const normalizeTransactions = (rows: TxRow[]) =>
  rows.map((r) => ({
    id: r.id,
    accountId: r.accountId ?? '',
    postedAt: r.postedAt ?? '',
    amount: r.amount ?? '0',
    rawDescription: r.rawDescription ?? '',
    description: r.description ?? null,
    merchantName: r.merchantName ?? null,
    purpose: r.purpose ?? null,
    categoryId: r.categoryId ?? null,
    month: r.month ?? '',
    paymentStatus: r.paymentStatus ?? '',
    reviewed: bool(r.reviewed),
    isInstallment: bool(r.isInstallment),
    isSubscription: bool(r.isSubscription),
  }))

/**
 * Seed the full transaction window from the bridge. The whole window lives in
 * LiveStore so every filter/sum in the Review view is computed locally.
 */
export const useTransactionsSeed = (monthsBack: number, monthsAhead: number): SeedState => {
  const { store } = useStore()
  const fetcher = useCallback(async () => {
    const { rows } = await api.transactions({
      monthsBack,
      monthsAhead,
      includeReviewed: true,
      limit: 5000,
    })
    store.commit(events.transactionsSeeded({ rows: normalizeTransactions(rows) }))
  }, [store, monthsBack, monthsAhead])
  return useSeed(fetcher, [fetcher])
}

const normalizeChart = (data: ChartData) =>
  data.months.map((m, i) => ({
    label: m.label,
    month: m.month ?? m.label,
    inflows: m.inflows ?? '0',
    outflows: m.outflows ?? '0',
    forecastInflowsRemaining: m.forecast_inflows_remaining ?? '0',
    forecastOutflowsRemaining: m.forecast_outflows_remaining ?? '0',
    closingBalance: m.closing_balance ?? m.projected_closing_balance ?? '0',
    projectedClosingBalance: m.projected_closing_balance ?? m.closing_balance ?? '0',
    isFuture: m.is_future ? 1 : 0,
    ordinal: i,
  }))

/** Re-seed the cash-evolution chart from the bridge. */
export const useChartSeed = (monthsBack: number, monthsAhead: number): SeedState => {
  const { store } = useStore()
  const fetcher = useCallback(async () => {
    const data = await api.chart(monthsBack, monthsAhead)
    store.commit(events.chartSeeded({ months: normalizeChart(data) }))
  }, [store, monthsBack, monthsAhead])
  return useSeed(fetcher, [fetcher])
}

const normalizeForecasts = (forecasts: ForecastRecord[]) =>
  forecasts.map((f) => ({
    forecastId: f.forecast_id,
    dueDate: f.due_date ?? null,
    description: f.description ?? '',
    amount: f.amount ?? '0',
    categoryId: f.category_id ?? null,
    accountId: f.account_id ?? null,
    status: f.status ?? '',
    kind: f.kind ?? 'manual',
    draggable: bool(f.draggable),
  }))

const normalizeTemplates = (templates: ForecastTemplateRecord[]) =>
  templates.map((t) => ({
    templateId: t.template_id,
    description: t.description ?? '',
    kind: t.kind ?? null,
    cadence: t.cadence ?? null,
    amount: t.amount ?? '0',
    status: t.status ?? '',
    confidence: t.confidence == null ? null : String(t.confidence),
  }))

/** Re-seed forecasts + templates from the bridge; reload after mutations. */
export const useForecastsSeed = (status: string | null): SeedState => {
  const { store } = useStore()
  const fetcher = useCallback(async () => {
    const [{ forecasts }, { templates }] = await Promise.all([
      api.forecasts({ status }),
      api.forecastTemplates({}),
    ])
    store.commit(events.forecastsSeeded({ rows: normalizeForecasts(forecasts) }))
    store.commit(events.forecastTemplatesSeeded({ rows: normalizeTemplates(templates) }))
  }, [store, status])
  return useSeed(fetcher, [fetcher])
}
