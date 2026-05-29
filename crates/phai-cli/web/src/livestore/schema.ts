import {
  Events,
  makeSchema,
  Schema,
  SessionIdSymbol,
  State,
} from '@livestore/livestore'

/**
 * phai web — LiveStore schema (client-only).
 *
 * Two kinds of state live here:
 *  - **Server reads** (transactions to review, categories, accounts) are seeded
 *    from the Rust bridge via `*Seeded` events and materialised into read
 *    tables. The bridge is the system of record (BigQuery/SQLite).
 *  - **User writes** (a review submitted in the UI) are event-sourced as
 *    `reviewSubmitted`, materialised into `pendingWrites` (the flush queue) and
 *    an optimistic `reviewOverlay`. The background flusher (bridge/sync.ts)
 *    POSTs pending rows to the bridge and emits `writeAcked` on success.
 *
 * No LiveStore sync backend is configured — see ADR on the client-only design.
 */

export const tables = {
  // Review queue + general transactions, seeded from the bridge.
  transactions: State.SQLite.table({
    name: 'transactions',
    columns: {
      id: State.SQLite.text({ primaryKey: true }),
      accountId: State.SQLite.text({ default: '' }),
      postedAt: State.SQLite.text({ default: '' }),
      amount: State.SQLite.text({ default: '0' }), // decimal-as-string, never float
      rawDescription: State.SQLite.text({ default: '' }),
      description: State.SQLite.text({ nullable: true }),
      merchantName: State.SQLite.text({ nullable: true }),
      purpose: State.SQLite.text({ nullable: true }),
      categoryId: State.SQLite.text({ nullable: true }),
      month: State.SQLite.text({ default: '' }), // YYYY-MM, for filtering
    },
  }),

  // Optimistic overlay of the user's edits, applied on top of `transactions`
  // until the bridge acks them.
  reviewOverlay: State.SQLite.table({
    name: 'reviewOverlay',
    columns: {
      transactionId: State.SQLite.text({ primaryKey: true }),
      description: State.SQLite.text({ nullable: true }),
      merchantName: State.SQLite.text({ nullable: true }),
      purpose: State.SQLite.text({ nullable: true }),
      categoryId: State.SQLite.text({ nullable: true }),
    },
  }),

  // The flush queue: one row per unsynced review submission.
  pendingWrites: State.SQLite.table({
    name: 'pendingWrites',
    columns: {
      writeId: State.SQLite.text({ primaryKey: true }),
      transactionId: State.SQLite.text({ default: '' }),
      payload: State.SQLite.json({ default: {} }), // HumanReviewPatch shape
      createdAt: State.SQLite.integer({ default: 0 }),
      attempts: State.SQLite.integer({ default: 0 }),
      lastError: State.SQLite.text({ nullable: true }),
    },
  }),

  categories: State.SQLite.table({
    name: 'categories',
    columns: {
      id: State.SQLite.text({ primaryKey: true }),
    },
  }),

  accounts: State.SQLite.table({
    name: 'accounts',
    columns: {
      id: State.SQLite.text({ primaryKey: true }),
      label: State.SQLite.text({ default: '' }),
      owner: State.SQLite.text({ default: '' }),
    },
  }),

  // Cash-evolution chart: one row per month, seeded from /api/chart.
  // All monetary fields are decimal-as-string, never float.
  chartMonths: State.SQLite.table({
    name: 'chartMonths',
    columns: {
      label: State.SQLite.text({ primaryKey: true }), // YYYY-MM, the x axis key
      inflows: State.SQLite.text({ default: '0' }),
      outflows: State.SQLite.text({ default: '0' }),
      forecastInflowsRemaining: State.SQLite.text({ default: '0' }),
      forecastOutflowsRemaining: State.SQLite.text({ default: '0' }),
      closingBalance: State.SQLite.text({ default: '0' }),
      projectedClosingBalance: State.SQLite.text({ default: '0' }),
      isFuture: State.SQLite.integer({ default: 0 }), // 0/1 — SQLite has no bool
      ordinal: State.SQLite.integer({ default: 0 }), // preserves bridge order
    },
  }),

  // Forecasts (planned cash movements), seeded from /api/forecasts.
  forecasts: State.SQLite.table({
    name: 'forecasts',
    columns: {
      forecastId: State.SQLite.text({ primaryKey: true }),
      dueDate: State.SQLite.text({ nullable: true }),
      description: State.SQLite.text({ default: '' }),
      amount: State.SQLite.text({ default: '0' }), // decimal-as-string
      categoryId: State.SQLite.text({ nullable: true }),
      accountId: State.SQLite.text({ nullable: true }),
      status: State.SQLite.text({ default: '' }),
    },
  }),

  // Proposed forecast templates (recurring patterns), seeded from
  // /api/forecast-templates. Accept/Dismiss act through the bridge.
  forecastTemplates: State.SQLite.table({
    name: 'forecastTemplates',
    columns: {
      templateId: State.SQLite.text({ primaryKey: true }),
      description: State.SQLite.text({ default: '' }),
      kind: State.SQLite.text({ nullable: true }),
      cadence: State.SQLite.text({ nullable: true }),
      amount: State.SQLite.text({ default: '0' }), // decimal-as-string
      status: State.SQLite.text({ default: '' }),
      confidence: State.SQLite.text({ nullable: true }),
    },
  }),

  // Session-local UI state (current tab, filters, selection cursor).
  ui: State.SQLite.clientDocument({
    name: 'ui',
    schema: Schema.Struct({
      tab: Schema.Literal('review', 'cashflow', 'forecasts'),
      monthFilter: Schema.NullOr(Schema.String),
      ownerFilter: Schema.NullOr(Schema.String),
      accountFilter: Schema.NullOr(Schema.String),
      merchantFilter: Schema.NullOr(Schema.String),
      categoryFilter: Schema.NullOr(Schema.String),
      includeReviewed: Schema.Boolean,
      cursor: Schema.Number,
      // Cashflow controls
      monthsBack: Schema.Number,
      monthsAhead: Schema.Number,
      // Forecasts controls
      forecastStatusFilter: Schema.NullOr(Schema.String),
    }),
    default: {
      id: SessionIdSymbol,
      value: {
        tab: 'review',
        monthFilter: null,
        ownerFilter: null,
        accountFilter: null,
        merchantFilter: null,
        categoryFilter: null,
        includeReviewed: false,
        cursor: 0,
        monthsBack: 6,
        monthsAhead: 6,
        forecastStatusFilter: null,
      },
    },
  }),
}

