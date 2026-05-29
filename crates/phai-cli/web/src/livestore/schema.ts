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

  // Session-local UI state (current tab, filters, selection cursor).
  ui: State.SQLite.clientDocument({
    name: 'ui',
    schema: Schema.Struct({
      tab: Schema.Literal('review', 'cashflow', 'forecasts'),
      monthFilter: Schema.NullOr(Schema.String),
      ownerFilter: Schema.NullOr(Schema.String),
      merchantFilter: Schema.NullOr(Schema.String),
      categoryFilter: Schema.NullOr(Schema.String),
      includeReviewed: Schema.Boolean,
      cursor: Schema.Number,
    }),
    default: {
      id: SessionIdSymbol,
      value: {
        tab: 'review',
        monthFilter: null,
        ownerFilter: null,
        merchantFilter: null,
        categoryFilter: null,
        includeReviewed: false,
        cursor: 0,
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
