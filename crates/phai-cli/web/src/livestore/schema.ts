import {
	Events,
	makeSchema,
	Schema,
	SessionIdSymbol,
	State,
} from "@livestore/livestore";

/**
 * phai web — LiveStore schema (client-only).
 *
 * Two kinds of state live here:
 *  - **Server reads** (the transaction window, categories, accounts, chart,
 *    forecasts) are seeded from the Rust bridge via `*Seeded` events and
 *    materialised into read tables. The bridge is the system of record
 *    (BigQuery/SQLite).
 *  - **User writes** (a review edit, a forecast created, a forecast dragged to a
 *    new month) are event-sourced, materialised into `pendingWrites` (the flush
 *    queue) and an optimistic overlay so the UI reflects them in the same frame.
 *    The background flusher (bridge/sync.ts) drains `pendingWrites`, routing each
 *    row to the right endpoint by its `type`, then emits `writeAcked` /
 *    `writeFailed`.
 *
 * Every sum/filter/month-selection in the UI is computed client-side from these
 * tables — never a network round-trip. No LiveStore sync backend is configured
 * (client-only design, see ADR-0001).
 */

/**
 * Store version — bump on ANY schema change here: table columns, event payload
 * schemas, or clientDocument value schemas. `STORE_ID` namespaces the
 * OPFS-persisted client DB. LiveStore's own schema hash only covers table
 * shapes (a clientDocument value is opaque JSON to it), so a value-schema
 * change silently reuses the old state DB and the old rows fail to decode at
 * query time (the v5.6.0 "no data" regression). Bumping abandons the old store
 * and starts fresh — safe, because the store is a disposable cache re-seeded
 * from the bridge (BigQuery/SQLite is the source of truth).
 * Enforced by __tests__/store-version.test.ts: change the schema and the
 * sentinel fails until you bump STORE_VERSION and re-record the fingerprint.
 */
export const STORE_VERSION = 10;
export const STORE_ID = `phai-s${STORE_VERSION}`;

// Computes current month as "YYYY-MM" for the default selectedMonth.
const currentMonth = (() => {
	const d = new Date();
	return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
})();

