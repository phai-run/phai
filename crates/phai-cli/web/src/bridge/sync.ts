import { queryDb } from '@livestore/livestore'
import { useStore } from '@livestore/react'
import { useEffect, useRef, useState } from 'react'
import { events, tables } from '../livestore/schema'
import { api, type FlushItem } from './api'

const pendingWrites$ = queryDb(tables.pendingWrites)

export interface SyncStatus {
  pending: number
  error: string | null
  seeded: boolean
}

/**
 * Wires LiveStore to the Rust bridge:
 *  1. On mount, seed read-models (categories, accounts) from the bridge.
 *  2. Continuously drain `pendingWrites` to `POST /api/events`, committing
 *     `writeAcked` / `writeFailed` on the result. Retries on the next tick.
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
