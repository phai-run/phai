/**
 * Unit tests for lib/derivations.ts — pure transaction grouping, filtering,
 * category parsing, and summary functions.
 *
 * Every filter, group, and sum in the phai UI must derive from these functions
 * so that in-browser behaviour and test assertions stay in lockstep.
 */
import { describe, expect, it } from "vitest";
import {
	buildAccountMap,
	buildOverlayMap,
	computeMonthSums,
	effectiveCategory,
	filterTransactions,
	groupByCategory,
	groupHierarchical,
	hasActiveFilters,
	parseCategory,
	transactionsForMonth,
	type ReviewOverlay,
	type TxFilters,
	type TxView,
} from "../derivations";
import { sumAmounts } from "../format";
import { generateAccounts, generateTransactions } from "./fixtures";

// ── Fixture setup ──────────────────────────────────────────────────────────

const syntheticTxs = generateTransactions({ months: 12, txsPerMonth: 50 }); // 600 txs
const syntheticAccounts = generateAccounts();
const accountMap = buildAccountMap(syntheticAccounts);
const emptyOverlay = new Map<string, ReviewOverlay>();

// ── parseCategory ──────────────────────────────────────────────────────────

describe("parseCategory", () => {
	it('splits "alimentacao:mercado" into parent and sub', () => {
		const result = parseCategory("alimentacao:mercado");
		expect(result).toEqual({ parent: "alimentacao", sub: "mercado" });
	});

	it("treats flat category as parent with null sub", () => {
		const result = parseCategory("moradia");
		expect(result).toEqual({ parent: "moradia", sub: null });
	});

	it("handles null as uncategorized sentinel", () => {
		const result = parseCategory(null);
		expect(result).toEqual({ parent: "—", sub: null });
	});

	it("handles empty string as uncategorized", () => {
		const result = parseCategory("");
		expect(result).toEqual({ parent: "—", sub: null });
	});

	it("only splits at first colon (a:b:c → parent=a, sub=b:c)", () => {
		const result = parseCategory("a:b:c");
		expect(result).toEqual({ parent: "a", sub: "b:c" });
	});
});

// ── filterTransactions ─────────────────────────────────────────────────────

describe("filterTransactions", () => {
	const noFilters: TxFilters = {
		accountFilter: null,
		ownerFilter: null,
		categoryFilter: null,
		textFilter: null,
		installmentsOnly: false,
		subscriptionsOnly: false,
		unreviewedOnly: false,
	};

	it("returns all transactions when no filters active", () => {
		const result = filterTransactions(
			syntheticTxs,
			noFilters,
			emptyOverlay,
			accountMap,
		);
		expect(result).toHaveLength(syntheticTxs.length);
	});

	it("filters by installment flag", () => {
		const result = filterTransactions(
			syntheticTxs,
			{ ...noFilters, installmentsOnly: true },
			emptyOverlay,
			accountMap,
		);
		for (const tx of result) {
			expect(tx.isInstallment).toBe(1);
		}
		// Should be a subset
		expect(result.length).toBeLessThanOrEqual(syntheticTxs.length);
	});

	it("filters by subscription flag", () => {
		const result = filterTransactions(
			syntheticTxs,
			{ ...noFilters, subscriptionsOnly: true },
			emptyOverlay,
			accountMap,
		);
		for (const tx of result) {
			expect(tx.isSubscription).toBe(1);
		}
	});

	it("filters by account id", () => {
		const target = syntheticAccounts[0]!.id;
		const result = filterTransactions(
			syntheticTxs,
			{ ...noFilters, accountFilter: target },
			emptyOverlay,
			accountMap,
		);
		for (const tx of result) {
			expect(tx.accountId).toBe(target);
		}
	});

	it("text search finds transactions by description or merchant", () => {
		// Find a transaction with a known merchant
		const tx = syntheticTxs.find(
			(t) => t.merchantName && t.merchantName.length > 5,
		);
		if (tx?.merchantName) {
			const needle = tx.merchantName.slice(2, 6).toLowerCase();
			const result = filterTransactions(
				syntheticTxs,
				{ ...noFilters, textFilter: needle },
				emptyOverlay,
				accountMap,
			);
			// The transaction we found should be in results
			expect(result.some((r) => r.id === tx.id)).toBe(true);
		}
	});

	it("combined filters produce intersection", () => {
		const result = filterTransactions(
			syntheticTxs,
			{ ...noFilters, installmentsOnly: true, subscriptionsOnly: true },
			emptyOverlay,
			accountMap,
		);
		// No transaction can be both installment AND subscription in our fixtures
		// (the generator ensures exclusivity), so this should be empty
		expect(result.length).toBe(0);
	});

	it("filtered count never exceeds total", () => {
		const result = filterTransactions(
			syntheticTxs,
			{ ...noFilters, installmentsOnly: true },
			emptyOverlay,
			accountMap,
		);
		expect(result.length).toBeLessThanOrEqual(syntheticTxs.length);
	});
});

