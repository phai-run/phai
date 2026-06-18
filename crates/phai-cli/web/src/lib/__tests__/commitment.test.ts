/**
 * Unit tests for the commitment-tier axis (ADR-0030): the derived
 * controllability classification that the sheet, treemap, and planning views
 * all read so they segment the same money the same way.
 */
import { describe, expect, it } from "vitest";
import {
	buildAccountMap,
	commitmentTier,
	filterTransactions,
	fixedCategoriesFromForecasts,
	type ReviewOverlay,
	type TxFilters,
	type TxView,
} from "../derivations";

const mk = (id: string, patch: Partial<TxView> = {}): TxView => ({
	id,
	accountId: "acc",
	postedAt: "2026-06-05",
	amount: "-100.00",
	rawDescription: "x",
	description: null,
	merchantName: null,
	purpose: null,
	categoryId: null,
	month: "2026-06",
	paymentStatus: "posted",
	reviewed: 1,
	isInstallment: 0,
	isSubscription: 0,
	...patch,
});

describe("fixedCategoriesFromForecasts", () => {
	it("collects parent categories of fixed-kind forecasts only", () => {
		const set = fixedCategoriesFromForecasts([
			{ kind: "fixed", categoryId: "moradia:aluguel" },
			{ kind: "fixed", categoryId: "saude:terapia" },
			{ kind: "subscription", categoryId: "assinaturas:streaming" },
			{ kind: "installment", categoryId: "eletronicos:notebook" },
			{ kind: "fixed", categoryId: null },
		]);
		expect([...set].sort()).toEqual(["moradia", "saude"]);
	});

	it("returns an empty set when there are no fixed forecasts", () => {
		expect(fixedCategoriesFromForecasts([]).size).toBe(0);
	});
});

describe("commitmentTier", () => {
	const fixed = new Set(["moradia", "saude"]);

	it("classifies installments as locked regardless of category", () => {
		expect(commitmentTier(mk("a", { isInstallment: 1 }), fixed)).toBe("locked");
		expect(
			commitmentTier(mk("b", { isInstallment: 1, categoryId: "lazer" }), fixed),
		).toBe("locked");
	});

	it("classifies subscriptions as cancellable", () => {
		expect(
			commitmentTier(
				mk("a", { isSubscription: 1, categoryId: "assinaturas:streaming" }),
				fixed,
			),
		).toBe("cancellable");
	});

	it("classifies fixed-category spend as locked", () => {
		expect(commitmentTier(mk("a", { categoryId: "moradia:aluguel" }), fixed)).toBe(
			"locked",
		);
		expect(commitmentTier(mk("b", { categoryId: "saude" }), fixed)).toBe("locked");
	});

	it("defaults everything else to variable", () => {
		expect(commitmentTier(mk("a", { categoryId: "mercado" }), fixed)).toBe(
			"variable",
		);
		expect(commitmentTier(mk("b", { categoryId: null }), fixed)).toBe("variable");
	});

	it("lets the subscription flag win over a fixed category (subs are cancellable)", () => {
		expect(
			commitmentTier(
				mk("a", { isSubscription: 1, categoryId: "moradia:streaming" }),
				fixed,
			),
		).toBe("cancellable");
	});

	it("treats an unprovided fixed set as no fixed categories", () => {
		expect(commitmentTier(mk("a", { categoryId: "moradia" }))).toBe("variable");
	});

	it("lets a per-transaction override win over every derived signal", () => {
		// An installment that the user pinned as variable reads as variable.
		expect(
			commitmentTier(mk("a", { isInstallment: 1, commitmentTier: "variable" }), fixed),
		).toBe("variable");
		// A plain tx pinned to locked.
		expect(
			commitmentTier(mk("b", { categoryId: "mercado", commitmentTier: "locked" }), fixed),
		).toBe("locked");
	});

	it("ignores an invalid override value and falls back to derived", () => {
		expect(
			commitmentTier(mk("a", { isSubscription: 1, commitmentTier: "bogus" }), fixed),
		).toBe("cancellable");
	});

	it("prefers the optimistic overlay tier over the seeded override", () => {
		const overlay = new Map([
			[
				"a",
				{
					transactionId: "a",
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: null,
					commitmentTier: "cancellable",
				},
			],
		]);
		expect(
			commitmentTier(
				mk("a", { commitmentTier: "locked" }),
				fixed,
				overlay,
			),
		).toBe("cancellable");
	});
});

describe("filterTransactions tierFilter", () => {
	const overlay = new Map<string, ReviewOverlay>();
	const accountMap = buildAccountMap([]);
	const base: TxFilters = {
		accountFilter: null,
		ownerFilter: null,
		categoryFilter: null,
		textFilter: null,
		installmentsOnly: false,
		subscriptionsOnly: false,
		unreviewedOnly: false,
	};
	const fixed = new Set(["moradia"]);
	const rows = [
		mk("inst", { isInstallment: 1 }),
		mk("sub", { isSubscription: 1 }),
		mk("rent", { categoryId: "moradia:aluguel" }),
		mk("market", { categoryId: "mercado" }),
	];

	it("keeps only locked rows", () => {
		const r = filterTransactions(
			rows,
			{ ...base, tierFilter: "locked" },
			overlay,
			accountMap,
			fixed,
		);
		expect(r.map((t) => t.id).sort()).toEqual(["inst", "rent"]);
	});

	it("keeps only cancellable rows", () => {
		const r = filterTransactions(
			rows,
			{ ...base, tierFilter: "cancellable" },
			overlay,
			accountMap,
			fixed,
		);
		expect(r.map((t) => t.id)).toEqual(["sub"]);
	});

	it("keeps only variable rows", () => {
		const r = filterTransactions(
			rows,
			{ ...base, tierFilter: "variable" },
			overlay,
			accountMap,
			fixed,
		);
		expect(r.map((t) => t.id)).toEqual(["market"]);
	});

	it("passes everything through when tierFilter is null", () => {
		const r = filterTransactions(rows, base, overlay, accountMap, fixed);
		expect(r.length).toBe(4);
	});
});
