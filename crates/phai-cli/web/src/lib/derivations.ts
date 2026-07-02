/**
 * Pure derivation functions extracted from view components.
 *
 * These operate on plain data (arrays, maps, strings) with zero dependencies on
 * React, LiveStore, or the DOM. They are the single source of truth for how
 * transactions are filtered, grouped, and categorised — every view must derive
 * its state through these functions so UI behaviour and test assertions stay in
 * sync.
 *
 * Amounts are decimal-as-string (rust_decimal serde). Sums use integer-cent
 * math via toCents / sumAmounts from ./format.ts.
 */

import { isNegative, sumAmounts, toCents } from "./format";

// ── Types ──────────────────────────────────────────────────────────────────

/** A transaction row as it flows through the derivation pipeline. */
export interface TxView {
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
	installmentMarker?: string | null;
	accountLabel?: string;
	reviewed: number; // 0/1
	isInstallment: number; // 0/1
	isSubscription: number; // 0/1
	/** Per-transaction tier override (ADR-0032); null/absent = derived. */
	commitmentTier?: string | null;
}

/** An optimistic overlay applied on top of a seed transaction. */
export interface ReviewOverlay {
	transactionId: string;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
	commitmentTier?: string | null;
}

/**
 * Controllability axis for planning (ADR-0030). The four `forecast_template`
 * kinds collapse to three tiers by the question "can I cut this?":
 *   - `locked`      — installments + fixed monthly bills; no short-term margin
 *   - `cancellable` — subscriptions; cancel at will
 *   - `variable`    — discretionary spend; where a budget bites
 */
export type CommitmentTier = "locked" | "cancellable" | "variable";

/** Tiers in cut-margin order (least → most controllable). */
export const COMMITMENT_TIERS: readonly CommitmentTier[] = [
	"locked",
	"cancellable",
	"variable",
];

/** Short labels for the planning/sheet/treemap surfaces (UI chrome is English). */
export const COMMITMENT_TIER_LABELS: Record<CommitmentTier, string> = {
	locked: "locked",
	cancellable: "cancellable",
	variable: "variable",
};

/** Filters that can be applied to a transaction list. */
export interface TxFilters {
	/** Filter by account id (exact match). */
	accountFilter: string | null;
	/** Filter by account owner (exact match). */
	ownerFilter: string | null;
	/** Filter by category id substring (case-insensitive). */
	categoryFilter: string | null;
	/** Free-text search across description, merchant, rawDescription, category. */
	textFilter: string | null;
	/** Show only installments. */
	installmentsOnly: boolean;
	/** Show only subscriptions. */
	subscriptionsOnly: boolean;
	/** Show only unreviewed transactions. */
	unreviewedOnly: boolean;
	/** Show only transactions with no effective category. */
	uncategorizedOnly?: boolean;
	/** Show only one controllability tier (ADR-0030); null/undefined = all. */
	tierFilter?: CommitmentTier | null;
}

/** Result of parsing a category id like "alimentacao:mercado". */
export interface ParsedCategory {
	parent: string;
	sub: string | null;
}

// ── Category parsing ───────────────────────────────────────────────────────

/**
 * Parse a colon-separated category id into parent + sub.
 *
 * Examples:
 *   "alimentacao:mercado" → { parent: "alimentacao", sub: "mercado" }
 *   "moradia"             → { parent: "moradia",     sub: null }
 *   null                  → { parent: "—",           sub: null }
 *   "a:b:c"               → { parent: "a",           sub: "b:c" }
 */
export const parseCategory = (categoryId: string | null): ParsedCategory => {
	if (categoryId == null || categoryId === "") {
		return { parent: "—", sub: null };
	}
	const idx = categoryId.indexOf(":");
	if (idx === -1) {
		return { parent: categoryId, sub: null };
	}
	return { parent: categoryId.slice(0, idx), sub: categoryId.slice(idx + 1) };
};

// ── Overlay application ────────────────────────────────────────────────────

/**
 * Build a lookup Map from transaction id to overlay.
 */
export const buildOverlayMap = (
	overlays: ReadonlyArray<ReviewOverlay>,
): Map<string, ReviewOverlay> =>
	new Map(overlays.map((o) => [o.transactionId, o]));

/**
 * Resolve the effective category for a transaction, overlay-first.
 */
export const effectiveCategory = (
	tx: TxView,
	overlayMap: Map<string, ReviewOverlay>,
): string | null => {
	const o = overlayMap.get(tx.id);
	return o?.categoryId ?? tx.categoryId;
};

/**
 * Merge a transaction with its optimistic review overlay so every downstream
 * consumer (sheet labels, treemap, filters, sums) sees the edited values — not
 * just the category. Without this, an edited description/merchant only shows in
 * the modal (which reads the overlay) while the list keeps the stale seed value
 * until a full re-seed. `??` keeps unset patch fields (null) from clobbering the
 * seed; an explicit empty string still clears.
 */
export const effectiveTx = (
	tx: TxView,
	overlayMap: Map<string, ReviewOverlay>,
): TxView => {
	const o = overlayMap.get(tx.id);
	if (!o) return tx;
	return {
		...tx,
		description: o.description ?? tx.description,
		merchantName: o.merchantName ?? tx.merchantName,
		purpose: o.purpose ?? tx.purpose,
		categoryId: o.categoryId ?? tx.categoryId,
		commitmentTier: o.commitmentTier ?? tx.commitmentTier,
	};
};

// ── Account lookup ─────────────────────────────────────────────────────────

export interface AccountInfo {
	id: string;
	label: string;
	owner: string;
}

export const buildAccountMap = (
	accounts: ReadonlyArray<AccountInfo>,
): Map<string, AccountInfo> => new Map(accounts.map((a) => [a.id, a]));

// ── Commitment tier (ADR-0030) ─────────────────────────────────────────────

const EMPTY_FIXED: ReadonlySet<string> = new Set<string>();

/**
 * Build the set of parent categories the user treats as "fixed" — derived from
 * seeded fixed-kind forecasts, which carry the category of every confirmed
 * fixed monthly bill (rent, therapy, school…). The classification lives in
 * runtime data (forecasts), never hardcoded in shared source (ADR-0008).
 */
export const fixedCategoriesFromForecasts = (
	forecasts: ReadonlyArray<{ kind: string; categoryId: string | null }>,
): Set<string> => {
	const set = new Set<string>();
	for (const f of forecasts) {
		if (f.kind === "fixed" && f.categoryId) {
			set.add(parseCategory(f.categoryId).parent);
		}
	}
	return set;
};

/**
 * Classify a transaction on the controllability axis (ADR-0030). The
 * per-transaction installment/subscription flags decide first (a streaming
 * subscription stays `cancellable` even inside a "fixed" category); otherwise
 * the parent category is checked against the user's fixed-category set.
 */