export const tables = {
	// The full transaction window, seeded from /api/transactions.
	transactions: State.SQLite.table({
		name: "transactions",
		columns: {
			id: State.SQLite.text({ primaryKey: true }),
			accountId: State.SQLite.text({ default: "" }),
			postedAt: State.SQLite.text({ default: "" }),
			amount: State.SQLite.text({ default: "0" }), // decimal-as-string, never float
			rawDescription: State.SQLite.text({ default: "" }),
			description: State.SQLite.text({ nullable: true }),
			merchantName: State.SQLite.text({ nullable: true }),
			purpose: State.SQLite.text({ nullable: true }),
			categoryId: State.SQLite.text({ nullable: true }),
			month: State.SQLite.text({ default: "" }), // YYYY-MM
			paymentStatus: State.SQLite.text({ default: "" }),
			installmentMarker: State.SQLite.text({ nullable: true }),
			reviewed: State.SQLite.integer({ default: 0 }),
			isInstallment: State.SQLite.integer({ default: 0 }),
			isSubscription: State.SQLite.integer({ default: 0 }),
			commitmentTier: State.SQLite.text({ nullable: true }),
		},
	}),

	// Optimistic overlay of the user's review edits.
	reviewOverlay: State.SQLite.table({
		name: "reviewOverlay",
		columns: {
			transactionId: State.SQLite.text({ primaryKey: true }),
			writeId: State.SQLite.text({ default: "" }),
			description: State.SQLite.text({ nullable: true }),
			merchantName: State.SQLite.text({ nullable: true }),
			purpose: State.SQLite.text({ nullable: true }),
			categoryId: State.SQLite.text({ nullable: true }),
			commitmentTier: State.SQLite.text({ nullable: true }),
		},
	}),

	// The flush queue: one row per unsynced write.
	pendingWrites: State.SQLite.table({
		name: "pendingWrites",
		columns: {
			writeId: State.SQLite.text({ primaryKey: true }),
			type: State.SQLite.text({ default: "review" }),
			transactionId: State.SQLite.text({ default: "" }),
			forecastId: State.SQLite.text({ default: "" }),
			payload: State.SQLite.json({ default: {} }),
			createdAt: State.SQLite.integer({ default: 0 }),
			attempts: State.SQLite.integer({ default: 0 }),
			lastError: State.SQLite.text({ nullable: true }),
		},
	}),

	categories: State.SQLite.table({
		name: "categories",
		columns: {
			id: State.SQLite.text({ primaryKey: true }),
		},
	}),

	accounts: State.SQLite.table({
		name: "accounts",
		columns: {
			id: State.SQLite.text({ primaryKey: true }),
			label: State.SQLite.text({ default: "" }),
			owner: State.SQLite.text({ default: "" }),
		},
	}),

	// Cash-evolution chart: one row per month, seeded from /api/chart.
	chartMonths: State.SQLite.table({
		name: "chartMonths",
		columns: {
			label: State.SQLite.text({ primaryKey: true }),
			month: State.SQLite.text({ default: "" }),
			inflows: State.SQLite.text({ default: "0" }),
			outflows: State.SQLite.text({ default: "0" }),
			forecastInflowsRemaining: State.SQLite.text({ default: "0" }),
			forecastOutflowsRemaining: State.SQLite.text({ default: "0" }),
			closingBalance: State.SQLite.text({ default: "0" }),
			projectedClosingBalance: State.SQLite.text({ default: "0" }),
			isFuture: State.SQLite.integer({ default: 0 }),
			ordinal: State.SQLite.integer({ default: 0 }),
		},
	}),

	// Forecasts (planned cash movements), seeded from /api/forecasts.
	forecasts: State.SQLite.table({
		name: "forecasts",
		columns: {
			forecastId: State.SQLite.text({ primaryKey: true }),
			dueDate: State.SQLite.text({ nullable: true }),
			description: State.SQLite.text({ default: "" }),
			amount: State.SQLite.text({ default: "0" }),
			categoryId: State.SQLite.text({ nullable: true }),
			accountId: State.SQLite.text({ nullable: true }),
			status: State.SQLite.text({ default: "" }),
			kind: State.SQLite.text({ default: "manual" }),
			draggable: State.SQLite.integer({ default: 0 }),
			templateId: State.SQLite.text({ nullable: true }),
			realizedTransactionId: State.SQLite.text({ nullable: true }),
			realizedAt: State.SQLite.text({ nullable: true }),
			metadataJson: State.SQLite.json({ default: {} }),
		},
	}),

	// Optimistic overlay of dragged-forecast re-dating.
	forecastOverlay: State.SQLite.table({
		name: "forecastOverlay",
		columns: {
			forecastId: State.SQLite.text({ primaryKey: true }),
			writeId: State.SQLite.text({ default: "" }),
			dueDate: State.SQLite.text({ nullable: true }),
		},
	}),

	// Proposed forecast templates, seeded from /api/forecast-templates.
	forecastTemplates: State.SQLite.table({
		name: "forecastTemplates",
		columns: {
			templateId: State.SQLite.text({ primaryKey: true }),
			description: State.SQLite.text({ default: "" }),
			kind: State.SQLite.text({ nullable: true }),
			cadence: State.SQLite.text({ nullable: true }),
			amount: State.SQLite.text({ default: "0" }),
			status: State.SQLite.text({ default: "" }),
			confidence: State.SQLite.text({ nullable: true }),
		},
	}),

	// Session-local UI state (month selection, filters, chart compact state).
	ui: State.SQLite.clientDocument({
		name: "ui",
		schema: Schema.Struct({
			// Dashboard: the selected month drives the detail panel below the chart.
			selectedMonth: Schema.NullOr(Schema.String),
			// Month-detail presentation: "planilha" (flat sheet with inline
			// editing, the default), "categorias" (treemap drill-down) or
			// "plano" (planejamento: budgets + cut simulator).
			detailMode: Schema.String,
			// Month-detail filters (all applied client-side over the seeded window).
			ownerFilter: Schema.NullOr(Schema.String),
			accountFilter: Schema.NullOr(Schema.String),
			categoryFilter: Schema.NullOr(Schema.String),
			textFilter: Schema.NullOr(Schema.String), // text search across description/merchant
			installmentsOnly: Schema.Boolean,
			subscriptionsOnly: Schema.Boolean,
			unreviewedOnly: Schema.Boolean,
			uncategorizedOnly: Schema.Boolean,
			forecastStatusFilter: Schema.NullOr(Schema.String),
			// Controllability tier filter (ADR-0030): "locked"|"cancellable"|"variable".
			tierFilter: Schema.NullOr(Schema.String),
		}),
		default: {
			id: SessionIdSymbol,
			value: {
				selectedMonth: currentMonth,
				detailMode: "planilha",
				ownerFilter: null,
				accountFilter: null,
				categoryFilter: null,
				textFilter: null,
				installmentsOnly: false,
				subscriptionsOnly: false,
				unreviewedOnly: false,
				uncategorizedOnly: false,
				forecastStatusFilter: null,
				tierFilter: null,
			},
		},
	}),
};

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
	paymentStatus: Schema.String,
	installmentMarker: Schema.NullOr(Schema.String),
	reviewed: Schema.Number,
	isInstallment: Schema.Number,
	isSubscription: Schema.Number,
	commitmentTier: Schema.optional(Schema.NullOr(Schema.String)),
});

