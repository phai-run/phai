import { queryDb } from '@livestore/livestore'
import { useStore, useQuery } from '@livestore/react'
import { useState } from 'react'
import { events, tables } from '../livestore/schema'

const queue$ = queryDb(tables.transactions.orderBy('postedAt', 'desc'))
const overlay$ = queryDb(tables.reviewOverlay)
const categories$ = queryDb(tables.categories.orderBy('id', 'asc'))

/**
 * Review queue — the workhorse ported from the discontinued TUI.
 * v1 scaffold: list the queue, assign a category inline, commit a
 * `reviewSubmitted` event (optimistic + queued for the bridge).
 * Filters, keyboard nav, and anatomy edits land in a follow-up.
 */
export const Review = () => {
  const { store } = useStore()
  const rows = useQuery(queue$)
  const overlay = useQuery(overlay$)
  const categories = useQuery(categories$)
  const overlayById = new Map(overlay.map((o) => [o.transactionId, o]))

  if (rows.length === 0) {
    return <Empty />
  }

  return (
    <div>
      <h2 style={{ fontFamily: 'var(--font-display)' }}>
        Revisão <span style={{ color: 'var(--muted)', fontSize: '0.7em' }}>{rows.length}</span>
      </h2>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {rows.map((tx) => {
          const o = overlayById.get(tx.id)
          const category = o?.categoryId ?? tx.categoryId
          return (
            <Row
              key={tx.id}
              id={tx.id}
              description={o?.description ?? tx.description ?? tx.rawDescription}
              amount={tx.amount}
              postedAt={tx.postedAt}
              category={category}
              categories={categories.map((c) => c.id)}
              onCategory={(categoryId) =>
                store.commit(
                  events.reviewSubmitted({
                    writeId: crypto.randomUUID(),
                    transactionId: tx.id,
                    patch: { description: null, merchantName: null, purpose: null, categoryId },
                    submittedAt: Date.now(),
                  }),
                )
              }
            />
          )
        })}
      </div>
    </div>
  )
}

const Row = (props: {
  id: string
  description: string
  amount: string
  postedAt: string
  category: string | null
  categories: string[]
  onCategory: (c: string) => void
}) => {
  const [value, setValue] = useState(props.category ?? '')
  const negative = props.amount.trim().startsWith('-')
  return (
    <div
      style={{
        background: 'var(--surface)',
        border: '1px solid var(--border)',
        borderRadius: 'var(--radius-lg)',
        padding: 'var(--card-pad)',
        display: 'grid',
        gridTemplateColumns: '1fr auto',
        gap: 12,
        alignItems: 'center',
      }}
    >
      <div>
        <div style={{ fontWeight: 500 }}>{props.description}</div>
        <div className="mono" style={{ color: 'var(--muted)', fontSize: 12 }}>
          {props.postedAt}
        </div>
      </div>
      <div style={{ textAlign: 'right' }}>
        <div
          className="mono"
          style={{ color: negative ? 'var(--rose)' : 'var(--green)', fontWeight: 500 }}
        >
          {props.amount}
        </div>
        <input
          list="phai-categories"
          value={value}
          placeholder="categoria…"
          onChange={(e) => setValue(e.target.value)}
          onBlur={() => value && value !== props.category && props.onCategory(value)}
          className="mono"
          style={{
            marginTop: 6,
            background: 'var(--bg)',
            color: 'var(--cyan)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius-sm)',
            padding: '4px 8px',
            fontSize: 12,
            width: 180,
          }}
        />
        <datalist id="phai-categories">
          {props.categories.map((c) => (
            <option key={c} value={c} />
          ))}
        </datalist>
      </div>
    </div>
  )
}

const Empty = () => (
  <div style={{ textAlign: 'center', padding: '80px 0', color: 'var(--muted)' }}>
    <span className="phi" style={{ fontSize: '3rem' }}>
      φ
    </span>
    <p className="mono">Sem pendências para revisar.</p>
  </div>
)
