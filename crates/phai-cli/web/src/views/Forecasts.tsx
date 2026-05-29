import { queryDb } from '@livestore/livestore'
import { useQuery, useClientDocument } from '@livestore/react'
import { useMemo, useState } from 'react'
import { tables } from '../livestore/schema'
import { api } from '../bridge/api'
import { useForecastsSeed } from '../bridge/sync'
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

const forecasts$ = queryDb(tables.forecasts.orderBy('dueDate', 'asc'))
const templates$ = queryDb(tables.forecastTemplates.orderBy('description', 'asc'))
const categories$ = queryDb(tables.categories.orderBy('id', 'asc'))
const accounts$ = queryDb(tables.accounts.orderBy('label', 'asc'))

const STATUSES = ['ativo', 'realizado', 'descartado']

/**
 * Forecasts view — planned cash movements, proposed templates, and a manual
 * add form. Mutations (accept/dismiss/create) go straight to the bridge, then
 * re-seed.
 */
export const Forecasts = () => {
  const [ui, setUi] = useClientDocument(tables.ui)
  const forecasts = useQuery(forecasts$)
  const templates = useQuery(templates$)
  const categories = useQuery(categories$)
  const accounts = useQuery(accounts$)
  const seed = useForecastsSeed(ui.forecastStatusFilter)
  const [mutationError, setMutationError] = useState<string | null>(null)

  const run = async (fn: () => Promise<unknown>) => {
    try {
      setMutationError(null)
      await fn()
      seed.reload()
    } catch (e: unknown) {
      setMutationError(String(e))
    }
  }

  const categoryIds = useMemo(() => categories.map((c) => c.id), [categories])

  return (
    <div>
      <ViewHeader title="Previsões" count={forecasts.length} accent={ACCENT} />

      <FilterBar>
        <Label>status</Label>
        <Pill
          accent={ACCENT}
          active={ui.forecastStatusFilter == null}
          onClick={() => setUi({ forecastStatusFilter: null })}
        >
          todos
        </Pill>
        {STATUSES.map((s) => (
          <Pill
            key={s}
            accent={ACCENT}
            active={ui.forecastStatusFilter === s}
            onClick={() => setUi({ forecastStatusFilter: s })}
          >
            {s}
          </Pill>
        ))}
        <Pill accent={ACCENT} onClick={() => seed.reload()}>
          ↻ atualizar
        </Pill>
      </FilterBar>

      {(seed.error || mutationError) && <ErrorNote error={seed.error ?? mutationError ?? ''} />}
      {seed.loading && forecasts.length === 0 && <LoadingNote />}

      <AddForecastForm
        categories={categoryIds}
        accounts={accounts}
        onCreate={(payload) => run(() => api.createForecast(payload))}
      />

      {templates.length > 0 && (
        <section style={{ marginTop: 32 }}>
          <SectionTitle>Modelos propostos</SectionTitle>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {templates.map((t) => (
              <TemplateCard
                key={t.templateId}
                description={t.description}
                kind={t.kind}
                cadence={t.cadence}
                amount={t.amount}
                confidence={t.confidence}
                onAccept={() => run(() => api.acceptForecastTemplate(t.templateId, 6))}
                onDismiss={() => run(() => api.dismissForecastTemplate(t.templateId))}
              />
            ))}
          </div>
        </section>
      )}

      <section style={{ marginTop: 32 }}>
        <SectionTitle>Previsões</SectionTitle>
        {forecasts.length === 0 && !seed.loading ? (
          <EmptyState message="Sem previsões." />
        ) : (
          <ForecastTable rows={forecasts} />
        )}
      </section>
    </div>
  )
}

const SectionTitle = ({ children }: { children: React.ReactNode }) => (
  <h3
    style={{
      fontFamily: 'var(--font-display)',
      fontSize: '1.05rem',
      fontWeight: 500,
      margin: '0 0 14px',
    }}
  >
    {children}
  </h3>
)

const ForecastTable = ({
  rows,
}: {
  rows: ReadonlyArray<{
    forecastId: string
    dueDate: string | null
    description: string
    amount: string
    categoryId: string | null
    status: string
  }>
}) => (
  <Card accent={ACCENT} style={{ padding: 0, overflow: 'hidden' }}>
    <table
      className="mono"
      style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}
    >
      <thead>
        <tr style={{ color: 'var(--muted)', textAlign: 'left' }}>
          <Th>vencimento</Th>
          <Th>descrição</Th>
          <Th>categoria</Th>
          <Th style={{ textAlign: 'right' }}>valor</Th>
          <Th>status</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((f) => (
          <tr key={f.forecastId} style={{ borderTop: '1px solid var(--border)' }}>
            <Td>{f.dueDate ?? '—'}</Td>
            <Td style={{ color: 'var(--white)' }}>{f.description}</Td>
            <Td style={{ color: 'var(--cyan)' }}>{f.categoryId ?? '—'}</Td>
            <Td style={{ textAlign: 'right', color: amountColor(f.amount) }}>
              {formatMoney(f.amount)}
            </Td>
            <Td>{f.status}</Td>
          </tr>
        ))}
      </tbody>
    </table>
  </Card>
)

