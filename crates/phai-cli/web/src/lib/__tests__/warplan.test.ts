/**
 * Unit tests for the planilha sorting + plano-de-guerra derivations.
 *
 * All data is synthetic (AGENTS.md §1). Amounts are decimal strings; sums use
 * integer-cent math so assertions are exact.
 */
import { describe, expect, it } from "vitest";
import {
	buildEnvelopeWrites,
	buildWarPlan,
	previousMonths,
	sheetLabel,
	simulateWarPlan,
	simulateWarPlanGoals,
	sortForSheet,
	type AccountInfo,
	type EnvelopeForecastRef,
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
		// moradia keeps a history-only row (its month spend moved to lazer).
		const housing = plan.rows.find((r) => r.parent === "moradia")!;
		expect(housing.realizado).toBe(0);
		expect(housing.projecao).toBe(0);
		expect(plan.rows.find((r) => r.parent === "lazer")!.realizado).toBe(1000);
	});

	it("flags locked subs read-only while keeping their spend visible (ADR-0030)", () => {
		const txs = [
			// variable — simulatable.
			tx({ id: "v", categoryId: "lazer:bar", amount: "-200.00" }),
			// installment — locked.
			tx({
				id: "i",
				categoryId: "lazer:parcelado",
				amount: "-300.00",
				isInstallment: 1,
			}),
			// fixed-category bill — locked via the fixed envelope below.
			tx({ id: "f", categoryId: "moradia:aluguel", amount: "-1000.00" }),
			// manual lock override.
			tx({
				id: "o",
				categoryId: "lazer:show",
				amount: "-150.00",
				commitmentTier: "locked",
			}),
		];
		const fcs = [
			forecast({ categoryId: "moradia", amount: "-1000.00", kind: "fixed" }),
		];
		const plan = buildWarPlan(txs, "2026-06", fcs, noOverlay);

		// All spend stays visible — locked is shown, not hidden.
		expect(plan.totalRealizado).toBe(1650);

		const lazer = plan.rows.find((r) => r.parent === "lazer")!;
		const subLocked = (sub: string) =>
			lazer.subs.find((s) => s.sub === sub)!.locked;
		expect(subLocked("bar")).toBe(false); // variable → simulatable
		expect(subLocked("parcelado")).toBe(true); // installment → read-only
		expect(subLocked("show")).toBe(true); // manual lock → read-only

		const aluguel = plan.rows
			.find((r) => r.parent === "moradia")!
			.subs.find((s) => s.sub === "aluguel")!;
		expect(aluguel.realizado).toBe(1000); // visible
		expect(aluguel.locked).toBe(true); // but read-only
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

// ── buildWarPlan subs (per-subcategory slider rows) ─────────────────────────

describe("buildWarPlan subs", () => {
	const transactions = [
		tx({ id: "s1", categoryId: "alimentacao:mercado", amount: "-300.00" }),
		tx({ id: "s2", categoryId: "alimentacao:restaurantes", amount: "-200.00" }),
		tx({ id: "s3", categoryId: "doacoes", amount: "-50.00" }), // flat category
		// History for the 3m averages.
		tx({ id: "s4", month: "2026-05", categoryId: "alimentacao:mercado", amount: "-600.00" }),
		tx({ id: "s5", month: "2026-04", categoryId: "alimentacao:delivery", amount: "-300.00" }),
		// History-only parent: no spend in the selected month.
		tx({ id: "s6", month: "2026-05", categoryId: "transporte:app", amount: "-90.00" }),
	];
	const forecasts = [
		forecast({ categoryId: "lazer", amount: "-150.00" }), // envelope-only parent
	];
	const plan = buildWarPlan(transactions, "2026-06", forecasts, noOverlay);
	const byParent = new Map(plan.rows.map((r) => [r.parent, r]));

	it("exposes per-sub realized + 3m average, sorted by weight desc", () => {
		const food = byParent.get("alimentacao")!;
		expect(food.subs.map((s) => s.sub)).toEqual([
			"mercado",
			"restaurantes",
			"delivery",
		]);
		const [mercado, restaurantes, delivery] = food.subs;
		expect(mercado).toMatchObject({
			categoryId: "alimentacao:mercado",
			realizado: 300,
			media3m: 200,
		});
		expect(restaurantes).toMatchObject({
			categoryId: "alimentacao:restaurantes",
			realizado: 200,
			media3m: 0,
		});
		expect(delivery).toMatchObject({
			categoryId: "alimentacao:delivery",
			realizado: 0,
			media3m: 100,
		});
	});

	it("goalBase opens at the 3m average, floored at realized spend", () => {
		const food = byParent.get("alimentacao")!;
		const bases = new Map(food.subs.map((s) => [s.sub, s.goalBase]));
		expect(bases.get("mercado")).toBe(300); // max(media 200, realizado 300)
		expect(bases.get("restaurantes")).toBe(200); // no history → realized
		expect(bases.get("delivery")).toBe(100); // history only → media3m
	});

	it("flat categories become a single '—' sub keyed by the parent id", () => {
		const flat = byParent.get("doacoes")!;
		expect(flat.subs).toHaveLength(1);
		expect(flat.subs[0]).toMatchObject({
			sub: "—",
			categoryId: "doacoes",
			realizado: 50,
			goalBase: 50,
		});
	});

	it("includes history-only parents so their goals are settable", () => {
		const transport = byParent.get("transporte")!;
		expect(transport.realizado).toBe(0);
		expect(transport.orcamento).toBeNull();
		expect(transport.projecao).toBe(0); // contributes nothing to the baseline
		expect(transport.subs[0]).toMatchObject({
			sub: "app",
			categoryId: "transporte:app",
			media3m: 30,
			goalBase: 30,
		});
	});

	it("envelope-only parents get a pseudo-sub opening at the envelope", () => {
		const leisure = byParent.get("lazer")!;
		expect(leisure.subs).toHaveLength(1);
		expect(leisure.subs[0]).toMatchObject({
			sub: "—",
			categoryId: "lazer",
			realizado: 0,
			media3m: 0,
			goalBase: 150,
		});
	});
});

// ── simulateWarPlanGoals ────────────────────────────────────────────────────

describe("simulateWarPlanGoals", () => {
	const transactions = [
		tx({ id: "g1", categoryId: "alimentacao:mercado", amount: "-500.00" }),
		tx({ id: "g2", categoryId: "moradia:aluguel", amount: "-1000.00" }),
		tx({ id: "g3", month: "2026-05", categoryId: "alimentacao:mercado", amount: "-900.00" }),
		tx({ id: "g4", month: "2026-04", categoryId: "alimentacao:delivery", amount: "-300.00" }),
	];
	const forecasts = [
		forecast({ categoryId: "alimentacao", amount: "-700.00" }),
		forecast({ categoryId: "moradia", amount: "-1200.00" }),
	];
	// alimentacao: realizado 500, projecao 700.
	//   subs: mercado (realizado 500, media 300 → base 500), delivery (base 100).
	// moradia: realizado 1000, projecao 1200. sub aluguel base 1000.
	const plan = buildWarPlan(transactions, "2026-06", forecasts, noOverlay);

	it("keeps the baseline with no goals", () => {
		const sim = simulateWarPlanGoals(plan, new Map());
		expect(sim.projecaoSimulada).toBe(plan.totalProjecao);
		expect(sim.economiaMes).toBe(0);
		expect(sim.goalByParent.size).toBe(0);
	});

	it("a touched parent switches to goal mode; untouched stay at baseline", () => {
		const sim = simulateWarPlanGoals(
			plan,
			new Map([["alimentacao:mercado", 350]]),
		);
		// alimentacao simulated: max(500, 350) + max(0, 100) = 600.
		// moradia untouched → baseline 1200.
		expect(sim.projecaoSimulada).toBe(600 + 1200);
		expect(sim.economiaMes).toBe(1900 - 1800);
		// The envelope goal is what the sliders show, not the floored spend.
		expect(sim.goalByParent.get("alimentacao")).toBe(350 + 100);
		expect(sim.goalByParent.has("moradia")).toBe(false);
		expect(sim.simulatedByParent.get("alimentacao")).toBe(600);
		expect(sim.simulatedByParent.get("moradia")).toBe(1200); // baseline
	});

	it("goals above the projection yield a negative saving", () => {
		const sim = simulateWarPlanGoals(
			plan,
			new Map([["moradia:aluguel", 1500]]),
		);
		expect(sim.projecaoSimulada).toBe(700 + 1500);
		expect(sim.economiaMes).toBe(-300);
		expect(sim.goalByParent.get("moradia")).toBe(1500);
	});

	it("clamps negative goal values to zero", () => {
		const sim = simulateWarPlanGoals(
			plan,
			new Map([["alimentacao:mercado", -50]]),
		);
		expect(sim.projecaoSimulada).toBe(600 + 1200); // floor at realizado 500
		expect(sim.goalByParent.get("alimentacao")).toBe(0 + 100);
	});
});

// ── buildEnvelopeWrites ─────────────────────────────────────────────────────

describe("buildEnvelopeWrites", () => {
	const ref = (
		over: Partial<EnvelopeForecastRef> & { forecastId: string },
	): EnvelopeForecastRef => ({
		amount: "-100.00",
		categoryId: "alimentacao",
		kind: "manual",
		status: "ativo",
		month: "2026-06",
		...over,
	});

	const existing: EnvelopeForecastRef[] = [
		ref({ forecastId: "f-ali-jun", amount: "-700.00" }),
		ref({ forecastId: "f-ali-jun-2", amount: "-100.00" }),
		ref({ forecastId: "f-laz-jul", categoryId: "lazer", amount: "-150.00", month: "2026-07", status: "active" }),
		// None of these may be treated as envelopes:
		ref({ forecastId: "f-inst", kind: "installment" }),
		ref({ forecastId: "f-inactive", status: "inativo" }),
		ref({ forecastId: "f-sub", categoryId: "moradia:servicos" }),
		ref({ forecastId: "f-card", categoryId: null }),
		ref({ forecastId: "f-income", categoryId: "receitas", amount: "900.00" }),
	];

	it("updates existing envelopes and creates missing ones per month", () => {
		const writes = buildEnvelopeWrites(
			new Map([
				["alimentacao", 450],
				["lazer", 120],
			]),
			["2026-06", "2026-07"],
			existing,
		);
		expect(writes).toEqual([
			{
				forecastId: "f-ali-jun",
				month: "2026-06",
				categoryId: "alimentacao",
				// Sibling envelope f-ali-jun-2 keeps -100 → first one closes the gap.
				amount: "-350.00",
				description: "",
				dueDate: "2026-06-30",
			},
			{
				forecastId: null,
				month: "2026-07",
				categoryId: "alimentacao",
				amount: "-450.00",
				description: "meta alimentacao",
				dueDate: "2026-07-31",
			},
			{
				forecastId: null,
				month: "2026-06",
				categoryId: "lazer",
				amount: "-120.00",
				description: "meta lazer",
				dueDate: "2026-06-30",
			},
			{
				forecastId: "f-laz-jul",
				month: "2026-07",
				categoryId: "lazer",
				amount: "-120.00",
				description: "",
				dueDate: "2026-07-31",
			},
		]);
	});

	it("never flips an envelope positive when siblings already cover the goal", () => {
		const writes = buildEnvelopeWrites(
			new Map([["alimentacao", 50]]),
			["2026-06"],
			existing,
		);
		expect(writes[0]).toMatchObject({
			forecastId: "f-ali-jun",
			amount: "0.00",
		});
	});

	it("skips the uncategorized parent and handles leap February", () => {
		const writes = buildEnvelopeWrites(
			new Map([
				["—", 50],
				["lazer", 80],
			]),
			["2028-02"],
			[],
		);
		expect(writes).toEqual([
			{
				forecastId: null,
				month: "2028-02",
				categoryId: "lazer",
				amount: "-80.00",
				description: "meta lazer",
				dueDate: "2028-02-29",
			},
		]);
	});
});
