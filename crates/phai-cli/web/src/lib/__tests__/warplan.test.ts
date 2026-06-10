/**
 * Unit tests for the planilha sorting + plano-de-guerra derivations.
 *
 * All data is synthetic (AGENTS.md §1). Amounts are decimal strings; sums use
 * integer-cent math so assertions are exact.
 */
import { describe, expect, it } from "vitest";
import {
	buildWarPlan,
	previousMonths,
	sheetLabel,
	simulateWarPlan,
	sortForSheet,
	type AccountInfo,
	type PlanForecast,
	type TxView,
} from "../derivations";

const tx = (over: Partial<TxView> & { id: string }): TxView => ({
	accountId: "acc-1",
	postedAt: "2026-06-05",
	amount: "-100.00",
	rawDescription: "RAW DESC",
	description: null,
	merchantName: null,
	purpose: null,
	categoryId: null,
	month: "2026-06",
	paymentStatus: "posted",
	installmentMarker: null,
	reviewed: 0,
	isInstallment: 0,
	isSubscription: 0,
	...over,
});

const forecast = (over: Partial<PlanForecast>): PlanForecast => ({
	amount: "-100.00",
	categoryId: null,
	kind: "manual",
	status: "ativo",
	month: "2026-06",
	...over,
});

const noOverlay = new Map();
const noAccounts = new Map<string, AccountInfo>();

// ── previousMonths ──────────────────────────────────────────────────────────

describe("previousMonths", () => {
	it("walks back across a year boundary", () => {
		expect(previousMonths("2026-02", 3)).toEqual([
			"2026-01",
			"2025-12",
			"2025-11",
		]);
	});

	it("returns empty for malformed keys", () => {
		expect(previousMonths("garbage", 3)).toEqual([]);
	});
});

// ── sortForSheet ────────────────────────────────────────────────────────────

describe("sortForSheet", () => {
	const rows = [
		tx({ id: "a", amount: "-50.00", postedAt: "2026-06-03", categoryId: "b-cat" }),
		tx({ id: "b", amount: "-150.00", postedAt: "2026-06-01", categoryId: "a-cat" }),
		tx({ id: "c", amount: "75.00", postedAt: "2026-06-02", categoryId: null }),
	];

	it("sorts by amount ascending (most negative first)", () => {
		const sorted = sortForSheet(rows, { key: "amount", dir: 1 }, noOverlay, noAccounts);
		expect(sorted.map((t) => t.id)).toEqual(["b", "a", "c"]);
	});

	it("sorts by date descending", () => {
		const sorted = sortForSheet(rows, { key: "date", dir: -1 }, noOverlay, noAccounts);
		expect(sorted.map((t) => t.id)).toEqual(["a", "c", "b"]);
	});

	it("sorts by category using the overlay-effective value", () => {
		const overlay = new Map([
			[
				"c",
				{
					transactionId: "c",
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: "zz-cat",
				},
			],
		]);
		const sorted = sortForSheet(rows, { key: "category", dir: 1 }, overlay, noAccounts);
		expect(sorted.map((t) => t.id)).toEqual(["b", "a", "c"]);
	});

	it("does not mutate the input", () => {
		const before = rows.map((t) => t.id);
		sortForSheet(rows, { key: "amount", dir: 1 }, noOverlay, noAccounts);
		expect(rows.map((t) => t.id)).toEqual(before);
	});
});

describe("sheetLabel", () => {
	it("prefers human description, then merchant, then raw", () => {
		expect(sheetLabel(tx({ id: "x", description: "Almoço" }))).toBe("Almoço");
		expect(sheetLabel(tx({ id: "y", merchantName: "Bistrô" }))).toBe("Bistrô");
		expect(sheetLabel(tx({ id: "z" }))).toBe("RAW DESC");
	});
});

// ── buildWarPlan ────────────────────────────────────────────────────────────