export const commitmentTier = (
	tx: TxView,
	fixedCategories: ReadonlySet<string> = EMPTY_FIXED,
	overlayMap?: Map<string, ReviewOverlay>,
): CommitmentTier => {
	// Manual per-transaction override wins over every derived signal (ADR-0032);
	// the optimistic overlay (an unflushed edit) takes precedence over the seed.
	const override = overlayMap?.get(tx.id)?.commitmentTier ?? tx.commitmentTier;
	if (
		override === "locked" ||
		override === "cancellable" ||
		override === "variable"
	) {
		return override;
	}
	if (tx.isInstallment === 1) return "locked";
	if (tx.isSubscription === 1) return "cancellable";
	if (fixedCategories.has(parseCategory(tx.categoryId).parent)) return "locked";
	return "variable";
};

// ── Filtering ──────────────────────────────────────────────────────────────

/**
 * Apply UI filters to a list of transactions.
 *
 * Returns a new array; does not mutate the input.
 */
export const filterTransactions = (
	transactions: ReadonlyArray<TxView>,
	filters: TxFilters,
	overlayMap: Map<string, ReviewOverlay>,
	accountMap: Map<string, AccountInfo>,
	fixedCategories: ReadonlySet<string> = EMPTY_FIXED,
): TxView[] => {
	const cat = filters.categoryFilter?.trim().toLowerCase() ?? null;
	const text = filters.textFilter?.trim().toLowerCase() ?? null;

	return transactions.filter((tx) => {
		if (filters.installmentsOnly && !tx.isInstallment) return false;
		if (filters.subscriptionsOnly && !tx.isSubscription) return false;
		if (
			filters.tierFilter &&
			commitmentTier(tx, fixedCategories, overlayMap) !== filters.tierFilter
		)
			return false;
		if (filters.unreviewedOnly && tx.reviewed) return false;
		if (
			filters.uncategorizedOnly &&
			(effectiveCategory(tx, overlayMap) ?? "") !== ""
		)
			return false;
		if (filters.accountFilter && tx.accountId !== filters.accountFilter)
			return false;
		if (filters.ownerFilter) {
			if ((accountMap.get(tx.accountId)?.owner ?? "") !== filters.ownerFilter)
				return false;
		}
		if (cat) {
			if (
				!(effectiveCategory(tx, overlayMap) ?? "").toLowerCase().includes(cat)
			)
				return false;
		}
		if (text) {
			const haystack = [
				tx.description,
				tx.merchantName,
				tx.rawDescription,
				effectiveCategory(tx, overlayMap),
			]
				.filter(Boolean)
				.join(" ")
				.toLowerCase();
			if (!haystack.includes(text)) return false;
		}
		return true;
	});
};

/**
 * True when any filter is active (not at its default/neutral value).
 */
export const hasActiveFilters = (filters: TxFilters): boolean =>
	filters.installmentsOnly ||
	filters.subscriptionsOnly ||
	filters.unreviewedOnly ||
	!!filters.uncategorizedOnly ||
	!!filters.tierFilter ||
	!!filters.accountFilter ||
	!!filters.ownerFilter ||
	!!filters.categoryFilter ||
	!!filters.textFilter;

// ── Flat grouping (by category id) ─────────────────────────────────────────

export interface FlatGroups {
	income: TxView[];
	/** Expense categories sorted by absolute total descending. */
	expEntries: Array<[string, TxView[]]>;
}

/**
 * Group transactions into income (non-negative) and expense-by-category.
 *
 * Expense categories are sorted by absolute sum descending. Category resolution
 * uses overlayMap first, then the seed categoryId.
 */
export const groupByCategory = (
	transactions: ReadonlyArray<TxView>,
	overlayMap: Map<string, ReviewOverlay>,
): FlatGroups => {
	const income: TxView[] = [];
	const expMap = new Map<string, TxView[]>();

	for (const tx of transactions) {
		if (!isNegative(tx.amount)) {
			income.push(tx);
		} else {
			const cat = effectiveCategory(tx, overlayMap) ?? "—";
			const list = expMap.get(cat) ?? [];
			list.push(tx);
			expMap.set(cat, list);
		}
	}

	const expEntries = Array.from(expMap.entries()).sort((a, b) => {
		const sumA = Math.abs(sumAmounts(a[1].map((t) => t.amount)));
		const sumB = Math.abs(sumAmounts(b[1].map((t) => t.amount)));
		return sumB - sumA;
	});

	return { income, expEntries };
};

// ── Hierarchical grouping (parent → subcategory) ───────────────────────────

export interface SubGroup {
	total: number; // already-parsed number (sumAmounts result)
	txs: TxView[];
}

export interface ParentGroup {
	total: number;
	subs: Map<string, SubGroup>;
}

export interface HierarchicalGroups {
	income: TxView[];
	expenses: Map<string, ParentGroup>;
}

// ── Array-based view types (for iteration in React components) ─────────────

/** A subcategory as a flat object for easy iteration in views. */
export interface HierarchicalSubGroup {
	sub: string; // subcategory key ("—" for flat parents)
	total: number;
	count: number;
	txs: TxView[];
}

/** A parent category as a flat object for easy iteration in views. */
export interface HierarchicalParentGroup {
	parent: string;
	total: number;
	count: number;
	subs: HierarchicalSubGroup[];
	/** True when this parent has subcategories beyond the "—" sentinel. */
	hasSubs: boolean;
}

/** Array-based hierarchical groups (convenience for React rendering). */
export interface HierarchicalGroupsArray {
	income: TxView[];
	expenses: HierarchicalParentGroup[];
}

/** Convert Map-based HierarchicalGroups to an array-based shape for views. */
export const toHierarchicalArray = (
	groups: HierarchicalGroups,
): HierarchicalGroupsArray => {
	const expenses: HierarchicalParentGroup[] = [];
	for (const [parent, group] of groups.expenses) {
		const subs: HierarchicalSubGroup[] = [];
		let count = 0;
		for (const [subKey, sub] of group.subs) {
			subs.push({
				sub: subKey,
				total: sub.total,
				count: sub.txs.length,
				txs: sub.txs,
			});
			count += sub.txs.length;
		}
		const hasSubs =
			subs.length > 1 || (subs.length === 1 && subs[0]!.sub !== "—");
		expenses.push({ parent, total: group.total, count, subs, hasSubs });
	}
	return { income: groups.income, expenses };
};

/**
 * Group expenses hierarchically: parent category → subcategory.
 *
 * - "alimentacao:mercado" and "alimentacao:restaurante" roll up under
 *   "alimentacao".
 * - Flat categories (no colon) become a parent with a single null-key sub.
 * - Uncategorized (null / "—") is treated as parent "—".
 * - Parent categories are sorted by absolute total descending.
 * - Subcategories within each parent are sorted by absolute total descending.
 */