const ReviewPatch = Schema.Struct({
	description: Schema.NullOr(Schema.String),
	merchantName: Schema.NullOr(Schema.String),
	purpose: Schema.NullOr(Schema.String),
	categoryId: Schema.NullOr(Schema.String),
	// Per-transaction tier override (ADR-0032); "" clears, omitted = no change.
	commitmentTier: Schema.optional(Schema.NullOr(Schema.String)),
});

const ChartMonth = Schema.Struct({
	label: Schema.String,
	month: Schema.String,
	inflows: Schema.String,
	outflows: Schema.String,
	forecastInflowsRemaining: Schema.String,
	forecastOutflowsRemaining: Schema.String,
	closingBalance: Schema.String,
	projectedClosingBalance: Schema.String,
	isFuture: Schema.Number,
	ordinal: Schema.Number,
});

const ForecastRow = Schema.Struct({
	forecastId: Schema.String,
	dueDate: Schema.NullOr(Schema.String),
	description: Schema.String,
	amount: Schema.String,
	categoryId: Schema.NullOr(Schema.String),
	accountId: Schema.NullOr(Schema.String),
	status: Schema.String,
	kind: Schema.String,
	draggable: Schema.Number,
	templateId: Schema.NullOr(Schema.String),
	realizedTransactionId: Schema.NullOr(Schema.String),
	realizedAt: Schema.NullOr(Schema.String),
	metadataJson: Schema.Record({ key: Schema.String, value: Schema.Unknown }),
});

const ForecastTemplateRow = Schema.Struct({
	templateId: Schema.String,
	description: Schema.String,
	kind: Schema.NullOr(Schema.String),
	cadence: Schema.NullOr(Schema.String),
	amount: Schema.String,
	status: Schema.String,
	confidence: Schema.NullOr(Schema.String),
});