// ── hasActiveFilters ───────────────────────────────────────────────────────

describe("hasActiveFilters", () => {
	it("returns false when no filters are active", () => {
		expect(
			hasActiveFilters({
				accountFilter: null,
				ownerFilter: null,
				categoryFilter: null,
				textFilter: null,
				installmentsOnly: false,
				subscriptionsOnly: false,
				unreviewedOnly: false,
			}),
		).toBe(false);
	});

	it("returns true when any toggle is on", () => {
		expect(
			hasActiveFilters({
				accountFilter: null,
				ownerFilter: null,
				categoryFilter: null,
				textFilter: null,
				installmentsOnly: true,
				subscriptionsOnly: false,
				unreviewedOnly: false,
			}),
		).toBe(true);
	});

	it("returns true when text filter is set", () => {
		expect(
			hasActiveFilters({
				accountFilter: null,
				ownerFilter: null,
				categoryFilter: null,
				textFilter: "mercado",
				installmentsOnly: false,
				subscriptionsOnly: false,
				unreviewedOnly: false,
			}),
		).toBe(true);
	});
});

// ── groupByCategory ────────────────────────────────────────────────────────

describe("groupByCategory", () => {
	it("separates income from expenses", () => {
		const groups = groupByCategory(syntheticTxs, emptyOverlay);
		for (const tx of groups.income) {
			expect(Number(tx.amount)).toBeGreaterThanOrEqual(0);
		}
		for (const [, txs] of groups.expEntries) {
			for (const tx of txs) {
				expect(Number(tx.amount)).toBeLessThan(0);
			}
		}
	});

	it("all transactions are accounted for (no loss)", () => {
		const groups = groupByCategory(syntheticTxs, emptyOverlay);
		const totalGrouped =
			groups.income.length +
			groups.expEntries.reduce((sum, [, txs]) => sum + txs.length, 0);
		expect(totalGrouped).toBe(syntheticTxs.length);
	});

	it("expense categories are sorted by absolute sum desc", () => {
		const groups = groupByCategory(syntheticTxs, emptyOverlay);
		const sums = groups.expEntries.map(([, txs]) => {
			const total = txs.reduce((s, tx) => s + Math.abs(Number(tx.amount)), 0);
			return total;
		});
		for (let i = 1; i < sums.length; i++) {
			expect(sums[i - 1]!).toBeGreaterThanOrEqual(sums[i]!);
		}
	});

	it('uncategorized transactions use "—" as key', () => {
		const txs = syntheticTxs.filter((t) => t.categoryId === null);
		const hasUncategorized = txs.some((t) => Number(t.amount) < 0);
		if (hasUncategorized) {
			const groups = groupByCategory(syntheticTxs, emptyOverlay);
			const uncatEntry = groups.expEntries.find(([cat]) => cat === "—");
			expect(uncatEntry).toBeDefined();
		}
	});

	it("overlay changes category grouping", () => {
		// Pick an expense transaction and overlay it to a new category
		const expenseTx = syntheticTxs.find((t) => Number(t.amount) < 0);
		expect(expenseTx).toBeDefined();
		if (!expenseTx) return;

		const overlay: ReviewOverlay = {
			transactionId: expenseTx.id,
			description: null,
			merchantName: null,
			purpose: null,
			categoryId: "teste:categoria-overlay",
		};
		const overlayMap = buildOverlayMap([overlay]);

		const groups = groupByCategory(syntheticTxs, overlayMap);
		const overlayEntry = groups.expEntries.find(
			([cat]) => cat === "teste:categoria-overlay",
		);
		expect(overlayEntry).toBeDefined();
		expect(overlayEntry![1].some((tx) => tx.id === expenseTx.id)).toBe(true);
	});
});

