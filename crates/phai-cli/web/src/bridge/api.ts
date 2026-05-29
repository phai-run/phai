/**
 * Bridge client — talks to the Rust `phai serve` HTTP API.
 *
 * Reads (GET) pull the system-of-record data (BigQuery/SQLite) to seed
 * LiveStore. Writes flush committed user actions so the Rust side can apply them
 * with an audit trail. Each flush kind has its own endpoint:
 *  - review edits   → POST /api/events
 *  - forecast moves → POST /api/forecast/move
 *  - forecast adds  → POST /api/forecast
 */

export interface TxRow {
  id: string
  accountId: string
  postedAt: string
  amount: string
  rawDescription: string
  description: string | null
  merchantName: string | null
  purpose: string | null
  categoryId: string | null
  month: string
  paymentStatus: string
  reviewed: boolean
  isInstallment: boolean
  isSubscription: boolean
}

export interface AccountRow {
  id: string
  label: string
  owner: string
}

export interface ReviewPatch {
  description: string | null
  merchantName: string | null
  purpose: string | null
  categoryId: string | null
}

export interface ReviewFlushItem {
  writeId: string
  transactionId: string
  patch: ReviewPatch
}

const json = async <T>(res: Response): Promise<T> => {
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return (await res.json()) as T
}

export interface FlushResult {
  acked: string[]
  failed: { writeId: string; error: string }[]
}

/**
 * Cash-evolution chart shape (Rust `ChartData`). Amounts are decimal strings.
 * Field names mirror the Rust serde shape; we tolerate a couple of aliases the
 * backend might use for the closing balance (see `sync.ts`).
 */
export interface ChartMonthApi {
  label: string
  inflows: string
  outflows: string
  forecast_inflows_remaining?: string
  forecast_outflows_remaining?: string
  closing_balance?: string
  projected_closing_balance?: string
  is_future?: boolean
}
export interface ChartData {
  months: ChartMonthApi[]
}

/** Forecast domain record (snake_case). Amount is a decimal string. */
export interface ForecastRecord {
  forecast_id: string
  due_date: string | null
  description: string
  amount: string
  category_id: string | null
  account_id: string | null
  status: string
  kind?: string
  draggable?: boolean
}

/** Forecast template domain record (snake_case). */
export interface ForecastTemplateRecord {
  template_id: string
  description: string
  kind: string | null
  cadence: string | null
  amount: string
  status: string
  confidence: number | string | null
}

export interface NewForecast {
  description: string
  amount: string // decimal string; negative = saída
  due_date?: string
  category_id?: string
  account_id?: string
}

const trimParams = (record: Record<string, string | null | undefined>): URLSearchParams => {
  const p = new URLSearchParams()
  for (const [k, v] of Object.entries(record)) {
    if (v != null && v !== '') p.set(k, v)
  }
  return p
}

const postJson = <T>(url: string, body: unknown): Promise<T> =>
  fetch(url, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  }).then((r) => json<T>(r))

export const api = {
  /** Seed the full transaction window — filtering/summing is then all local. */
  transactions: (params: {
    monthsBack: number
    monthsAhead: number
    includeReviewed?: boolean
    limit?: number
  }): Promise<{ rows: TxRow[] }> =>
    fetch(
      `/api/transactions?${trimParams({
        months_back: String(params.monthsBack),
        months_ahead: String(params.monthsAhead),
        include_reviewed: String(params.includeReviewed ?? true),
        limit: String(params.limit ?? 5000),
      })}`,
    ).then((r) => json<{ rows: TxRow[] }>(r)),

  categories: (): Promise<{ ids: string[] }> =>
    fetch('/api/categories').then((r) => json<{ ids: string[] }>(r)),
  accounts: (): Promise<{ rows: AccountRow[] }> =>
    fetch('/api/accounts').then((r) => json<{ rows: AccountRow[] }>(r)),

  chart: (monthsBack: number, monthsAhead: number): Promise<ChartData> =>
    fetch(
      `/api/chart?${trimParams({
        months_back: String(monthsBack),
        months_ahead: String(monthsAhead),
      })}`,
    ).then((r) => json<ChartData>(r)),

  forecasts: (filters: {
    status?: string | null
    from?: string | null
    until?: string | null
  }): Promise<{ forecasts: ForecastRecord[] }> =>
    fetch(`/api/forecasts?${trimParams(filters)}`).then((r) =>
      json<{ forecasts: ForecastRecord[] }>(r),
    ),

  forecastTemplates: (filters: {
    kind?: string | null
    status?: string | null
  }): Promise<{ templates: ForecastTemplateRecord[] }> =>
    fetch(`/api/forecast-templates?${trimParams(filters)}`).then((r) =>
      json<{ templates: ForecastTemplateRecord[] }>(r),
    ),

  createForecast: (forecast: NewForecast): Promise<{ forecast_id: string }> =>
    postJson<{ forecast_id: string }>('/api/forecast', forecast),

  /** Re-date a forecast (drag-and-drop in Planejamento). */
  moveForecast: (forecastId: string, dueDate: string): Promise<unknown> =>
    postJson('/api/forecast/move', { forecastId, dueDate }),

  acceptForecastTemplate: (templateId: string, materializeMonths = 6): Promise<unknown> =>
    postJson('/api/forecast-template/accept', {
      template_id: templateId,
      materialize_months: materializeMonths,
    }),

  dismissForecastTemplate: (templateId: string): Promise<unknown> =>
    postJson('/api/forecast-template/dismiss', { template_id: templateId }),

  /** Apply a batch of committed review writes; returns the writeIds that succeeded. */
  flushReviews: (items: ReviewFlushItem[]): Promise<FlushResult> =>
    postJson<FlushResult>('/api/events', { writes: items }),
}