export const events = {
	// ── Server reads → seed events ──────────────────────────────────────────
	transactionsSeeded: Events.synced({
		name: "v1.TransactionsSeeded",
		schema: Schema.Struct({ rows: Schema.Array(TxRow) }),
	}),
	transactionsPageSeeded: Events.synced({
		name: "v1.TransactionsPageSeeded",
		schema: Schema.Struct({ rows: Schema.Array(TxRow) }),
	}),
	categoriesSeeded: Events.synced({
		name: "v1.CategoriesSeeded",
		schema: Schema.Struct({ ids: Schema.Array(Schema.String) }),
	}),
	accountsSeeded: Events.synced({
		name: "v1.AccountsSeeded",
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
		name: "v1.ChartSeeded",
		schema: Schema.Struct({ months: Schema.Array(ChartMonth) }),
	}),
	forecastsSeeded: Events.synced({
		name: "v1.ForecastsSeeded",
		schema: Schema.Struct({ rows: Schema.Array(ForecastRow) }),
	}),
	forecastTemplatesSeeded: Events.synced({
		name: "v1.ForecastTemplatesSeeded",
		schema: Schema.Struct({ rows: Schema.Array(ForecastTemplateRow) }),
	}),
	bridgeIdentityChanged: Events.synced({
		name: "v1.BridgeIdentityChanged",
		schema: Schema.Struct({
			oldIdentity: Schema.String,
			newIdentity: Schema.String,
		}),
	}),

	// ── User writes ─────────────────────────────────────────────────────────
	reviewSubmitted: Events.synced({
		name: "v1.ReviewSubmitted",
		schema: Schema.Struct({
			writeId: Schema.String,
			transactionId: Schema.String,
			patch: ReviewPatch,
			submittedAt: Schema.Number,
		}),
	}),
	forecastMoved: Events.synced({
		name: "v1.ForecastMoved",
		schema: Schema.Struct({
			writeId: Schema.String,
			forecastId: Schema.String,
			dueDate: Schema.String,
			movedAt: Schema.Number,
		}),
	}),
	forecastCreated: Events.synced({
		name: "v1.ForecastCreated",
		schema: Schema.Struct({
			writeId: Schema.String,
			description: Schema.String,
			amount: Schema.String,
			dueDate: Schema.String,
			categoryId: Schema.NullOr(Schema.String),
			accountId: Schema.NullOr(Schema.String),
			uiRole: Schema.NullOr(Schema.String),
			createdAt: Schema.Number,
		}),
	}),
	// A war-plan goal confirmed as a monthly budget envelope. forecastId ""
	// creates a new envelope; otherwise the existing one is re-amounted.
	forecastEnvelopeUpserted: Events.synced({
		name: "v1.ForecastEnvelopeUpserted",
		schema: Schema.Struct({
			writeId: Schema.String,
			forecastId: Schema.String,
			description: Schema.String,
			amount: Schema.String,
			dueDate: Schema.String,
			categoryId: Schema.String,
			upsertedAt: Schema.Number,
		}),
	}),
	forecastDeleted: Events.synced({
		name: "v1.ForecastDeleted",
		schema: Schema.Struct({
			writeId: Schema.String,
			forecastId: Schema.String,
			deletedAt: Schema.Number,
		}),
	}),
	forecastSettled: Events.synced({
		name: "v1.ForecastSettled",
		schema: Schema.Struct({
			writeId: Schema.String,
			forecastId: Schema.String,
			transactionId: Schema.String,
			predictedAmount: Schema.String,
			actualAmount: Schema.String,
			actualDate: Schema.String,
			actualDescription: Schema.String,
			settledAt: Schema.String,
			settledAtMs: Schema.Number,
		}),
	}),
	forecastCreateAcked: Events.synced({
		name: "v1.ForecastCreateAcked",
		schema: Schema.Struct({
			writeId: Schema.String,
			localForecastId: Schema.String,
			serverForecastId: Schema.String,
			description: Schema.String,
			amount: Schema.String,
			dueDate: Schema.NullOr(Schema.String),
			categoryId: Schema.NullOr(Schema.String),
			accountId: Schema.NullOr(Schema.String),
			status: Schema.String,
			kind: Schema.String,
			draggable: Schema.Number,
			templateId: Schema.NullOr(Schema.String),
			realizedTransactionId: Schema.NullOr(Schema.String),
			realizedAt: Schema.NullOr(Schema.String),
			metadataJson: Schema.Record({
				key: Schema.String,
				value: Schema.Unknown,
			}),
		}),
	}),
	writeAcked: Events.synced({
		name: "v1.WriteAcked",
		schema: Schema.Struct({ writeId: Schema.String }),
	}),
	writeFailed: Events.synced({
		name: "v1.WriteFailed",
		schema: Schema.Struct({
			writeId: Schema.String,
			error: Schema.String,
			attempts: Schema.Number,
		}),
	}),
	writeAbandoned: Events.synced({
		name: "v1.WriteAbandoned",
		schema: Schema.Struct({
			writeId: Schema.String,
			type: Schema.String,
			transactionId: Schema.String,
			forecastId: Schema.String,
			error: Schema.String,
		}),
	}),

	uiSet: tables.ui.set,
};