export const groupHierarchical = (
	transactions: ReadonlyArray<TxView>,
	overlayMap: Map<string, ReviewOverlay>,
): HierarchicalGroups => {
	const income: TxView[] = [];
	const parentMap = new Map<string, Map<string, TxView[]>>();

	for (const tx of transactions) {
		if (!isNegative(tx.amount)) {
			income.push(tx);
			continue;
		}
		const cat = effectiveCategory(tx, overlayMap) ?? null;
		const { parent, sub } = parseCategory(cat);
		const subKey = sub ?? "—"; // sentinel for "no subcategory"

		let subs = parentMap.get(parent);
		if (!subs) {
			subs = new Map();
			parentMap.set(parent, subs);
		}
		const list = subs.get(subKey) ?? [];
		list.push(tx);
		subs.set(subKey, list);
	}

	// Build the result with computed totals, sorted.
	const expenses = new Map<string, ParentGroup>();
	const parentEntries = Array.from(parentMap.entries()).map(
		([parent, subs]) => {
			const subGroups = new Map<string, SubGroup>();
			let parentTotal = 0;

			// Sort subcategories by absolute total desc.
			const subEntries = Array.from(subs.entries())
				.map(([subKey, txs]) => {
					const total = Math.abs(sumAmounts(txs.map((t) => t.amount)));
					return { subKey, total, txs };
				})
				.sort((a, b) => b.total - a.total);

			for (const { subKey, total, txs } of subEntries) {
				subGroups.set(subKey, { total, txs });
				parentTotal += total;
			}

			return [parent, { total: parentTotal, subs: subGroups }] as const;
		},
	);

	// Sort parents by total desc.
	parentEntries.sort((a, b) => b[1].total - a[1].total);

	for (const [parent, group] of parentEntries) {
		expenses.set(parent, group);
	}

	return { income, expenses };
};

// ── Month helpers ──────────────────────────────────────────────────────────

/** Filter transactions to a single month ("YYYY-MM"). */
export const transactionsForMonth = (
	transactions: ReadonlyArray<TxView>,
	month: string,
): TxView[] => transactions.filter((t) => t.month === month);

// ── Sum helpers ────────────────────────────────────────────────────────────

export interface MonthSums {
	entradas: number;
	saidas: number;
}

/** Compute total entradas (income) and saidas (expenses) for a set of transactions. */
export const computeMonthSums = (
	transactions: ReadonlyArray<TxView>,
): MonthSums => {
	const out = transactions
		.filter((t) => isNegative(t.amount))
		.map((t) => t.amount);
	const inc = transactions
		.filter((t) => !isNegative(t.amount))
		.map((t) => t.amount);
	return { saidas: Math.abs(sumAmounts(out)), entradas: sumAmounts(inc) };
};

// ── Planilha (flat sheet) sorting ───────────────────────────────────────────

export type SheetSortKey =
	| "date"
	| "description"
	| "account"
	| "category"
	| "amount"
	| "origin"
	| "flow";

export interface SheetSort {
	key: SheetSortKey;
	/** 1 = ascending, -1 = descending. */
	dir: 1 | -1;
}

/** Visible label of a transaction (human description wins over raw). */
export const sheetLabel = (tx: TxView): string =>
	tx.description || tx.merchantName || tx.rawDescription;

/**
 * Sort a transaction list for the sheet view. Stable for equal keys (falls
 * back to postedAt desc, then id, so re-renders never shuffle rows).
 */
export const sortForSheet = (
	transactions: ReadonlyArray<TxView>,
	sort: SheetSort,
	overlayMap: Map<string, ReviewOverlay>,
	accountMap: Map<string, AccountInfo>,
): TxView[] => {
	const keyOf = (tx: TxView): string | number => {
		switch (sort.key) {
			case "date":
				return tx.postedAt;
			case "description":
				return sheetLabel(tx).toLowerCase();
			case "account":
				return (accountMap.get(tx.accountId)?.label ?? tx.accountId).toLowerCase();
			case "category":
				return (effectiveCategory(tx, overlayMap) ?? "").toLowerCase();
			case "amount":
				return toCents(tx.amount);
			case "origin":
				// Every row here is a real transaction — origin can't discriminate.
				return 0;
			case "flow":
				return isNegative(tx.amount) ? 1 : 0;
		}
	};
	return [...transactions].sort((a, b) => {
		const ka = keyOf(a);
		const kb = keyOf(b);
		const cmp = ka < kb ? -1 : ka > kb ? 1 : 0;
		if (cmp !== 0) return cmp * sort.dir;
		const dateCmp = a.postedAt < b.postedAt ? 1 : a.postedAt > b.postedAt ? -1 : 0;
		return dateCmp !== 0 ? dateCmp : a.id < b.id ? -1 : 1;
	});
};

// ── Plano de guerra (budget envelopes vs realized + simulation) ─────────────

/** The forecast fields the war plan needs (subset of the view shape). */
export interface PlanForecast {
	amount: string;
	categoryId: string | null;
	kind: string;
	status: string;
	month: string | null;
}

/** One slider row: a subcategory ("—" when the parent is flat). */
export interface WarPlanSubRow {
	sub: string;
	/** Full category id: the parent alone for "—", else "parent:sub". */
	categoryId: string;
	/** Realized expense magnitude in the selected month. */
	realizado: number;
	/** Average realized magnitude over the 3 calendar months before `month`. */
	media3m: number;
	/**
	 * Where the goal slider opens: the 3-month average, floored at what is
	 * already spent (money out the door can't be budgeted away). Envelope-only
	 * parents (no transactions at all) open at the envelope itself.
	 */
	goalBase: number;
}

export interface WarPlanRow {
	parent: string;
	/** Realized expense magnitude in the selected month. */
	realizado: number;
	/** Monthly budget envelope for the parent (null when none is defined). */
	orcamento: number | null;
	/** Average realized magnitude over the 3 calendar months before `month`. */
	media3m: number;
	/**
	 * What the month is on track to cost: `max(realizado, orçamento)` for an
	 * open month (mirrors the chart's envelope model), plain `realizado` once
	 * the month is closed.
	 */
	projecao: number;
	/** Subcategory slider rows, heaviest (realized or average) first. */
	subs: WarPlanSubRow[];
	/**
	 * This month's locked-tier spend in this parent (rent, installments, fixed
	 * bills). Excluded from `realizado`/`subs`/`projecao` — planning only
	 * simulates what can be cut — but surfaced as a "🔒 fixo" note so the
	 * committed amount is acknowledged (ADR-0030).
	 */
	lockedRealizado: number;
}

export interface WarPlan {
	rows: WarPlanRow[];
	/** Installment-kind forecast magnitude committed in the month (no category). */
	parcelasComprometidas: number;
	totalRealizado: number;
	totalOrcamento: number;
	totalProjecao: number;
}

/** Both status vocabularies coexist in the runtime (pt/en) — match either. */
export const ACTIVE_FORECAST_STATUSES: ReadonlySet<string> = new Set([
	"ativo",
	"active",
]);

/**
 * A budget envelope is an ACTIVE parent-level EXPENSE forecast. Card bills
 * (no category), sub-level planned items and installment forecasts are not
 * budgets. Single predicate shared by the war plan (reading envelopes) and
 * the goal persistence (updating them) so the two can never disagree.
 */
