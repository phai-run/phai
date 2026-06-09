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
}

/** An optimistic overlay applied on top of a seed transaction. */
export interface ReviewOverlay {
	transactionId: string;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
}

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

// ── Account lookup ─────────────────────────────────────────────────────────

export interface AccountInfo {
	id: string;
	label: string;
	owner: string;
}

export const buildAccountMap = (
	accounts: ReadonlyArray<AccountInfo>,
): Map<string, AccountInfo> => new Map(accounts.map((a) => [a.id, a]));

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
): TxView[] => {
	const cat = filters.categoryFilter?.trim().toLowerCase() ?? null;
	const text = filters.textFilter?.trim().toLowerCase() ?? null;

	return transactions.filter((tx) => {
		if (filters.installmentsOnly && !tx.isInstallment) return false;
		if (filters.subscriptionsOnly && !tx.isSubscription) return false;
		if (filters.unreviewedOnly && tx.reviewed) return false;
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
	| "amount";

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
}

export interface WarPlan {
	rows: WarPlanRow[];
	/** Installment-kind forecast magnitude committed in the month (no category). */
	parcelasComprometidas: number;
	totalRealizado: number;
	totalOrcamento: number;
	totalProjecao: number;
}

const ACTIVE_FORECAST_STATUSES = new Set(["ativo", "active"]);

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
export const buildWarPlan = (
	transactions: ReadonlyArray<TxView>,
	month: string,
	monthForecasts: ReadonlyArray<PlanForecast>,
	overlayMap: Map<string, ReviewOverlay>,
	mode: "open" | "past" = "open",
): WarPlan => {
	const realizadoBy = new Map<string, number>();
	const historyBy = new Map<string, number>();
	const history = new Set(previousMonths(month, 3));

	for (const tx of transactions) {
		if (!isNegative(tx.amount)) continue;
		const inMonth = tx.month === month;
		const inHistory = history.has(tx.month);
		if (!inMonth && !inHistory) continue;
		const parent = parseCategory(effectiveCategory(tx, overlayMap)).parent;
		const mag = Math.abs(toCents(tx.amount)) / 100;
		if (inMonth) realizadoBy.set(parent, (realizadoBy.get(parent) ?? 0) + mag);
		else historyBy.set(parent, (historyBy.get(parent) ?? 0) + mag);
	}

	const orcamentoBy = new Map<string, number>();
	let parcelasComprometidas = 0;
	for (const f of monthForecasts) {
		if (f.month !== month) continue;
		if (!ACTIVE_FORECAST_STATUSES.has(f.status)) continue;
		const mag = Math.abs(toCents(f.amount)) / 100;
		if (!isNegative(f.amount)) continue;
		if (f.kind === "installment") {
			parcelasComprometidas += mag;
			continue;
		}
		const cat = f.categoryId ?? "";
		// Parent-level category = a budget envelope; anything else (card bills
		// with no category, sub-category planned items) is not a budget.
		if (cat !== "" && !cat.includes(":")) {
			orcamentoBy.set(cat, (orcamentoBy.get(cat) ?? 0) + mag);
		}
	}

	const parents = new Set<string>([
		...realizadoBy.keys(),
		...orcamentoBy.keys(),
	]);
	const rows: WarPlanRow[] = [];
	for (const parent of parents) {
		const realizado = realizadoBy.get(parent) ?? 0;
		const orcamento = orcamentoBy.get(parent) ?? null;
		const media3m = (historyBy.get(parent) ?? 0) / 3;
		const projecao =
			mode === "past" ? realizado : Math.max(realizado, orcamento ?? 0);
		rows.push({ parent, realizado, orcamento, media3m, projecao });
	}
	rows.sort((a, b) => b.projecao - a.projecao);

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