const materializers = State.SQLite.materializers(events, {
	"v1.TransactionsSeeded": ({ rows }) => [
		tables.transactions.delete(),
		...rows.map((r) => tables.transactions.insert(r)),
	],
	"v1.TransactionsPageSeeded": ({ rows }) =>
		rows.map((r) => tables.transactions.insert(r)),
	"v1.CategoriesSeeded": ({ ids }) => [
		tables.categories.delete(),
		...ids.map((id) => tables.categories.insert({ id })),
	],
	"v1.AccountsSeeded": ({ rows }) => [
		tables.accounts.delete(),
		...rows.map((r) => tables.accounts.insert(r)),
	],
	"v1.ChartSeeded": ({ months }) => [
		tables.chartMonths.delete(),
		...months.map((m) => tables.chartMonths.insert(m)),
	],
	"v1.ForecastsSeeded": ({ rows }) => [
		tables.forecasts.delete(),
		...rows.map((r) => tables.forecasts.insert(r)),
	],
	"v1.ForecastTemplatesSeeded": ({ rows }) => [
		tables.forecastTemplates.delete(),
		...rows.map((r) => tables.forecastTemplates.insert(r)),
	],
	"v1.BridgeIdentityChanged": () => [
		tables.transactions.delete(),
		tables.categories.delete(),
		tables.accounts.delete(),
		tables.chartMonths.delete(),
		tables.forecasts.delete(),
		tables.forecastTemplates.delete(),
		tables.pendingWrites.delete(),
		tables.reviewOverlay.delete(),
		tables.forecastOverlay.delete(),
	],
	"v1.ReviewSubmitted": ({ writeId, transactionId, patch, submittedAt }) => [
		tables.pendingWrites.insert({
			writeId,
			type: "review",
			transactionId,
			payload: patch,
			createdAt: submittedAt,
			attempts: 0,
		}),
		tables.reviewOverlay
			.insert({ transactionId, writeId, ...patch })
			.onConflict("transactionId", "replace"),
	],
	"v1.ForecastMoved": ({ writeId, forecastId, dueDate, movedAt }) => [
		tables.pendingWrites.insert({
			writeId,
			type: "forecastMove",
			forecastId,
			payload: { dueDate },
			createdAt: movedAt,
			attempts: 0,
		}),
		tables.forecastOverlay
			.insert({ forecastId, writeId, dueDate })
			.onConflict("forecastId", "replace"),
	],
	"v1.ForecastCreated": ({
		writeId,
		description,
		amount,
		dueDate,
		categoryId,
		accountId,
		uiRole,
		createdAt,
	}) => [
		tables.pendingWrites.insert({
			writeId,
			type: "forecastCreate",
			payload: {
				description,
				amount,
				due_date: dueDate,
				category_id: categoryId,
				account_id: accountId,
				ui_role: uiRole,
			},
			createdAt,
			attempts: 0,
		}),
		tables.forecasts.insert({
			forecastId: writeId,
			description,
			amount,
			dueDate,
			categoryId,
			accountId,
			status: "active",
			kind: "manual",
			draggable: 1,
			metadataJson: uiRole ? { ui_role: uiRole } : {},
		}),
	],
	"v1.ForecastEnvelopeUpserted": ({
		writeId,
		forecastId,
		description,
		amount,
		dueDate,
		categoryId,
		upsertedAt,
	}) => [
		tables.pendingWrites.insert({
			writeId,
			type: "forecastEnvelope",
			forecastId,
			// drainQueue POSTs this verbatim to /api/forecast (snake_case body).
			payload: {
				forecast_id: forecastId || null,
				description,
				amount,
				due_date: dueDate,
				category_id: categoryId,
			},
			createdAt: upsertedAt,
			attempts: 0,
		}),
		forecastId
			? tables.forecasts.update({ amount, dueDate }).where({ forecastId })
			: tables.forecasts.insert({
					forecastId: writeId,
					description,
					amount,
					dueDate,
					categoryId,
					status: "ativo",
					kind: "manual",
					draggable: 1,
					metadataJson: {},
				}),
	],
	"v1.ForecastDeleted": ({ writeId, forecastId, deletedAt }) => [
		tables.pendingWrites.insert({
			writeId,
			type: "forecastDelete",
			forecastId,
			payload: {},
			createdAt: deletedAt,
			attempts: 0,
		}),
		tables.forecasts
			.update({ status: "descartado" })
			.where({ forecastId }),
	],
	"v1.ForecastSettled": ({
		writeId,
		forecastId,
		transactionId,
		predictedAmount,
		actualAmount,
		actualDate,
		actualDescription,
		settledAt,
		settledAtMs,
	}) => [
		tables.pendingWrites.insert({
			writeId,
			type: "forecastSettle",
			forecastId,
			payload: { transactionId },
			createdAt: settledAtMs,
			attempts: 0,
		}),
		tables.forecasts
			.update({
				amount: actualAmount,
				status: "realizado",
				realizedTransactionId: transactionId,
				realizedAt: settledAt,
				metadataJson: {
					ui_role: "planned_transaction",
					predicted_amount: predictedAmount,
					realized_amount: actualAmount,
					realized_transaction_date: actualDate,
					realized_transaction_description: actualDescription,
					realization_source: "manual",
				},
			})
			.where({ forecastId }),
	],
	"v1.ForecastCreateAcked": ({
		writeId,
		localForecastId,
		serverForecastId,
		description,
		amount,
		dueDate,
		categoryId,
		accountId,
		status,
		kind,
		draggable,
		templateId,
		realizedTransactionId,
		realizedAt,
		metadataJson,
	}) => [
		tables.pendingWrites.delete().where({ writeId }),
		tables.forecasts.delete().where({ forecastId: localForecastId }),
		tables.forecasts.insert({
			forecastId: serverForecastId,
			description,
			amount,
			dueDate,
			categoryId,
			accountId,
			status,
			kind,
			draggable,
			templateId,
			realizedTransactionId,
			realizedAt,
			metadataJson,
		}),
		tables.pendingWrites
			.update({ forecastId: serverForecastId })
			.where({ forecastId: localForecastId }),
		tables.forecastOverlay
			.update({ forecastId: serverForecastId })
			.where({ forecastId: localForecastId }),
	],
	"v1.WriteAcked": ({ writeId }) =>
		tables.pendingWrites.delete().where({ writeId }),
	"v1.WriteFailed": ({ writeId, error, attempts }) =>
		tables.pendingWrites
			.update({ lastError: error, attempts })
			.where({ writeId }),
	"v1.WriteAbandoned": ({ writeId, type, transactionId, forecastId }) => [
		tables.pendingWrites.delete().where({ writeId }),
		...(type === "review" && transactionId
			? [tables.reviewOverlay.delete().where({ transactionId, writeId })]
			: []),
		...(type === "forecastMove" && forecastId
			? [tables.forecastOverlay.delete().where({ forecastId, writeId })]
			: []),
		...(type === "forecastCreate" || (type === "forecastEnvelope" && !forecastId)
			? [tables.forecasts.delete().where({ forecastId: writeId })]
			: []),
	],
});

const state = State.SQLite.makeState({ tables, materializers });
export const schema = makeSchema({ events, state });
