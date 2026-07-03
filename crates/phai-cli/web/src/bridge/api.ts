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
	id: string;
	accountId: string;
	postedAt: string;
	amount: string;
	rawDescription: string;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
	month: string;
	paymentStatus: string;
	installmentMarker: string | null;
	reviewed: boolean;
	isInstallment: boolean;
	isSubscription: boolean;
	commitmentTier: string | null;
}

export interface AccountRow {
	id: string;
	label: string;
	owner: string;
	/** "bank" = checking account, "credit" = card, … */
	accountType?: string;
	/** Latest known balance (decimal string); null when no snapshot yet. */
	balance?: string | null;
}

export interface CardRow {
	accountId: string;
	label: string;
	/** "aberta" = bill with open balance; "fechada" = selected closed cycle. */
	state: "aberta" | "fechada" | "em-dia";
	cycleMonth: string | null;
	total: string;
	openAmount: string;
	dueDate: string | null;
	creditLimit: string | null;
	usedAmount: string | null;
	installmentDebt: string;
	installmentMonthAmount: string;
	installmentEndingAmount: string;
	installmentCount: number;
	installments: CardInstallmentRow[];
}

export interface CardInstallmentRow {
	transactionId: string;
	transactionDate: string;
	description: string;
	amount: string;
	marker: string;
	current: number;
	total: number;
	remaining: number;
	endingThisMonth: boolean;
}

export interface ReviewPatch {
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
	commitmentTier?: string | null;
}

export interface ReviewFlushItem {
	writeId: string;
	transactionId: string;
	patch: ReviewPatch;
}

const json = async <T>(res: Response): Promise<T> => {
	if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
	return (await res.json()) as T;
};

export interface FlushResult {
	acked: string[];
	failed: { writeId: string; error: string }[];
}

/**
 * Cash-evolution chart shape (Rust `ChartData`). Amounts are decimal strings.
 * Field names mirror the Rust serde shape; we tolerate a couple of aliases the
 * backend might use for the closing balance (see `sync.ts`).
 */
export interface ChartMonthApi {
	label: string;
	month?: string; // YYYY-MM — canonical match key
	inflows: string;
	outflows: string;
	forecast_inflows_remaining?: string;
	forecast_outflows_remaining?: string;
	closing_balance?: string;
	projected_closing_balance?: string;
	is_future?: boolean;
}
export interface ChartData {
	months: ChartMonthApi[];
}

/** Forecast domain record (snake_case). Amount is a decimal string. */
export interface ForecastRecord {
	forecast_id: string;
	due_date: string | null;
	description: string;
	amount: string;
	category_id: string | null;
	account_id: string | null;
	status: string;
	kind?: string;
	draggable?: boolean;
	template_id?: string | null;
	realized_transaction_id?: string | null;
	realized_at?: string | null;
	metadata_json?: Record<string, unknown>;
}

/** Forecast template domain record (snake_case). */
export interface ForecastTemplateRecord {
	template_id: string;
	description: string;
	kind: string | null;
	cadence: string | null;
	amount: string;
	status: string;
	confidence: number | string | null;
}

export interface NewForecast {
	description: string;
	amount: string; // decimal string; negative = saída
	due_date?: string;
	category_id?: string;
	account_id?: string;
	ui_role?: string;
}

/** Budget envelope write: update in place (forecast_id) or create (null). */
export interface EnvelopeUpsert {
	forecast_id: string | null;
	/** Empty on update — the bridge keeps the stored description. */
	description: string;
	amount: string; // decimal string; negative = saída
	due_date: string;
	/** null on update = keep the stored category (inline sheet re-amount). */
	category_id: string | null;
}

/** Planning scenario (ADR-0037), snake_case from the bridge. */
export interface PlanScenarioRecord {
	scenario_id: string;
	name: string;
	description: string | null;
	status: string;
}

/** A scenario's typed delta, snake_case from the bridge. */
export interface PlanChangeRecord {
	change_id: string;
	scenario_id: string;
	kind: string;
	target_forecast_id: string | null;
	target_template_id: string | null;
	month: string | null;
	effective_from: string | null;
	amount: string | null;
	months_count: number | null;
	description: string | null;
	category_id: string | null;
	account_id: string | null;
	status: string;
}

/** Body of POST /api/scenario/change (camelCase; ids client-generated). */
export interface NewScenarioChange {
	changeId: string;
	scenarioId: string;
	kind: string;
	targetForecastId?: string | null;
	targetTemplateId?: string | null;
	month?: string | null;
	effectiveFrom?: string | null;
	amount?: string | null;
	monthsCount?: number | null;
	description?: string | null;
	categoryId?: string | null;
	accountId?: string | null;
}

export interface PromotionSummary {
	applied: { change_id: string; kind: string; description: string }[];
	skipped_orphans: string[];
	forecasts_written: number;
	templates_written: number;
}