export const isBudgetEnvelope = (f: {
	amount: string;
	categoryId: string | null;
	kind: string;
	status: string;
}): boolean =>
	ACTIVE_FORECAST_STATUSES.has(f.status) &&
	isNegative(f.amount) &&
	f.kind !== "installment" &&
	f.categoryId != null &&
	f.categoryId !== "" &&
	!f.categoryId.includes(":");

/** Previous `n` calendar months of a "YYYY-MM" key, most recent first. */
export const previousMonths = (month: string, n: number): string[] => {
	const [y, m] = month.split("-").map(Number);
	if (!y || !m) return [];
	const out: string[] = [];
	let year = y;
	let mon = m;
	for (let i = 0; i < n; i++) {
		mon -= 1;
		if (mon === 0) {
			mon = 12;
			year -= 1;
		}
		out.push(`${year}-${String(mon).padStart(2, "0")}`);
	}
	return out;
};

/**
 * Build the war-plan table for a month: per parent category, the realized
 * spend, the budget envelope (an active parent-level expense forecast in that
 * month — how finance envelopes are modelled), the 3-month average and the
 * projection. Card-bill forecasts (no category) and installment forecasts are
 * excluded from the per-category rows; installments surface as a single
 * committed magnitude so the table never double-counts a real expense.
 */
/** Per-sub spend magnitudes accumulated while bucketing transactions. */
interface SubSpend {
	realizado: number;
	historico: number;
	/** Of `realizado + historico`, how much is locked-tier (ADR-0030). */
	locked: number;
}

/**
 * Slider rows for one parent — only the *simulatable* (non-locked) subs;
 * envelope-only parents get a pseudo-sub. Majority-locked subs (rent,
 * installments, fixed bills) are dropped from the list and their realized
 * spend is summed into `lockedRealizado` for the "🔒 fixo" note (ADR-0030).
 */
const buildSubRows = (
	parent: string,
	subCells: ReadonlyMap<string, SubSpend> | undefined,
	orcamento: number | null,
	allowEnvelopeSub: boolean,
): { subs: WarPlanSubRow[]; lockedRealizado: number } => {
	let lockedRealizado = 0;
	const subs: WarPlanSubRow[] = [];
	for (const [subKey, cell] of subCells?.entries() ?? []) {
		const total = cell.realizado + cell.historico;
		const isLocked = total > 0 && cell.locked >= total / 2;
		if (isLocked) {
			lockedRealizado += cell.realizado;
			continue;
		}
		subs.push({
			sub: subKey,
			categoryId: subKey === "—" ? parent : `${parent}:${subKey}`,
			realizado: cell.realizado,
			media3m: cell.historico / 3,
			goalBase: Math.max(cell.historico / 3, cell.realizado),
		});
	}
	subs.sort(
		(a, b) =>
			Math.max(b.realizado, b.media3m) - Math.max(a.realizado, a.media3m),
	);
	if (subs.length === 0 && orcamento != null && allowEnvelopeSub) {
		// Envelope-only parent: a pseudo-sub so the envelope itself is sliddable.
		// Suppressed for fixed (locked) categories — their envelope is committed.
		subs.push({
			sub: "—",
			categoryId: parent,
			realizado: 0,
			media3m: 0,
			goalBase: orcamento,
		});
	}
	return { subs, lockedRealizado };
};

/**
 * Bucket expense magnitudes by parent → subKey for the war plan: the selected
 * month's spend plus the 3 previous months' (for the average). History-only
 * parents stay in — no spend this month, but they still deserve a slider.
 */
const accumulateSpend = (
	transactions: ReadonlyArray<TxView>,
	month: string,
	overlayMap: Map<string, ReviewOverlay>,
	fixedCategories: ReadonlySet<string>,
): Map<string, Map<string, SubSpend>> => {
	const spendBy = new Map<string, Map<string, SubSpend>>();
	const history = new Set(previousMonths(month, 3));
	for (const tx of transactions) {
		if (!isNegative(tx.amount)) continue;
		const inMonth = tx.month === month;
		if (!inMonth && !history.has(tx.month)) continue;
		const { parent, sub } = parseCategory(effectiveCategory(tx, overlayMap));
		const subKey = sub ?? "—";
		const mag = Math.abs(toCents(tx.amount)) / 100;
		// Locked spend (installments, fixed bills, a manual lock) is committed —
		// it stays visible but is marked non-simulatable, not cut (ADR-0030).
		const isLocked =
			commitmentTier(tx, fixedCategories, overlayMap) === "locked";
		let subs = spendBy.get(parent);
		if (!subs) {
			subs = new Map();
			spendBy.set(parent, subs);
		}
		const cell = subs.get(subKey) ?? { realizado: 0, historico: 0, locked: 0 };
		if (inMonth) cell.realizado += mag;
		else cell.historico += mag;
		if (isLocked) cell.locked += mag;
		subs.set(subKey, cell);
	}
	return spendBy;
};

/** Sum a month's active envelopes per parent + its committed installments. */
const accumulateEnvelopes = (
	monthForecasts: ReadonlyArray<PlanForecast>,
	month: string,
): { orcamentoBy: Map<string, number>; parcelasComprometidas: number } => {
	const orcamentoBy = new Map<string, number>();
	let parcelasComprometidas = 0;
	for (const f of monthForecasts) {
		if (f.month !== month) continue;
		if (!ACTIVE_FORECAST_STATUSES.has(f.status)) continue;
		if (!isNegative(f.amount)) continue;
		const mag = Math.abs(toCents(f.amount)) / 100;
		if (f.kind === "installment") {
			parcelasComprometidas += mag;
		} else if (isBudgetEnvelope(f)) {
			const cat = f.categoryId as string;
			orcamentoBy.set(cat, (orcamentoBy.get(cat) ?? 0) + mag);
		}
	}
	return { orcamentoBy, parcelasComprometidas };
};