const TxRow = Schema.Struct({
  id: Schema.String,
  accountId: Schema.String,
  postedAt: Schema.String,
  amount: Schema.String,
  rawDescription: Schema.String,
  description: Schema.NullOr(Schema.String),
  merchantName: Schema.NullOr(Schema.String),
  purpose: Schema.NullOr(Schema.String),
  categoryId: Schema.NullOr(Schema.String),
  month: Schema.String,
})

const ReviewPatch = Schema.Struct({
  description: Schema.NullOr(Schema.String),
  merchantName: Schema.NullOr(Schema.String),
  purpose: Schema.NullOr(Schema.String),
  categoryId: Schema.NullOr(Schema.String),
})

const ChartMonth = Schema.Struct({
  label: Schema.String,
  inflows: Schema.String,
  outflows: Schema.String,
  forecastInflowsRemaining: Schema.String,
  forecastOutflowsRemaining: Schema.String,
  closingBalance: Schema.String,
  projectedClosingBalance: Schema.String,
  isFuture: Schema.Number, // 0/1
  ordinal: Schema.Number,
})

const ForecastRow = Schema.Struct({
  forecastId: Schema.String,
  dueDate: Schema.NullOr(Schema.String),
  description: Schema.String,
  amount: Schema.String,
  categoryId: Schema.NullOr(Schema.String),
  accountId: Schema.NullOr(Schema.String),
  status: Schema.String,
})

