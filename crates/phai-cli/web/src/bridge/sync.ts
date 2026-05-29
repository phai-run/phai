import { queryDb } from '@livestore/livestore'
import { useStore } from '@livestore/react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { events, tables } from '../livestore/schema'
import {
  api,
  type ChartData,
  type FlushItem,
  type ForecastRecord,
  type ForecastTemplateRecord,
} from './api'

const pendingWrites$ = queryDb(tables.pendingWrites)

export interface SyncStatus {
  pending: number
  error: string | null
  seeded: boolean
}

/**
 * Wires LiveStore to the Rust bridge:
 *  1. On mount, seed reference data (categories, accounts) from the bridge.
 *  2. Continuously drain `pendingWrites` to `POST /api/events`, committing
 *     `writeAcked` / `writeFailed` on the result. Retries on the next tick.
 *
 * The per-view re-seed of the review queue, chart, forecasts and templates is
 * handled by the dedicated hooks below (`useReviewQueueSeed`, etc.), which the
 * views call so a seed only fires when that view is mounted.
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

  // 2. Drain the pending-write queue.
  useEffect(() => {
    const flush = async () => {
      if (flushing.current) return
      const rows = store.query(pendingWrites$)
      setPending(rows.length)
      if (rows.length === 0) return
      flushing.current = true
      try {
        const items: FlushItem[] = rows.map((r) => ({
          writeId: r.writeId,
          transactionId: r.transactionId,
          patch: r.payload as FlushItem['patch'],
        }))
        const res = await api.flush(items)
        store.commit(...res.acked.map((writeId) => events.writeAcked({ writeId })))
        store.commit(
          ...res.failed.map((f) => events.writeFailed({ writeId: f.writeId, error: f.error })),
        )
        setError(res.failed.length > 0 ? res.failed[0].error : null)
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

export interface SeedState {
  loading: boolean
  error: string | null
  reload: () => void
}

/**
 * Generic "fetch from bridge → commit a seed event" hook. Re-runs whenever
 * `deps` change (e.g. filters) and exposes a manual `reload`.
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

export interface ReviewFilters {
  month: string | null
  owner: string | null
  accountId: string | null
  merchant: string | null
  category: string | null
  includeReviewed: boolean
}

/** Re-seed the review queue from the bridge whenever filters change. */
export const useReviewQueueSeed = (filters: ReviewFilters): SeedState => {
  const { store } = useStore()
  const fetcher = useCallback(async () => {
    const params = new URLSearchParams()
    if (filters.month) params.set('month', filters.month)
    if (filters.owner) params.set('owner', filters.owner)
    if (filters.accountId) params.set('account_id', filters.accountId)
    if (filters.merchant) params.set('merchant', filters.merchant)
    if (filters.category) params.set('category', filters.category)
    if (filters.includeReviewed) params.set('include_reviewed', 'true')
    const { rows } = await api.reviewQueue(params)
    store.commit(events.queueSeeded({ rows }))
  }, [
    store,
    filters.month,
    filters.owner,
    filters.accountId,
    filters.merchant,
    filters.category,
    filters.includeReviewed,
  ])
  return useSeed(fetcher, [fetcher])
}

const normalizeChart = (data: ChartData) =>
  data.months.map((m, i) => ({
    label: m.label,
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