export const buildWarPlan = (
	transactions: ReadonlyArray<TxView>,
	month: string,
	monthForecasts: ReadonlyArray<PlanForecast>,
	overlayMap: Map<string, ReviewOverlay>,
	mode: "open" | "past" = "open",
): WarPlan => {
	const fixedCategories = fixedCategoriesFromForecasts(monthForecasts);
	const spendBy = accumulateSpend(
		transactions,
		month,
		overlayMap,
		fixedCategories,
	);
	const { orcamentoBy, parcelasComprometidas } = accumulateEnvelopes(
		monthForecasts,
		month,
	);
	const parents = new Set<string>([...spendBy.keys(), ...orcamentoBy.keys()]);
	const rows: WarPlanRow[] = [];
	for (const parent of parents) {
		const orcamento = orcamentoBy.get(parent) ?? null;
		const { subs, lockedRealizado } = buildSubRows(
			parent,
			spendBy.get(parent),
			orcamento,
			!fixedCategories.has(parent),
		);
		// A parent with nothing simulatable (all spend locked, no envelope) drops
		// out of the war plan entirely — only its "🔒 fixo" weight lives on in the
		// annual chart, not here.
		if (subs.length === 0) continue;
		const realizado = subs.reduce((s, x) => s + x.realizado, 0);
		const media3m = subs.reduce((s, x) => s + x.media3m, 0);
		const projecao =
			mode === "past" ? realizado : Math.max(realizado, orcamento ?? 0);
		rows.push({
			parent,
			realizado,
			orcamento,
			media3m,
			projecao,
			subs,
			lockedRealizado,
		});
	}
	rows.sort(
		(a, b) =>
			b.projecao - a.projecao ||
			b.media3m - a.media3m ||
			a.parent.localeCompare(b.parent),
	);

	return {
		rows,
		parcelasComprometidas,
		totalRealizado: rows.reduce((s, r) => s + r.realizado, 0),
		totalOrcamento: rows.reduce((s, r) => s + (r.orcamento ?? 0), 0),
		totalProjecao: rows.reduce((s, r) => s + r.projecao, 0),
	};
};

export interface WarPlanSimulation {
	/** Projected month total with the simulated budgets applied. */
	projecaoSimulada: number;
	/** How much the month improves vs. the baseline projection. */
	economiaMes: number;
}

/**
 * Apply simulated budget targets (parent → new monthly budget) to a war plan.
 * Realized spend is a floor — money already out the door can't be simulated
 * away — so each row contributes `max(realizado, target ?? baseline)`.
 */
export const simulateWarPlan = (
	plan: WarPlan,
	targets: ReadonlyMap<string, number>,
): WarPlanSimulation => {
	let sim = 0;
	for (const row of plan.rows) {
		const target = targets.get(row.parent);
		sim +=
			target == null
				? row.projecao
				: Math.max(row.realizado, Math.max(0, target));
	}
	return {
		projecaoSimulada: sim,
		economiaMes: plan.totalProjecao - sim,
	};
};

export interface WarPlanGoalSimulation {
	/** Projected month total with the slider goals applied. */
	projecaoSimulada: number;
	/** Baseline projection minus simulated (negative = goals above projection). */
	economiaMes: number;
	/**
	 * Per-parent envelope goal (sum of its sub sliders) for parents with at
	 * least one explicit goal — exactly what gets persisted on confirm.
	 */
	goalByParent: Map<string, number>;
	/** Per-parent simulated month total (baseline projecao when untouched). */
	simulatedByParent: Map<string, number>;
}

const round2 = (v: number): number => Math.round(v * 100) / 100;

/**
 * Apply per-subcategory goal sliders to a war plan. `goals` is keyed by the
 * sub's `categoryId`. A parent with at least one explicit goal switches to
 * goal mode: its month projection becomes the sum over its subs of
 * `max(realizado, goal ?? goalBase)` — realized spend is a floor, and
 * untouched sliders sit at their opening value so the math always matches
 * what the sliders display. Untouched parents keep the baseline projection.
 */
export const simulateWarPlanGoals = (
	plan: WarPlan,
	goals: ReadonlyMap<string, number>,
): WarPlanGoalSimulation => {
	let sim = 0;
	const goalByParent = new Map<string, number>();
	const simulatedByParent = new Map<string, number>();
	for (const row of plan.rows) {
		const touched = row.subs.some((s) => goals.has(s.categoryId));
		if (!touched) {
			sim += row.projecao;
			simulatedByParent.set(row.parent, row.projecao);
			continue;
		}
		let simulated = 0;
		let goalTotal = 0;
		for (const s of row.subs) {
			const goal = Math.max(0, goals.get(s.categoryId) ?? s.goalBase);
			goalTotal += goal;
			simulated += Math.max(s.realizado, goal);
		}
		goalByParent.set(row.parent, round2(goalTotal));
		simulatedByParent.set(row.parent, simulated);
		sim += simulated;
	}
	return {
		projecaoSimulada: sim,
		economiaMes: plan.totalProjecao - sim,
		goalByParent,
		simulatedByParent,
	};
};

// ── Envelope persistence (goal → monthly budget forecasts) ──────────────────

/** The forecast fields needed to find the envelopes a goal must update. */
export interface EnvelopeForecastRef {
	forecastId: string;
	amount: string;
	categoryId: string | null;
	kind: string;
	status: string;
	month: string | null;
}

/** One bridge write: update (forecastId set) or create (null) an envelope. */
export interface EnvelopeWrite {
	forecastId: string | null;
	month: string;
	categoryId: string;
	/** Negative decimal string — envelopes are expenses. */
	amount: string;
	/** Empty on update (the bridge keeps the existing description). */
	description: string;
	dueDate: string;
}

/** Last calendar day of a "YYYY-MM" month, as "YYYY-MM-DD". */
export const lastDayOfMonth = (month: string): string => {
	const [y, m] = month.split("-").map(Number);
	const day = y && m ? new Date(y, m, 0).getDate() : 1;
	return `${month}-${String(day).padStart(2, "0")}`;
};

/**
 * Turn confirmed parent goals into bridge writes, one per (parent, month).
 * When a month already has envelope forecasts for the parent, the first one
 * is re-amounted so the month's envelope TOTAL equals the goal (siblings are
 * left alone, never flipped positive); otherwise a new envelope forecast is
 * created. Due dates sit on the last day of the month so the envelope keeps
 * counting as "remaining" in the cash chart for the whole month.
 */
export const buildEnvelopeWrites = (
	goalByParent: ReadonlyMap<string, number>,
	months: ReadonlyArray<string>,
	existing: ReadonlyArray<EnvelopeForecastRef>,
): EnvelopeWrite[] => {
	const parents = Array.from(goalByParent.keys())
		.filter((p) => p !== "—") // uncategorized spend can't carry a budget
		.sort((a, b) => a.localeCompare(b));
	return parents.flatMap((parent) =>
		months.map((month) =>
			envelopeWriteFor(parent, goalByParent.get(parent) ?? 0, month, existing),
		),
	);
};

/** The single (parent, month) write: update the first envelope or create one. */
const envelopeWriteFor = (
	parent: string,
	goal: number,
	month: string,
	existing: ReadonlyArray<EnvelopeForecastRef>,
): EnvelopeWrite => {
	const envelopes = existing.filter(
		(f) => f.month === month && f.categoryId === parent && isBudgetEnvelope(f),
	);
	const dueDate = lastDayOfMonth(month);
	if (envelopes.length === 0) {
		return {
			forecastId: null,
			month,
			categoryId: parent,
			amount: (-goal).toFixed(2),
			description: `meta ${parent}`,
			dueDate,
		};
	}
	const siblings = envelopes
		.slice(1)
		.reduce((s, f) => s + Math.abs(toCents(f.amount)) / 100, 0);
	return {
		forecastId: envelopes[0].forecastId,
		month,
		categoryId: parent,
		amount: (-Math.max(0, round2(goal - siblings))).toFixed(2),
		description: "",
		dueDate,
	};
};