const ForecastTemplateRow = Schema.Struct({
  templateId: Schema.String,
  description: Schema.String,
  kind: Schema.NullOr(Schema.String),
  cadence: Schema.NullOr(Schema.String),
  amount: Schema.String,
  status: Schema.String,
  confidence: Schema.NullOr(Schema.String),
})

export const events = {
  // ── Server reads → seed events ──────────────────────────────────────────
  queueSeeded: Events.synced({
    name: 'v1.QueueSeeded',
    schema: Schema.Struct({ rows: Schema.Array(TxRow) }),
  }),
  categoriesSeeded: Events.synced({
    name: 'v1.CategoriesSeeded',
    schema: Schema.Struct({ ids: Schema.Array(Schema.String) }),
  }),
  accountsSeeded: Events.synced({
    name: 'v1.AccountsSeeded',
    schema: Schema.Struct({
      rows: Schema.Array(
        Schema.Struct({
          id: Schema.String,
          label: Schema.String,
          owner: Schema.String,
        }),
      ),
    }),
  }),
  chartSeeded: Events.synced({
    name: 'v1.ChartSeeded',
    schema: Schema.Struct({ months: Schema.Array(ChartMonth) }),
  }),
  forecastsSeeded: Events.synced({
    name: 'v1.ForecastsSeeded',
    schema: Schema.Struct({ rows: Schema.Array(ForecastRow) }),
  }),
  forecastTemplatesSeeded: Events.synced({
    name: 'v1.ForecastTemplatesSeeded',
    schema: Schema.Struct({ rows: Schema.Array(ForecastTemplateRow) }),
  }),

  // ── User writes → review flow ───────────────────────────────────────────
  reviewSubmitted: Events.synced({
    name: 'v1.ReviewSubmitted',
    schema: Schema.Struct({
      writeId: Schema.String,
      transactionId: Schema.String,
      patch: ReviewPatch,
      submittedAt: Schema.Number,
    }),
  }),
  writeAcked: Events.synced({
    name: 'v1.WriteAcked',
    schema: Schema.Struct({ writeId: Schema.String }),
  }),
  writeFailed: Events.synced({
    name: 'v1.WriteFailed',
    schema: Schema.Struct({ writeId: Schema.String, error: Schema.String }),
  }),

  uiSet: tables.ui.set,
}

const materializers = State.SQLite.materializers(events, {
  'v1.QueueSeeded': ({ rows }) => [
    tables.transactions.delete(),
    ...rows.map((r) => tables.transactions.insert(r)),
  ],
  'v1.CategoriesSeeded': ({ ids }) => [
    tables.categories.delete(),
    ...ids.map((id) => tables.categories.insert({ id })),
  ],
  'v1.AccountsSeeded': ({ rows }) => [
    tables.accounts.delete(),
    ...rows.map((r) => tables.accounts.insert(r)),
  ],
  'v1.ChartSeeded': ({ months }) => [
    tables.chartMonths.delete(),
    ...months.map((m) => tables.chartMonths.insert(m)),
  ],
  'v1.ForecastsSeeded': ({ rows }) => [
    tables.forecasts.delete(),
    ...rows.map((r) => tables.forecasts.insert(r)),
  ],
  'v1.ForecastTemplatesSeeded': ({ rows }) => [
    tables.forecastTemplates.delete(),
    ...rows.map((r) => tables.forecastTemplates.insert(r)),
  ],
  'v1.ReviewSubmitted': ({ writeId, transactionId, patch, submittedAt }) => [
    tables.pendingWrites.insert({
      writeId,
      transactionId,
      payload: patch,
      createdAt: submittedAt,
      attempts: 0,
    }),
    // optimistic overlay so the row reflects the edit immediately
    tables.reviewOverlay
      .insert({ transactionId, ...patch })
      .onConflict('transactionId', 'replace'),
  ],
  'v1.WriteAcked': ({ writeId }) =>
    tables.pendingWrites.delete().where({ writeId }),
  'v1.WriteFailed': ({ writeId, error }) =>
    tables.pendingWrites.update({ lastError: error }).where({ writeId }),
})

const state = State.SQLite.makeState({ tables, materializers })
export const schema = makeSchema({ events, state })