export interface BridgeIdentity {
	identity: string;
	backend: string;
	/** Binary version (CARGO_PKG_VERSION). Optional: `vite dev` may talk to an older binary. */
	version?: string;
}

const trimParams = (
	record: Record<string, string | null | undefined>,
): URLSearchParams => {
	const p = new URLSearchParams();
	for (const [k, v] of Object.entries(record)) {
		if (v != null && v !== "") p.set(k, v);
	}
	return p;
};

const postJson = <T>(url: string, body: unknown): Promise<T> =>
	fetch(url, {
		method: "POST",
		headers: { "content-type": "application/json" },
		body: JSON.stringify(body),
	}).then((r) => json<T>(r));

export interface TransactionsPage {
	rows: TxRow[];
	total: number;
	offset: number;
	hasMore: boolean;
}

export const api = {
	identity: (): Promise<BridgeIdentity> =>
		fetch("/api/identity").then((r) => json<BridgeIdentity>(r)),

	/** Fetch one page of the transaction window. Use offset to paginate. */
	transactions: (params: {
		monthsBack: number;
		monthsAhead: number;
		includeReviewed?: boolean;
		limit?: number;
		offset?: number;
	}): Promise<TransactionsPage> =>
		fetch(
			`/api/transactions?${trimParams({
				months_back: String(params.monthsBack),
				months_ahead: String(params.monthsAhead),
				include_reviewed: String(params.includeReviewed ?? true),
				limit: String(params.limit ?? 5000),
				offset: params.offset != null ? String(params.offset) : null,
			})}`,
		).then((r) => json<TransactionsPage>(r)),

	categories: (): Promise<{ ids: string[] }> =>
		fetch("/api/categories").then((r) => json<{ ids: string[] }>(r)),
	accounts: (): Promise<{ rows: AccountRow[] }> =>
		fetch("/api/accounts").then((r) => json<{ rows: AccountRow[] }>(r)),
	cards: (month?: string): Promise<{ rows: CardRow[] }> =>
		fetch(`/api/cards${month ? `?month=${encodeURIComponent(month)}` : ""}`).then(
			(r) => json<{ rows: CardRow[] }>(r),
		),

	chart: (monthsBack: number, monthsAhead: number): Promise<ChartData> =>
		fetch(
			`/api/chart?${trimParams({
				months_back: String(monthsBack),
				months_ahead: String(monthsAhead),
			})}`,
		).then((r) => json<ChartData>(r)),

	forecasts: (filters: {
		status?: string | null;
		from?: string | null;
		until?: string | null;
	}): Promise<{ forecasts: ForecastRecord[] }> =>
		fetch(`/api/forecasts?${trimParams(filters)}`).then((r) =>
			json<{ forecasts: ForecastRecord[] }>(r),
		),

	forecastTemplates: (filters: {
		kind?: string | null;
		status?: string | null;
	}): Promise<{ templates: ForecastTemplateRecord[] }> =>
		fetch(`/api/forecast-templates?${trimParams(filters)}`).then((r) =>
			json<{ templates: ForecastTemplateRecord[] }>(r),
		),

	createForecast: (forecast: NewForecast): Promise<{ forecast_id: string }> =>
		postJson<{ forecast_id: string }>("/api/forecast", forecast),

	/** Upsert a budget envelope (inline amount edit or creation). */
	upsertForecast: (envelope: EnvelopeUpsert): Promise<{ forecastId: string }> =>
		postJson<{ forecastId: string }>("/api/forecast", envelope),

	/** Re-date a forecast (drag-and-drop in Planejamento). */
	moveForecast: (forecastId: string, dueDate: string): Promise<unknown> =>
		postJson("/api/forecast/move", { forecastId, dueDate }),

	deleteForecast: (forecastId: string): Promise<unknown> =>
		postJson("/api/forecast/delete", { forecastId }),

	/**
	 * Soft-discard ANY active forecast, template-materialized included —
	 * the unified sheet's "só em {mês}" removal.
	 */
	discardForecast: (forecastId: string): Promise<unknown> =>
		postJson("/api/forecast/discard", { forecastId }),

	/**
	 * End a recurrence in the baseline: `effectiveFrom` (YYYY-MM) is the first
	 * month without it — the unified sheet's "de {mês} em diante" removal.
	 */
	endForecastTemplate: (
		templateId: string,
		effectiveFrom: string,
	): Promise<unknown> =>
		postJson("/api/forecast-template/end", { templateId, effectiveFrom }),

	settleForecast: (
		forecastId: string,
		transactionId: string,
	): Promise<unknown> =>
		postJson("/api/forecast/settle", { forecastId, transactionId }),

	acceptForecastTemplate: (
		templateId: string,
		materializeMonths = 6,
	): Promise<unknown> =>
		postJson("/api/forecast-template/accept", {
			template_id: templateId,
			materialize_months: materializeMonths,
		}),

	dismissForecastTemplate: (templateId: string): Promise<unknown> =>
		postJson("/api/forecast-template/dismiss", { template_id: templateId }),

	// ── Planning scenarios (ADR-0037) ───────────────────────────────────────
	scenarios: (status?: string | null): Promise<{ scenarios: PlanScenarioRecord[] }> =>
		fetch(`/api/scenarios?${trimParams({ status })}`).then((r) =>
			json<{ scenarios: PlanScenarioRecord[] }>(r),
		),

	createScenario: (body: {
		scenarioId: string;
		name: string;
		description?: string | null;
	}): Promise<{ scenarioId: string }> =>
		postJson<{ scenarioId: string }>("/api/scenario", body),

	archiveScenario: (scenarioId: string): Promise<unknown> =>
		postJson("/api/scenario/archive", { scenarioId }),

	deleteScenario: (scenarioId: string): Promise<unknown> =>
		postJson("/api/scenario/delete", { scenarioId }),

	scenarioChanges: (
		scenarioId: string,
	): Promise<{ changes: PlanChangeRecord[]; orphaned: string[] }> =>
		fetch(
			`/api/scenario/changes?${trimParams({ scenario_id: scenarioId })}`,
		).then((r) => json<{ changes: PlanChangeRecord[]; orphaned: string[] }>(r)),

	addScenarioChange: (body: NewScenarioChange): Promise<{ changeId: string }> =>
		postJson<{ changeId: string }>("/api/scenario/change", body),

	deleteScenarioChange: (
		changeId: string,
		scenarioId: string,
	): Promise<unknown> =>
		postJson("/api/scenario/change/delete", { changeId, scenarioId }),

	scenarioProjection: (
		scenarioId: string,
		monthsBack: number,
		monthsAhead: number,
	): Promise<ChartData> =>
		fetch(
			`/api/scenario/projection?${trimParams({
				scenario_id: scenarioId,
				months_back: String(monthsBack),
				months_ahead: String(monthsAhead),
			})}`,
		).then((r) => json<ChartData>(r)),

	promoteScenario: (scenarioId: string): Promise<PromotionSummary> =>
		postJson<PromotionSummary>("/api/scenario/promote", { scenarioId }),

	version: (): Promise<VersionStatus> =>
		fetch("/api/version").then((r) => json<VersionStatus>(r)),

	triggerUpdate: (): Promise<UpdateResult> =>
		fetch("/api/update", { method: "POST" }).then((r) =>
			json<UpdateResult>(r),
		),

	/** Apply a batch of committed review writes; returns the writeIds that succeeded. */
	flushReviews: (items: ReviewFlushItem[]): Promise<FlushResult> =>
		postJson<FlushResult>("/api/events", { writes: items }),

	/** Whether this machine has an activated backend yet (onboarding gate). */
	status: (): Promise<ActivationStatus> =>
		fetch("/api/status").then((r) => json<ActivationStatus>(r)),

	/**
	 * Pull fresh transactions from Pluggy (runs the CLI sync under the hood).
	 * Long-running; surfaces the bridge's error message on failure.
	 */
	sync: async (): Promise<SyncResult> => {
		const res = await fetch("/api/sync", { method: "POST" });
		if (!res.ok) {
			const detail = (await res.json().catch(() => null)) as {
				error?: string;
			} | null;
			throw new Error(detail?.error ?? `${res.status} ${res.statusText}`);
		}
		return (await res.json()) as SyncResult;
	},

	/**
	 * Activate this machine from an encrypted invite. Surfaces the bridge's
	 * own error message (e.g. wrong passphrase) so onboarding can show it.
	 */
	activate: async (
		token: string,
		passphrase: string,
	): Promise<ActivateResult> => {
		const res = await fetch("/api/activate", {
			method: "POST",
			headers: { "content-type": "application/json" },
			body: JSON.stringify({ token, passphrase }),
		});
		if (!res.ok) {
			const detail = (await res.json().catch(() => null)) as {
				error?: string;
			} | null;
			throw new Error(detail?.error ?? `${res.status} ${res.statusText}`);
		}
		return (await res.json()) as ActivateResult;
	},
};

export interface VersionStatus {
	currentVersion: string;
	latestVersion: string | null;
	updateAvailable: boolean;
	lastCheck: string | null;
	checking: boolean;
	error: string | null;
}

export interface UpdateResult {
	status: "restarting" | "up_to_date";
	version: string;
}

export interface ActivationStatus {
	activated: boolean;
	label: string | null;
	projectId: string | null;
	datasetId: string | null;
	syncAvailable: boolean;
}

export interface SyncResult {
	new_transactions_count?: number;
	new_transactions?: Array<{
		description?: string | null;
		amount?: string;
		posted_at?: string | null;
		account?: string | null;
	}>;
}

export interface ActivateResult {
	ok: boolean;
	label: string;
}