// ── Per-month category distribution (chart "Despesas" modes) ────────────────

export interface CategoryMonthSeries {
	/** Parent categories ranked by total spend desc; tail collapsed to "outros". */
	categories: string[];
	/** month ("YYYY-MM") → parent category → positive expense magnitude. */
	byMonth: Map<string, Map<string, number>>;
}

const OUTROS = "outros";

/**
 * Break monthly expenses down by PARENT category for the stacked/line
 * "Despesas" chart modes. Keeps the `topN` biggest parents; everything else
 * rolls into an "outros" bucket so the legend stays readable. Category
 * resolution is overlay-first (matches the transaction list).
 */
export const expensesByMonthCategory = (
	transactions: ReadonlyArray<TxView>,
	overlayMap: Map<string, ReviewOverlay>,
	topN = 6,
): CategoryMonthSeries => {
	const totals = new Map<string, number>();
	const raw = new Map<string, Map<string, number>>(); // month -> parent -> mag

	for (const tx of transactions) {
		if (!isNegative(tx.amount)) continue;
		const parent = parseCategory(effectiveCategory(tx, overlayMap)).parent;
		const mag = Math.abs(toCents(tx.amount)) / 100;
		totals.set(parent, (totals.get(parent) ?? 0) + mag);
		let m = raw.get(tx.month);
		if (!m) {
			m = new Map();
			raw.set(tx.month, m);
		}
		m.set(parent, (m.get(parent) ?? 0) + mag);
	}

	const ranked = Array.from(totals.entries())
		.sort((a, b) => b[1] - a[1])
		.map(([cat]) => cat);
	const top = new Set(ranked.slice(0, topN));
	const hasOutros = ranked.length > topN;
	const categories = ranked.slice(0, topN);
	if (hasOutros) categories.push(OUTROS);

	const byMonth = new Map<string, Map<string, number>>();
	for (const [month, cats] of raw) {
		const collapsed = new Map<string, number>();
		for (const [cat, mag] of cats) {
			const key = top.has(cat) ? cat : OUTROS;
			collapsed.set(key, (collapsed.get(key) ?? 0) + mag);
		}
		byMonth.set(month, collapsed);
	}

	return { categories, byMonth };
};

// ── Unified sheet (transactions + forecasts + scenario rows) ────────────────

/**
 * Where a sheet row comes from (ADR-0037 unified sheet). Drives the origin
 * icon, the origin filter chips and the origin sort order.
 */
export type SheetOrigin =
	| "real"
	| "installment"
	| "recurring"
	| "fixed"
	| "manual"
	| "scenario";

/** Origin glyphs for the sheet's first column. */
export const SHEET_ORIGIN_ICONS: Record<SheetOrigin, string> = {
	real: "✓",
	installment: "≡",
	recurring: "↻",
	fixed: "⌂",
	manual: "✎",
	scenario: "🧪",
};

/** Classify a forecast row on the sheet-origin axis. */
export const forecastSheetOrigin = (forecast: {
	kind: string;
	templateId: string | null;
}): SheetOrigin => {
	if (forecast.kind === "installment") return "installment";
	if (forecast.kind === "fixed") return "fixed";
	if (forecast.kind === "manual" && !forecast.templateId) return "manual";
	return "recurring";
};

/** Whole calendar months from `from` to `to` (both "YYYY-MM"; can be negative). */
export const monthDiff = (from: string, to: string): number => {
	const [fy, fm] = from.split("-").map(Number);
	const [ty, tm] = to.split("-").map(Number);
	if (!fy || !fm || !ty || !tm) return Number.NaN;
	return (ty - fy) * 12 + (tm - fm);
};

/** The forecast fields the unified sheet needs. */
export interface SheetForecastLike {
	forecastId: string;
	dueDate: string | null;
	description: string;
	amount: string;
	categoryId: string | null;
	accountId: string | null;
	status: string;
	kind: string;
	templateId: string | null;
}

/** A scenario delta as the sheet consumes it (subset of the LiveStore row). */
export interface ScenarioChangeLike {
	changeId: string;
	kind: string;
	targetForecastId: string | null;
	targetTemplateId: string | null;
	month: string | null;
	effectiveFrom: string | null;
	amount: string | null;
	monthsCount: number | null;
	description: string | null;
	categoryId: string | null;
	accountId: string | null;
}

/** One planned (non-realized) row of the unified sheet. */
export interface PlannedSheetRow {
	/** forecastId for baseline rows, changeId for scenario-added rows. */
	id: string;
	forecastId: string | null;
	templateId: string | null;
	origin: SheetOrigin;
	description: string;
	/** Effective amount after scenario adjustments (decimal string). */
	amount: string;
	/** The pre-adjustment amount when an `adjust_amount` delta applied. */
	originalAmount: string | null;
	/** True when a `skip_forecast` / `end_template` delta removed this row. */
	skipped: boolean;
	dueDate: string; // YYYY-MM-DD
	categoryId: string | null;
	accountId: string | null;
	/** "n/N" for hypothetical installments. */
	installmentLabel: string | null;
	/** The scenario change that created this row (scenario-added rows only). */
	changeId: string | null;
	/** The adjust_amount change applied to this row, if any. */
	adjustChangeId: string | null;
	/** The skip/end change that skipped this row, if any. */
	skipChangeId: string | null;
}

const plannedRowFromForecast = (
	forecast: SheetForecastLike,
	month: string,
): PlannedSheetRow => ({
	id: forecast.forecastId,
	forecastId: forecast.forecastId,
	templateId: forecast.templateId,
	origin: forecastSheetOrigin(forecast),
	description: forecast.description,
	amount: forecast.amount,
	originalAmount: null,
	skipped: false,
	dueDate: forecast.dueDate ?? `${month}-01`,
	categoryId: forecast.categoryId,
	accountId: forecast.accountId,
	installmentLabel: null,
	changeId: null,
	adjustChangeId: null,
	skipChangeId: null,
});

const changeAppliesToRow = (
	change: ScenarioChangeLike,
	row: PlannedSheetRow,
	month: string,
): boolean => {
	if (change.kind === "adjust_amount" || change.kind === "skip_forecast") {
		return change.targetForecastId === row.forecastId;
	}
	if (change.kind === "end_template") {
		return (
			row.templateId != null &&
			change.targetTemplateId === row.templateId &&
			change.effectiveFrom != null &&
			change.effectiveFrom <= month
		);
	}
	return false;
};

const applyChangesToRow = (
	row: PlannedSheetRow,
	changes: ReadonlyArray<ScenarioChangeLike>,
	month: string,
): PlannedSheetRow => {
	let out = row;
	for (const change of changes) {
		if (!changeAppliesToRow(change, out, month)) continue;
		if (change.kind === "adjust_amount" && change.amount != null) {
			out = {
				...out,
				amount: change.amount,
				originalAmount: out.originalAmount ?? row.amount,
				adjustChangeId: change.changeId,
			};
		} else {
			out = {
				...out,
				skipped: true,
				skipChangeId: out.skipChangeId ?? change.changeId,
			};
		}
	}
	return out;
};