// ── groupHierarchical ──────────────────────────────────────────────────────

describe("groupHierarchical", () => {
	it("groups expenses by parent category", () => {
		const groups = groupHierarchical(syntheticTxs, emptyOverlay);
		// Should have at least some parent categories
		expect(groups.expenses.size).toBeGreaterThan(0);
	});

	it("parent total = sum of subcategory totals", () => {
		const groups = groupHierarchical(syntheticTxs, emptyOverlay);
		for (const [, parent] of groups.expenses) {
			const subsTotal = Array.from(parent.subs.values()).reduce(
				(sum, sub) => sum + sub.total,
				0,
			);
			// Floating point from sumAmounts division by 100; allow tiny diff
			expect(Math.abs(parent.total - subsTotal)).toBeLessThan(0.01);
		}
	});

	it("grand total of expenses = sum of all parent totals", () => {
		const groups = groupHierarchical(syntheticTxs, emptyOverlay);
		const parentsTotal = Array.from(groups.expenses.values()).reduce(
			(sum, p) => sum + p.total,
			0,
		);

		// Compare against flat sum of all expense amounts
		const allExpenseAmounts = syntheticTxs
			.filter((t) => Number(t.amount) < 0)
			.map((t) => t.amount);
		const flatTotal = Math.abs(sumAmounts(allExpenseAmounts));

		expect(Math.abs(parentsTotal - flatTotal)).toBeLessThan(0.01);
	});

	it("parents are sorted by total descending", () => {
		const groups = groupHierarchical(syntheticTxs, emptyOverlay);
		const totals = Array.from(groups.expenses.values()).map((p) => p.total);
		for (let i = 1; i < totals.length; i++) {
			expect(totals[i - 1]!).toBeGreaterThanOrEqual(totals[i]!);
		}
	});

	it("subcategories within parent are sorted by total descending", () => {
		const groups = groupHierarchical(syntheticTxs, emptyOverlay);
		for (const [, parent] of groups.expenses) {
			const subsTotals = Array.from(parent.subs.values()).map((s) => s.total);
			for (let i = 1; i < subsTotals.length; i++) {
				expect(subsTotals[i - 1]!).toBeGreaterThanOrEqual(subsTotals[i]!);
			}
		}
	});

	it("all expense transactions are accounted for in the hierarchy", () => {
		const groups = groupHierarchical(syntheticTxs, emptyOverlay);
		const hierarchicalCount =
			groups.income.length +
			Array.from(groups.expenses.values()).reduce(
				(sum, p) =>
					sum +
					Array.from(p.subs.values()).reduce((s, sub) => s + sub.txs.length, 0),
				0,
			);
		expect(hierarchicalCount).toBe(syntheticTxs.length);
	});

	it("overlay recategorization updates both groups correctly", () => {
		// Find an expense in alimentacao:mercado
		const tx = syntheticTxs.find(
			(t) => t.categoryId === "alimentacao:mercado" && Number(t.amount) < 0,
		);
		expect(tx).toBeDefined();
		if (!tx) return;

		// Overlay it to moradia:aluguel
		const overlay: ReviewOverlay = {
			transactionId: tx.id,
			description: null,
			merchantName: null,
			purpose: null,
			categoryId: "moradia:aluguel",
		};
		const overlayMap = buildOverlayMap([overlay]);

		const groups = groupHierarchical(syntheticTxs, overlayMap);

		// The transaction should now be under moradia → aluguel
		const moradia = groups.expenses.get("moradia");
		expect(moradia).toBeDefined();
		const aluguelSub = moradia!.subs.get("aluguel");
		expect(aluguelSub).toBeDefined();
		expect(aluguelSub!.txs.some((t) => t.id === tx.id)).toBe(true);

		// It should NOT be under alimentacao → mercado
		const alimentacao = groups.expenses.get("alimentacao");
		if (alimentacao) {
			const mercadoSub = alimentacao.subs.get("mercado");
			if (mercadoSub) {
				expect(mercadoSub.txs.some((t) => t.id === tx.id)).toBe(false);
			}
		}
	});
});

