/**
 * Bridge client — talks to the Rust `phai serve` HTTP API.
 *
 * Reads (GET) pull the system-of-record data (BigQuery/SQLite) to seed
 * LiveStore. Writes (POST /api/events) flush committed review submissions so
 * the Rust side can apply them with an audit trail.
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

export interface FlushItem {
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

export const api = {
  reviewQueue: (params: URLSearchParams): Promise<{ rows: TxRow[] }> =>
    fetch(`/api/review-queue?${params}`).then((r) => json<{ rows: TxRow[] }>(r)),
  categories: (): Promise<{ ids: string[] }> =>
    fetch('/api/categories').then((r) => json<{ ids: string[] }>(r)),
  accounts: (): Promise<{ rows: AccountRow[] }> =>
    fetch('/api/accounts').then((r) => json<{ rows: AccountRow[] }>(r)),
  /** Apply a batch of committed writes; returns the writeIds that succeeded. */
  flush: (items: FlushItem[]): Promise<FlushResult> =>
    fetch('/api/events', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ writes: items }),
    }).then((r) => json<FlushResult>(r)),
}