const scenarioAddedRow = (
	change: ScenarioChangeLike,
	month: string,
	installmentLabel: string | null,
): PlannedSheetRow => ({
	id: change.changeId,
	forecastId: null,
	templateId: null,
	origin: "scenario",
	description: change.description ?? "",
	amount: change.amount ?? "0",
	originalAmount: null,
	skipped: false,
	dueDate: `${month}-01`,
	categoryId: change.categoryId,
	accountId: change.accountId,
	installmentLabel,
	changeId: change.changeId,
	adjustChangeId: null,
	skipChangeId: null,
});

const scenarioAddedRows = (
	changes: ReadonlyArray<ScenarioChangeLike>,
	month: string,
): PlannedSheetRow[] => {
	const rows: PlannedSheetRow[] = [];
	for (const change of changes) {
		if (change.kind === "add_one_shot" && change.month === month) {
			rows.push(scenarioAddedRow(change, month, null));
		} else if (
			change.kind === "hypothetical_installment" &&
			change.effectiveFrom != null
		) {
			const idx = monthDiff(change.effectiveFrom, month);
			const count = change.monthsCount ?? 0;
			if (Number.isFinite(idx) && idx >= 0 && idx < count) {
				rows.push(scenarioAddedRow(change, month, `${idx + 1}/${count}`));
			}
		}
	}
	return rows;
};

/**
 * Derive the planned rows of one sheet month with a scenario applied
 * (ADR-0037, client-side): active forecasts of the month get `adjust_amount`
 * substitutions (keeping `originalAmount` for the strikethrough),
 * `skip_forecast` / `end_template` marks (`skipped`), and the scenario's own
 * `add_one_shot` / `hypothetical_installment` rows are appended (installment
 * position "n/N" computed from `effectiveFrom`). With `changes` empty this is
 * simply the month's active forecasts — the baseline path.
 */
export const applyScenarioToMonthRows = (
	forecasts: ReadonlyArray<SheetForecastLike>,
	changes: ReadonlyArray<ScenarioChangeLike>,
	month: string,
): PlannedSheetRow[] => {
	const base = forecasts
		.filter(
			(f) =>
				ACTIVE_FORECAST_STATUSES.has(f.status) &&
				(f.dueDate ?? "").slice(0, 7) === month,
		)
		.map((f) => applyChangesToRow(plannedRowFromForecast(f, month), changes, month));
	return [...base, ...scenarioAddedRows(changes, month)];
};

// ── Unified sheet sorting + localStorage persistence ────────────────────────

/** The row fields the unified sort compares. */
export interface UnifiedRowKeys {
	id: string;
	date: string;
	description: string;
	account: string;
	category: string | null;
	amount: string;
	origin: string;
}

const ORIGIN_SORT_ORDER: Record<string, number> = {
	real: 0,
	installment: 1,
	recurring: 2,
	fixed: 3,
	manual: 4,
	scenario: 5,
};

const unifiedSortKey = (row: UnifiedRowKeys, key: SheetSortKey): string | number => {
	switch (key) {
		case "date":
			return row.date;
		case "description":
			return row.description.toLowerCase();
		case "account":
			return row.account.toLowerCase();
		case "category":
			return (row.category ?? "").toLowerCase();
		case "amount":
			return toCents(row.amount);
		case "origin":
			return ORIGIN_SORT_ORDER[row.origin] ?? 9;
		case "flow":
			// Ascending = income first (matches the green-on-top mental model).
			return isNegative(row.amount) ? 1 : 0;
	}
};

/**
 * Sort the unified sheet (real + planned + scenario rows together). Stable
 * for equal keys: falls back to date desc, then id, so re-renders never
 * shuffle rows.
 */
export const sortUnifiedRows = <T extends UnifiedRowKeys>(
	rows: ReadonlyArray<T>,
	sort: SheetSort,
): T[] =>
	[...rows].sort((a, b) => {
		const ka = unifiedSortKey(a, sort.key);
		const kb = unifiedSortKey(b, sort.key);
		const cmp = ka < kb ? -1 : ka > kb ? 1 : 0;
		if (cmp !== 0) return cmp * sort.dir;
		const dateCmp = a.date < b.date ? 1 : a.date > b.date ? -1 : 0;
		return dateCmp !== 0 ? dateCmp : a.id < b.id ? -1 : 1;
	});

/** localStorage keys for the sheet's session-agnostic view preferences. */
export const SHEET_SORT_STORAGE_KEY = "phai:sheetSort";
export const SHEET_FILTERS_STORAGE_KEY = "phai:sheetFilters";

const SHEET_SORT_KEYS: ReadonlySet<string> = new Set([
	"date",
	"description",
	"account",
	"category",
	"amount",
	"origin",
	"flow",
]);

/**
 * Read the persisted sheet sort ({col,dir} JSON in localStorage — NEVER the
 * LiveStore ui document, so a sort click can't trigger a store migration).
 * Unknown/corrupt payloads read as null (caller falls back to the default).
 */