const Th = ({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) => (
  <th style={{ padding: '12px 14px', fontWeight: 400, ...style }}>{children}</th>
)
const Td = ({ children, style }: { children: React.ReactNode; style?: React.CSSProperties }) => (
  <td style={{ padding: '10px 14px', color: 'var(--muted)', ...style }}>{children}</td>
)

const TemplateCard = (props: {
  description: string
  kind: string | null
  cadence: string | null
  amount: string
  confidence: string | null
  onAccept: () => void
  onDismiss: () => void
}) => (
  <Card accent={ACCENT}>
    <div style={{ display: 'grid', gridTemplateColumns: '1fr auto', gap: 12, alignItems: 'center' }}>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontWeight: 500 }}>{props.description}</div>
        <div className="mono" style={{ color: 'var(--muted)', fontSize: 12, marginTop: 4 }}>
          {[props.kind, props.cadence, props.confidence != null ? `conf ${props.confidence}` : null]
            .filter(Boolean)
            .join(' · ') || '—'}
        </div>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 8 }}>
        <span className="mono" style={{ color: amountColor(props.amount), fontWeight: 500 }}>
          {formatMoney(props.amount)}
        </span>
        <div style={{ display: 'flex', gap: 8 }}>
          <Pill accent={ACCENT} active onClick={props.onAccept}>
            aceitar
          </Pill>
          <Pill onClick={props.onDismiss}>descartar</Pill>
        </div>
      </div>
    </div>
  </Card>
)

const AddForecastForm = ({
  categories,
  accounts,
  onCreate,
}: {
  categories: string[]
  accounts: ReadonlyArray<{ id: string; label: string }>
  onCreate: (payload: {
    description: string
    amount: string
    due_date?: string
    category_id?: string
    account_id?: string
  }) => void
}) => {
  const [description, setDescription] = useState('')
  const [amount, setAmount] = useState('')
  const [outflow, setOutflow] = useState(true)
  const [dueDate, setDueDate] = useState('')
  const [categoryId, setCategoryId] = useState('')
  const [accountId, setAccountId] = useState('')

  // The bridge derives the forecast idempotency key from due_date, so it is
  // required here — a null due date is rejected with a 500.
  const canSubmit = description.trim() !== '' && amount.trim() !== '' && dueDate !== ''

  const submit = () => {
    if (!canSubmit) return
    const magnitude = amount.replace(/^-/, '').trim()
    const signed = outflow ? `-${magnitude}` : magnitude
    onCreate({
      description: description.trim(),
      amount: signed,
      due_date: dueDate,
      category_id: categoryId || undefined,
      account_id: accountId || undefined,
    })
    setDescription('')
    setAmount('')
    setDueDate('')
    setCategoryId('')
    setAccountId('')
  }

  return (
    <Card accent={ACCENT} style={{ marginTop: 8 }}>
      <SectionTitle>Nova previsão</SectionTitle>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
        <Field label="descrição" full>
          <TextInput
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="ex: aluguel"
            style={{ width: '100%' }}
          />
        </Field>
        <Field label="valor">
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <Pill accent={ACCENT} active={outflow} onClick={() => setOutflow(true)}>
              saída
            </Pill>
            <Pill accent="var(--green)" active={!outflow} onClick={() => setOutflow(false)}>
              entrada
            </Pill>
            <TextInput
              inputMode="decimal"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
              placeholder="0,00"
              style={{ width: 110 }}
            />
          </div>
        </Field>
        <Field label="vencimento">
          <TextInput type="date" value={dueDate} onChange={(e) => setDueDate(e.target.value)} />
        </Field>
        <Field label="categoria">
          <Select value={categoryId} onChange={(e) => setCategoryId(e.target.value)}>
            <option value="">— sem categoria</option>
            {categories.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="conta">
          <Select value={accountId} onChange={(e) => setAccountId(e.target.value)}>
            <option value="">— sem conta</option>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.label || a.id}
              </option>
            ))}
          </Select>
        </Field>
      </div>
      <div style={{ marginTop: 14 }}>
        <Pill accent={ACCENT} active={canSubmit} onClick={submit}>
          adicionar →
        </Pill>
      </div>
    </Card>
  )
}

const Field = ({
  label,
  children,
  full,
}: {
  label: string
  children: React.ReactNode
  full?: boolean
}) => (
  <label
    style={{
      display: 'flex',
      flexDirection: 'column',
      gap: 6,
      gridColumn: full ? '1 / -1' : undefined,
    }}
  >
    <Label>{label}</Label>
    {children}
  </label>
)
