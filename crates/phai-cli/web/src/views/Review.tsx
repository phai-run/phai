import { queryDb } from '@livestore/livestore'
import { useStore, useQuery, useClientDocument } from '@livestore/react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { events, tables } from '../livestore/schema'
import { useReviewQueueSeed } from '../bridge/sync'
import { amountColor, formatMoney } from '../lib/format'
import {
  Card,
  EmptyState,
  ErrorNote,
  FilterBar,
  Label,
  LoadingNote,
  Pill,
  Select,
  TextInput,
  ViewHeader,
} from '../components/ui'

const ACCENT = 'var(--purple)'

const queue$ = queryDb(tables.transactions.orderBy('postedAt', 'desc'))
const overlay$ = queryDb(tables.reviewOverlay)
const categories$ = queryDb(tables.categories.orderBy('id', 'asc'))
const accounts$ = queryDb(tables.accounts.orderBy('label', 'asc'))

interface Patch {
  description: string | null
  merchantName: string | null
  purpose: string | null
  categoryId: string | null
}

/**
 * Review queue — the keyboard-first workhorse ported from the discontinued TUI.
 * Lists transactions to categorize (newest first), filters drive a bridge
 * re-seed, and inline edits (category + human anatomy) commit a `reviewSubmitted`
 * event (optimistic overlay + queued for flush).
 *
 * Keyboard: ↑/↓ move the selection cursor, Enter focuses the selected row's
 * category input. A fast reviewer never needs the mouse.
 */
export const Review = () => {
  const { store } = useStore()
  const [ui, setUi] = useClientDocument(tables.ui)
  const rows = useQuery(queue$)
  const overlay = useQuery(overlay$)
  const categories = useQuery(categories$)
  const accounts = useQuery(accounts$)

  const seed = useReviewQueueSeed({
    month: ui.monthFilter,
    owner: ui.ownerFilter,
    accountId: ui.accountFilter,
    merchant: ui.merchantFilter,
    category: ui.categoryFilter,
    includeReviewed: ui.includeReviewed,
  })

  const overlayById = useMemo(
    () => new Map(overlay.map((o) => [o.transactionId, o])),
    [overlay],
  )

  const categoryIds = useMemo(() => categories.map((c) => c.id), [categories])
  const owners = useMemo(
    () => Array.from(new Set(accounts.map((a) => a.owner).filter(Boolean))),
    [accounts],
  )

  const cursor = Math.min(ui.cursor, Math.max(0, rows.length - 1))
  const focusRef = useRef<(() => void) | null>(null)

  // Keyboard navigation: ↑/↓ move cursor, Enter focuses selected category.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName
      const typing = tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA'
      if (e.key === 'ArrowDown' && !typing) {
        e.preventDefault()
        setUi({ cursor: Math.min(cursor + 1, rows.length - 1) })
      } else if (e.key === 'ArrowUp' && !typing) {
        e.preventDefault()
        setUi({ cursor: Math.max(cursor - 1, 0) })
      } else if (e.key === 'Enter' && !typing) {
        e.preventDefault()
        focusRef.current?.()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [cursor, rows.length, setUi])

  const submit = (transactionId: string, patch: Patch) =>
    store.commit(
      events.reviewSubmitted({
        writeId: crypto.randomUUID(),
        transactionId,
        patch,
        submittedAt: Date.now(),
      }),
    )

  return (
    <div>
      <ViewHeader title="Revisão" count={rows.length} accent={ACCENT} />

      <FilterBar>
        <TextInput
          type="month"
          value={ui.monthFilter ?? ''}
          onChange={(e) => setUi({ monthFilter: e.target.value || null })}
          aria-label="mês"
        />
        <Select
          value={ui.ownerFilter ?? ''}
          onChange={(e) => setUi({ ownerFilter: e.target.value || null })}
          aria-label="responsável"
        >
          <option value="">todos · responsável</option>
          {owners.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </Select>
        <Select
          value={ui.accountFilter ?? ''}
          onChange={(e) => setUi({ accountFilter: e.target.value || null })}
          aria-label="conta"
        >
          <option value="">todas · conta</option>
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>
              {a.label || a.id}
            </option>
          ))}
        </Select>
        <TextInput
          placeholder="merchant…"
          value={ui.merchantFilter ?? ''}
          onChange={(e) => setUi({ merchantFilter: e.target.value || null })}
          aria-label="merchant"
        />
        <TextInput
          list="phai-categories"
          placeholder="categoria…"
          value={ui.categoryFilter ?? ''}
          onChange={(e) => setUi({ categoryFilter: e.target.value || null })}
          style={{ color: 'var(--cyan)' }}
          aria-label="categoria"
        />
        <Pill
          active={ui.includeReviewed}
          accent={ACCENT}
          onClick={() => setUi({ includeReviewed: !ui.includeReviewed })}
        >
          {ui.includeReviewed ? 'todas' : 'pendentes'}
        </Pill>
        <Pill accent={ACCENT} onClick={() => seed.reload()}>
          ↻ atualizar
        </Pill>
      </FilterBar>

      <datalist id="phai-categories">
        {categoryIds.map((c) => (
          <option key={c} value={c} />
        ))}
      </datalist>

      {seed.error && <ErrorNote error={seed.error} />}
      {seed.loading && rows.length === 0 && <LoadingNote />}

      {rows.length === 0 && !seed.loading ? (
        <EmptyState message="Sem pendências para revisar." />
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {rows.map((tx, i) => {
            const o = overlayById.get(tx.id)
            return (
              <ReviewRow
                key={tx.id}
                selected={i === cursor}
                registerFocus={i === cursor ? (fn) => (focusRef.current = fn) : undefined}
                onSelect={() => setUi({ cursor: i })}
                postedAt={tx.postedAt}
                amount={tx.amount}
                rawDescription={tx.rawDescription}
                description={o?.description ?? tx.description}
                merchantName={o?.merchantName ?? tx.merchantName}
                purpose={o?.purpose ?? tx.purpose}
                category={o?.categoryId ?? tx.categoryId}
                categories={categoryIds}
                onSubmit={(patch) => submit(tx.id, patch)}
              />
            )
          })}
        </div>
      )}
    </div>
  )
}