describe("buildWarPlan", () => {
	const transactions = [
		// Selected month: two food expenses + one housing + income (ignored).
		tx({ id: "m1", categoryId: "alimentacao:mercado", amount: "-300.00" }),
		tx({ id: "m2", categoryId: "alimentacao:restaurantes", amount: "-200.00" }),
		tx({ id: "m3", categoryId: "moradia:aluguel", amount: "-1000.00" }),
		tx({ id: "m4", categoryId: "receitas:salario", amount: "5000.00" }),
		// History (3 previous months) for the average.
		tx({ id: "h1", month: "2026-05", categoryId: "alimentacao:mercado", amount: "-600.00" }),
		tx({ id: "h2", month: "2026-04", categoryId: "alimentacao:delivery", amount: "-300.00" }),
		tx({ id: "h3", month: "2026-03", categoryId: "moradia:aluguel", amount: "-1000.00" }),
		// Outside the window — ignored.
		tx({ id: "h4", month: "2026-01", categoryId: "alimentacao:mercado", amount: "-999.00" }),
	];

	const forecasts = [
		forecast({ categoryId: "alimentacao", amount: "-700.00" }), // envelope
		forecast({ categoryId: "lazer", amount: "-150.00" }), // envelope without realized
		forecast({ categoryId: "moradia:servicos", amount: "-80.00" }), // sub-level: not a budget
		forecast({ categoryId: null, amount: "-2000.00" }), // card bill: ignored
		forecast({ categoryId: null, amount: "-120.00", kind: "installment" }), // committed
		forecast({ categoryId: "alimentacao", amount: "-50.00", status: "inativo" }), // inactive: ignored
		forecast({ categoryId: "receitas", amount: "900.00" }), // income: ignored
	];

	it("builds per-parent rows with realized, envelope, 3m average and projection", () => {
		const plan = buildWarPlan(transactions, "2026-06", forecasts, noOverlay);
		const byParent = new Map(plan.rows.map((r) => [r.parent, r]));

		const food = byParent.get("alimentacao")!;
		expect(food.realizado).toBe(500);
		expect(food.orcamento).toBe(700);
		expect(food.media3m).toBe(300); // (600 + 300) / 3
		expect(food.projecao).toBe(700); // max(realizado, orçamento)

		const housing = byParent.get("moradia")!;
		expect(housing.realizado).toBe(1000);
		expect(housing.orcamento).toBeNull();
		expect(housing.projecao).toBe(1000);

		// Envelope with no realized spend still shows up.
		const leisure = byParent.get("lazer")!;
		expect(leisure.realizado).toBe(0);
		expect(leisure.projecao).toBe(150);

		expect(plan.parcelasComprometidas).toBe(120);
		expect(plan.totalRealizado).toBe(1500);
		expect(plan.totalProjecao).toBe(700 + 1000 + 150);
	});

	it("rows are sorted by projection descending", () => {
		const plan = buildWarPlan(transactions, "2026-06", forecasts, noOverlay);
		const projections = plan.rows.map((r) => r.projecao);
		expect(projections).toEqual([...projections].sort((a, b) => b - a));
	});

	it("past mode projects plain realized (no envelope max)", () => {
		const plan = buildWarPlan(transactions, "2026-06", forecasts, noOverlay, "past");
		const food = plan.rows.find((r) => r.parent === "alimentacao")!;
		expect(food.projecao).toBe(500);
	});

	it("applies the review overlay before bucketing", () => {
		const overlay = new Map([
			[
				"m3",
				{
					transactionId: "m3",
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: "lazer:viagem",
				},
			],
		]);
		const plan = buildWarPlan(transactions, "2026-06", forecasts, overlay);
		expect(plan.rows.find((r) => r.parent === "moradia")).toBeUndefined();
		expect(plan.rows.find((r) => r.parent === "lazer")!.realizado).toBe(1000);
	});
});

// ── simulateWarPlan ─────────────────────────────────────────────────────────

describe("simulateWarPlan", () => {
	const transactions = [
		tx({ id: "m1", categoryId: "alimentacao:mercado", amount: "-500.00" }),
		tx({ id: "m2", categoryId: "moradia:aluguel", amount: "-1000.00" }),
	];
	const forecasts = [
		forecast({ categoryId: "alimentacao", amount: "-700.00" }),
		forecast({ categoryId: "moradia", amount: "-1200.00" }),
	];
	const plan = buildWarPlan(transactions, "2026-06", forecasts, noOverlay);

	it("keeps the baseline when no targets are set", () => {
		const sim = simulateWarPlan(plan, new Map());
		expect(sim.projecaoSimulada).toBe(plan.totalProjecao);
		expect(sim.economiaMes).toBe(0);
	});

	it("cuts an envelope down to the target, floored at realized spend", () => {
		// Food: target 600 < envelope 700, above realized 500 → contributes 600.
		// Housing: target 800 < realized 1000 → floored at 1000 (already spent).
		const sim = simulateWarPlan(
			plan,
			new Map([
				["alimentacao", 600],
				["moradia", 800],
			]),
		);
		expect(sim.projecaoSimulada).toBe(600 + 1000);
		expect(sim.economiaMes).toBe(plan.totalProjecao - 1600);
	});

	it("never lets a negative target below zero", () => {
		const sim = simulateWarPlan(plan, new Map([["alimentacao", -50]]));
		// max(realizado=500, max(0, -50)) = 500.
		expect(sim.projecaoSimulada).toBe(500 + 1200);
	});
});