// ── transactionsForMonth ───────────────────────────────────────────────────

describe("transactionsForMonth", () => {
	it("filters to only the given month", () => {
		const month = syntheticTxs[0]!.month;
		const result = transactionsForMonth(syntheticTxs, month);
		for (const tx of result) {
			expect(tx.month).toBe(month);
		}
	});

	it("returns empty for a month with no transactions", () => {
		const result = transactionsForMonth(syntheticTxs, "1999-01");
		expect(result).toHaveLength(0);
	});
});

// ── computeMonthSums ───────────────────────────────────────────────────────

describe("computeMonthSums", () => {
	it("returns positive entradas and saidas", () => {
		const sums = computeMonthSums(syntheticTxs);
		expect(sums.entradas).toBeGreaterThanOrEqual(0);
		expect(sums.saidas).toBeGreaterThanOrEqual(0);
	});

	it("entradas + (-saidas) = overall net", () => {
		const net = sumAmounts(syntheticTxs.map((t) => t.amount));
		const sums = computeMonthSums(syntheticTxs);
		// net = entradas - saidas
		// entradas - saidas = net
		expect(Math.abs(sums.entradas - sums.saidas - net)).toBeLessThan(0.01);
	});
});

// ── effectiveCategory / buildOverlayMap ─────────────────────────────────────

describe("effectiveCategory", () => {
	it("returns overlay category when present", () => {
		const tx: TxView = {
			id: "tx-001",
			accountId: "acc-1",
			postedAt: "2026-01-15",
			amount: "-50.00",
			rawDescription: "test",
			description: null,
			merchantName: null,
			purpose: null,
			categoryId: "original-cat",
			month: "2026-01",
			paymentStatus: "cleared",
			reviewed: 0,
			isInstallment: 0,
			isSubscription: 0,
		};

		const overlay: ReviewOverlay = {
			transactionId: "tx-001",
			description: null,
			merchantName: null,
			purpose: null,
			categoryId: "overridden-cat",
		};
		expect(effectiveCategory(tx, buildOverlayMap([overlay]))).toBe(
			"overridden-cat",
		);
	});

	it("falls back to seed category when no overlay", () => {
		const tx: TxView = {
			id: "tx-002",
			accountId: "acc-1",
			postedAt: "2026-01-15",
			amount: "-30.00",
			rawDescription: "test",
			description: null,
			merchantName: null,
			purpose: null,
			categoryId: "seed-cat",
			month: "2026-01",
			paymentStatus: "cleared",
			reviewed: 0,
			isInstallment: 0,
			isSubscription: 0,
		};
		expect(effectiveCategory(tx, emptyOverlay)).toBe("seed-cat");
	});

	it("returns null when neither overlay nor seed has category", () => {
		const tx: TxView = {
			id: "tx-003",
			accountId: "acc-1",
			postedAt: "2026-01-15",
			amount: "-10.00",
			rawDescription: "test",
			description: null,
			merchantName: null,
			purpose: null,
			categoryId: null,
			month: "2026-01",
			paymentStatus: "cleared",
			reviewed: 0,
			isInstallment: 0,
			isSubscription: 0,
		};
		expect(effectiveCategory(tx, emptyOverlay)).toBeNull();
	});
});