const ReviewRow = (props: {
  selected: boolean
  registerFocus?: (focus: () => void) => void
  onSelect: () => void
  postedAt: string
  amount: string
  rawDescription: string
  description: string | null
  merchantName: string | null
  purpose: string | null
  category: string | null
  categories: string[]
  onSubmit: (patch: Patch) => void
}) => {
  const [expanded, setExpanded] = useState(false)
  const [category, setCategory] = useState(props.category ?? '')
  const [description, setDescription] = useState(props.description ?? '')
  const [merchantName, setMerchantName] = useState(props.merchantName ?? '')
  const [purpose, setPurpose] = useState(props.purpose ?? '')
  const categoryRef = useRef<HTMLInputElement>(null)

  // Keep local edit state in sync when the overlay/seed changes underneath.
  useEffect(() => setCategory(props.category ?? ''), [props.category])
  useEffect(() => setDescription(props.description ?? ''), [props.description])
  useEffect(() => setMerchantName(props.merchantName ?? ''), [props.merchantName])
  useEffect(() => setPurpose(props.purpose ?? ''), [props.purpose])

  // Let the parent focus this row's category input on Enter.
  useEffect(() => {
    props.registerFocus?.(() => {
      setExpanded(true)
      categoryRef.current?.focus()
    })
  }, [props.registerFocus])

  const display = props.description || props.merchantName || props.rawDescription

  const commitCategory = () => {
    const next = category.trim() || null
    if (next !== (props.category ?? null)) {
      props.onSubmit({
        description: null,
        merchantName: null,
        purpose: null,
        categoryId: next,
      })
    }
  }

  const commitAnatomy = () => {
    props.onSubmit({
      description: description.trim() || null,
      merchantName: merchantName.trim() || null,
      purpose: purpose.trim() || null,
      categoryId: category.trim() || null,
    })
    setExpanded(false)
  }

  return (
    <Card selected={props.selected} accent={ACCENT} style={{ cursor: 'default' }}>
      <div
        onClick={props.onSelect}
        style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 12, alignItems: 'center' }}
      >
        <div style={{ minWidth: 0 }}>
          <div style={{ fontWeight: 500, overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {display}
          </div>
          <div className="mono" style={{ color: 'var(--muted)', fontSize: 12 }}>
            {props.postedAt}
            {props.merchantName ? ` · ${props.merchantName}` : ''}
          </div>
        </div>
        <div style={{ textAlign: 'right' }}>
          <div className="mono" style={{ color: amountColor(props.amount), fontWeight: 500 }}>
            {formatMoney(props.amount)}
          </div>
          <input
            ref={categoryRef}
            list="phai-categories"
            value={category}
            placeholder="categoria…"
            onChange={(e) => setCategory(e.target.value)}
            onFocus={props.onSelect}
            onBlur={commitCategory}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                ;(e.target as HTMLInputElement).blur()
              }
            }}
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
        </div>
      </div>

      <button
        onClick={() => setExpanded((v) => !v)}
        className="mono"
        style={{
          marginTop: 10,
          background: 'transparent',
          border: 'none',
          color: 'var(--muted)',
          fontSize: 11,
          cursor: 'pointer',
          padding: 0,
        }}
      >
        {expanded ? '◇ ocultar anatomia' : '◇ editar anatomia'}
      </button>

      {expanded && (
        <div
          style={{
            marginTop: 12,
            display: 'grid',
            gridTemplateColumns: '1fr',
            gap: 10,
            borderTop: '1px solid var(--border)',
            paddingTop: 12,
          }}
        >
          <AnatomyField label="descrição" value={description} onChange={setDescription} />
          <AnatomyField label="merchant" value={merchantName} onChange={setMerchantName} />
          <AnatomyField label="propósito" value={purpose} onChange={setPurpose} />
          <div style={{ display: 'flex', gap: 8 }}>
            <Pill accent={ACCENT} active onClick={commitAnatomy}>
              salvar →
            </Pill>
          </div>
        </div>
      )}
    </Card>
  )
}

const AnatomyField = ({
  label,
  value,
  onChange,
}: {
  label: string
  value: string
  onChange: (v: string) => void
}) => (
  <label style={{ display: 'grid', gridTemplateColumns: '110px 1fr', gap: 10, alignItems: 'center' }}>
    <Label>{label}</Label>
    <TextInput value={value} onChange={(e) => onChange(e.target.value)} />
  </label>
)