export const readSheetSort = (
	storage: Pick<Storage, "getItem">,
): SheetSort | null => {
	try {
		const raw = storage.getItem(SHEET_SORT_STORAGE_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as { col?: unknown; dir?: unknown };
		if (
			typeof parsed.col === "string" &&
			SHEET_SORT_KEYS.has(parsed.col) &&
			(parsed.dir === 1 || parsed.dir === -1)
		) {
			return { key: parsed.col as SheetSortKey, dir: parsed.dir };
		}
		return null;
	} catch {
		return null;
	}
};

export const writeSheetSort = (
	storage: Pick<Storage, "setItem">,
	sort: SheetSort,
): void => {
	try {
		storage.setItem(
			SHEET_SORT_STORAGE_KEY,
			JSON.stringify({ col: sort.key, dir: sort.dir }),
		);
	} catch {
		/* private mode / quota — sort just won't persist */
	}
};

export type SheetOriginFilter = "all" | SheetOrigin;
export type SheetFlowFilter = "all" | "in" | "out";

/** The two sheet-only filters persisted in localStorage (not the ui doc). */
export interface SheetLocalFilters {
	origin: SheetOriginFilter;
	flow: SheetFlowFilter;
}

export const DEFAULT_SHEET_LOCAL_FILTERS: SheetLocalFilters = {
	origin: "all",
	flow: "all",
};

const SHEET_ORIGIN_FILTER_VALUES: ReadonlySet<string> = new Set([
	"all",
	"real",
	"installment",
	"recurring",
	"fixed",
	"manual",
	"scenario",
]);
const SHEET_FLOW_FILTER_VALUES: ReadonlySet<string> = new Set([
	"all",
	"in",
	"out",
]);

export const readSheetLocalFilters = (
	storage: Pick<Storage, "getItem">,
): SheetLocalFilters => {
	try {
		const raw = storage.getItem(SHEET_FILTERS_STORAGE_KEY);
		if (!raw) return DEFAULT_SHEET_LOCAL_FILTERS;
		const parsed = JSON.parse(raw) as { origin?: unknown; flow?: unknown };
		return {
			origin:
				typeof parsed.origin === "string" &&
				SHEET_ORIGIN_FILTER_VALUES.has(parsed.origin)
					? (parsed.origin as SheetOriginFilter)
					: "all",
			flow:
				typeof parsed.flow === "string" &&
				SHEET_FLOW_FILTER_VALUES.has(parsed.flow)
					? (parsed.flow as SheetFlowFilter)
					: "all",
		};
	} catch {
		return DEFAULT_SHEET_LOCAL_FILTERS;
	}
};

export const writeSheetLocalFilters = (
	storage: Pick<Storage, "setItem">,
	filters: SheetLocalFilters,
): void => {
	try {
		storage.setItem(SHEET_FILTERS_STORAGE_KEY, JSON.stringify(filters));
	} catch {
		/* private mode / quota — filters just won't persist */
	}
};

/** Apply the sheet-only origin/flow chips to one unified row. */
export const matchesSheetLocalFilters = (
	row: { amount: string; origin: string },
	filters: SheetLocalFilters,
): boolean => {
	if (filters.origin !== "all" && row.origin !== filters.origin) return false;
	if (filters.flow === "in" && isNegative(row.amount)) return false;
	if (filters.flow === "out" && !isNegative(row.amount)) return false;
	return true;
};

// ── Unified sheet write routing (baseline vs. active scenario) ──────────────

/** The row identity fields the write router needs. */
export interface SheetRowRef {
	origin: SheetOrigin;
	forecastId: string | null;
	templateId: string | null;
	changeId: string | null;
}

/** "só em {mês}" vs. "de {mês} em diante". */
export type SheetDeleteScope = "month" | "onward";

export type SheetDeleteAction =
	| { kind: "baselineDelete"; forecastId: string }
	| { kind: "baselineDiscard"; forecastId: string }
	| { kind: "baselineEndTemplate"; templateId: string; effectiveFrom: string }
	| { kind: "scenarioSkip"; forecastId: string }
	| { kind: "scenarioEndTemplate"; templateId: string; effectiveFrom: string }
	| { kind: "scenarioRemoveChange"; changeId: string }
	| { kind: "none" };

/**
 * Decide what deleting a planned sheet row means (design D). Baseline: manual
 * one-shots are deleted, template-materialized rows are discarded for the
 * month or the whole template is ended from the month on. Scenario active:
 * the same gestures become plan deltas (skip_forecast / end_template), and a
 * scenario-added row simply removes its own change.
 */
export const routeSheetDelete = (
	row: SheetRowRef,
	scope: SheetDeleteScope,
	month: string,
	activeScenarioId: string | null,
): SheetDeleteAction => {
	if (row.origin === "scenario") {
		return row.changeId
			? { kind: "scenarioRemoveChange", changeId: row.changeId }
			: { kind: "none" };
	}
	if (scope === "onward" && row.templateId) {
		return activeScenarioId
			? {
					kind: "scenarioEndTemplate",
					templateId: row.templateId,
					effectiveFrom: month,
				}
			: {
					kind: "baselineEndTemplate",
					templateId: row.templateId,
					effectiveFrom: month,
				};
	}
	if (!row.forecastId) return { kind: "none" };
	if (activeScenarioId) {
		return { kind: "scenarioSkip", forecastId: row.forecastId };
	}
	return row.origin === "manual"
		? { kind: "baselineDelete", forecastId: row.forecastId }
		: { kind: "baselineDiscard", forecastId: row.forecastId };
};

export type SheetAmountAction =
	| { kind: "baselinePatch"; forecastId: string }
	| { kind: "scenarioAdjust"; forecastId: string }
	| { kind: "scenarioReplaceOneShot"; changeId: string }
	| { kind: "none" };

/**
 * Decide what an inline amount edit on a planned row means (design C).
 * Baseline: re-amount the forecast in place (envelope-upsert flow with
 * forecastId). Scenario: an adjust_amount delta; a scenario-added row
 * replaces its own change.
 */
export const routeSheetAmountEdit = (
	row: SheetRowRef,
	activeScenarioId: string | null,
): SheetAmountAction => {
	if (row.origin === "scenario") {
		return row.changeId
			? { kind: "scenarioReplaceOneShot", changeId: row.changeId }
			: { kind: "none" };
	}
	if (!row.forecastId) return { kind: "none" };
	return activeScenarioId
		? { kind: "scenarioAdjust", forecastId: row.forecastId }
		: { kind: "baselinePatch", forecastId: row.forecastId };
};

/** Decide what adding a sheet row means (design E). */
export const routeSheetAdd = (
	activeScenarioId: string | null,
): "forecastCreate" | "scenarioAddOneShot" =>
	activeScenarioId ? "scenarioAddOneShot" : "forecastCreate";

/** A subcategory slice of a parent's monthly spend, for the chart hover. */
export interface SubSlice {
	sub: string;
	mag: number;
}

/**
 * Per month → parent category → its subcategories (sorted by magnitude desc),
 * for the chart's per-segment hover ("hover a colour → that category + its top
 * subcategories"). The `outros` bucket of [`expensesByMonthCategory`] is not
 * modelled here — only real parents have subcategory detail.
 */
export const subExpensesByMonthCategory = (
	transactions: ReadonlyArray<TxView>,
	overlayMap: Map<string, ReviewOverlay>,
): Map<string, Map<string, SubSlice[]>> => {
	// month -> parent -> sub -> mag
	const raw = new Map<string, Map<string, Map<string, number>>>();
	for (const tx of transactions) {
		if (!isNegative(tx.amount)) continue;
		const { parent, sub } = parseCategory(effectiveCategory(tx, overlayMap));
		const subKey = sub ?? "(geral)";
		const mag = Math.abs(toCents(tx.amount)) / 100;
		let byParent = raw.get(tx.month);
		if (!byParent) {
			byParent = new Map();
			raw.set(tx.month, byParent);
		}
		let bySub = byParent.get(parent);
		if (!bySub) {
			bySub = new Map();
			byParent.set(parent, bySub);
		}
		bySub.set(subKey, (bySub.get(subKey) ?? 0) + mag);
	}

	const out = new Map<string, Map<string, SubSlice[]>>();
	for (const [month, byParent] of raw) {
		const parentMap = new Map<string, SubSlice[]>();
		for (const [parent, bySub] of byParent) {
			const slices = Array.from(bySub.entries())
				.map(([sub, mag]) => ({ sub, mag }))
				.sort((a, b) => b.mag - a.mag);
			parentMap.set(parent, slices);
		}
		out.set(month, parentMap);
	}
	return out;
};
