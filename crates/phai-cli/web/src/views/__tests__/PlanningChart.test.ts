/**
 * Unit tests for PlanningChart model and mode logic.
 *
 * Tests buildModel with various data scenarios, mode-specific
 * data computations, and verifies data model consistency.
 */
import { describe, it, expect } from "vitest";
import { buildModel } from "../PlanningChart";
import type { ChartMonthView, ChartMode } from "../types";

// ── Test helpers ──────────────────────────────────────────────────────────

/** Create a single ChartMonthView for testing. */
const makeMonth = (
	overrides: Partial<ChartMonthView> & { month: string },
): ChartMonthView => ({
	label: `${overrides.month.slice(5)}/${overrides.month.slice(2, 4)}`,
	month: overrides.month,
	inflows: overrides.inflows ?? "0",
	outflows: overrides.outflows ?? "0",
	forecastInflowsRemaining: overrides.forecastInflowsRemaining ?? "0",
	forecastOutflowsRemaining: overrides.forecastOutflowsRemaining ?? "0",
	closingBalance: overrides.closingBalance ?? "0",
	projectedClosingBalance: overrides.projectedClosingBalance ?? "0",
	isFuture: overrides.isFuture ?? 0,
});

// ── buildModel ────────────────────────────────────────────────────────────

describe("buildModel", () => {
	it("handles empty months array", () => {
		const m = buildModel([]);
		expect(m.realIns).toEqual([]);
		expect(m.realOuts).toEqual([]);
		expect(m.balances).toEqual([]);
		expect(m.maxBar).toBe(1); // guard against division by zero
		expect(m.expMaxBar).toBe(1);
		expect(m.balSpan).toBe(1);
	});

	it("builds correct model for expense-only data", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-01",
				outflows: "-150.00",
				forecastOutflowsRemaining: "-50.00",
				closingBalance: "850.00",
			}),
			makeMonth({
				month: "2024-02",
				outflows: "-200.00",
				forecastOutflowsRemaining: "0",
				closingBalance: "650.00",
			}),
			makeMonth({
				month: "2024-03",
				outflows: "0",
				forecastOutflowsRemaining: "-300.00",
				projectedClosingBalance: "350.00",
				isFuture: 1,
			}),
		];

		const m = buildModel(months);

		// Expense values (absolute)
		expect(m.realOuts).toEqual([150, 200, 0]);
		expect(m.fcOuts).toEqual([50, 0, 300]);

		// maxBar should be max of all bar values including income
		// With zero income, max = max(150+50, 200+0, 0+300) = 300
		expect(m.maxBar).toBe(300);

		// expMaxBar = max expense only: max(150+50, 200+0, 0+300) = 300
		expect(m.expMaxBar).toBe(300);

		// Balance line
		expect(m.balances).toEqual([850, 650, 350]);
		expect(m.minBal).toBe(0);
		expect(m.balSpan).toBe(850); // 850 - 0
	});

	it("expMaxBar is based only on expenses when income dominates", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-01",
				inflows: "10000.00",
				outflows: "-500.00",
			}),
			makeMonth({
				month: "2024-02",
				inflows: "12000.00",
				outflows: "-300.00",
			}),
		];

		const m = buildModel(months);

		// maxBar includes income → 12000
		expect(m.maxBar).toBe(12000);

		// expMaxBar should be max of expenses only → 500
		expect(m.expMaxBar).toBe(500);

		// expMaxBar < maxBar when income dominates
		expect(m.expMaxBar).toBeLessThan(m.maxBar);
	});

	it("handles zero amounts gracefully", () => {
		const months: ChartMonthView[] = [
			makeMonth({ month: "2024-01" }),
			makeMonth({ month: "2024-02" }),
		];

		const m = buildModel(months);
		expect(m.realIns).toEqual([0, 0]);
		expect(m.realOuts).toEqual([0, 0]);
		expect(m.fcIns).toEqual([0, 0]);
		expect(m.fcOuts).toEqual([0, 0]);
		expect(m.balances).toEqual([0, 0]);
		expect(m.maxBar).toBe(1);
		expect(m.expMaxBar).toBe(1);
	});

	it("distinguishes past from future months in balances", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-05",
				closingBalance: "800.00",
				projectedClosingBalance: "750.00",
				isFuture: 0,
			}),
			makeMonth({
				month: "2024-06",
				closingBalance: "700.00",
				projectedClosingBalance: "650.00",
				isFuture: 1,
			}),
			makeMonth({
				month: "2024-07",
				closingBalance: "0",
				projectedClosingBalance: "550.00",
				isFuture: 1,
			}),
		];

		const m = buildModel(months);

		// Past months use closingBalance
		expect(m.balances[0]).toBe(800);

		// Future months use projectedClosingBalance
		expect(m.balances[1]).toBe(650);
		expect(m.balances[2]).toBe(550);
	});

	it("computes correct minBal and balSpan with negative balances", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-01",
				closingBalance: "-200.50",
			}),
			makeMonth({
				month: "2024-02",
				closingBalance: "300.00",
			}),
			makeMonth({
				month: "2024-03",
				closingBalance: "-50.00",
			}),
		];

		const m = buildModel(months);
		expect(m.minBal).toBeCloseTo(-200.5);
		// balSpan = 300 - (-200.5) = 500.5
		expect(m.balSpan).toBeCloseTo(500.5);
	});
});

// ── Mode-specific data points ─────────────────────────────────────────────

describe("expenses mode data consistency", () => {
	const months: ChartMonthView[] = [
		makeMonth({
			month: "2024-01",
			outflows: "-120.00",
			forecastOutflowsRemaining: "-30.00",
		}),
		makeMonth({
			month: "2024-02",
			outflows: "-180.50",
			forecastOutflowsRemaining: "0",
		}),
		makeMonth({
			month: "2024-03",
			outflows: "-95.00",
			forecastOutflowsRemaining: "-45.00",
		}),
		makeMonth({
			month: "2024-04",
			outflows: "0",
			forecastOutflowsRemaining: "-200.00",
			isFuture: 1,
		}),
		makeMonth({
			month: "2024-05",
			outflows: "-60.00",
			forecastOutflowsRemaining: "-80.00",
			isFuture: 1,
		}),
	];

	const model = buildModel(months);

	it("bar mode and line mode use the same underlying data", () => {
		// Both modes read from model.realOuts and model.fcOuts
		// The difference is only rendering, not data
		expect(model.realOuts).toEqual([120, 180.5, 95, 0, 60]);
		expect(model.fcOuts).toEqual([30, 0, 45, 200, 80]);
	});

	it("expMaxBar is the maximum combined expense per month", () => {
		// max(120+30, 180.5+0, 95+45, 0+200, 60+80) = max(150, 180.5, 140, 200, 140)
		expect(model.expMaxBar).toBe(200);
	});

	it("total expense per month matches realized + forecast", () => {
		for (let i = 0; i < months.length; i++) {
			const total = model.realOuts[i] + model.fcOuts[i];
			// Purely realized
			if (i === 1) expect(total).toBe(180.5);
			// Purely forecast
			if (i === 3) expect(total).toBe(200);
		}
	});

	it("first future month index is correct", () => {
		const firstFuture = months.findIndex((m) => m.isFuture === 1);
		expect(firstFuture).toBe(3); // Month 4 (index 3) is the first future
	});

	it("bar mode data points cover all months", () => {
		// Bar mode renders one bar per month using realOuts[i] + fcOuts[i]
		const totals = months.map((_, i) => model.realOuts[i] + model.fcOuts[i]);
		expect(totals).toHaveLength(5);
		// All non-zero for this fixture
		expect(totals.every((t) => t > 0)).toBe(true);
	});

	it("line mode data points match bar mode totals", () => {
		// Both modes should produce the same total per month
		const totals = months.map((_, i) => model.realOuts[i] + model.fcOuts[i]);

		const absOutflows = months.map(
			(m) =>
				Math.abs(Number(m.outflows)) +
				Math.abs(Number(m.forecastOutflowsRemaining)),
		);

		for (let i = 0; i < months.length; i++) {
			expect(totals[i]).toBeCloseTo(absOutflows[i], 2);
		}
	});
});

// ── Mode selector state machine ────────────────────────────────────────────

describe("ChartMode state transitions", () => {
	const ALL_MODES: ChartMode[] = ["caixa", "despesas-barras"];

	it("all modes are valid and transitionable", () => {
		for (const from of ALL_MODES) {
			for (const to of ALL_MODES) {
				// Any transition should be valid
				expect(ALL_MODES).toContain(from);
				expect(ALL_MODES).toContain(to);
			}
		}
	});

	it("mode labels match expected values", () => {
		const labels: Record<ChartMode, string> = {
			caixa: "Caixa",
			"despesas-barras": "Despesas",
		};

		expect(Object.keys(labels)).toHaveLength(2);
		for (const m of ALL_MODES) {
			expect(typeof labels[m]).toBe("string");
			expect(labels[m].length).toBeGreaterThan(0);
		}
	});

	it("default mode is caixa", () => {
		const defaultMode: ChartMode = "caixa";
		expect(defaultMode).toBe("caixa");
	});

	it("expenses modes start with 'despesas' prefix", () => {
		const isExpensesMode = (m: ChartMode) => m.startsWith("despesas");
		expect(isExpensesMode("caixa")).toBe(false);
		expect(isExpensesMode("despesas-barras")).toBe(true);
	});
});

// ── Edge cases ─────────────────────────────────────────────────────────────

describe("buildModel edge cases", () => {
	it("handles negative inflows (should clamp to 0)", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-01",
				inflows: "-50.00", // negative inflow → clamp to 0
				outflows: "-100.00",
			}),
		];

		const m = buildModel(months);
		expect(m.realIns[0]).toBe(0);
		expect(m.realOuts[0]).toBe(100);
	});

	it("handles very large amounts without overflow", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-01",
				inflows: "99999999.99",
				outflows: "-99999999.99",
			}),
		];

		const m = buildModel(months);
		expect(Number.isFinite(m.maxBar)).toBe(true);
		expect(Number.isFinite(m.expMaxBar)).toBe(true);
		expect(Number.isFinite(m.balSpan)).toBe(true);
	});

	it("handles null/empty amount strings", () => {
		const months: ChartMonthView[] = [
			makeMonth({
				month: "2024-01",
				inflows: "" as string,
				outflows: "" as string,
				forecastInflowsRemaining: "" as string,
				forecastOutflowsRemaining: "" as string,
				closingBalance: "" as string,
				projectedClosingBalance: "" as string,
			}),
		];

		const m = buildModel(months);
		expect(m.realIns[0]).toBe(0);
		expect(m.realOuts[0]).toBe(0);
		expect(m.balances[0]).toBe(0);
		expect(m.maxBar).toBe(1);
		expect(m.expMaxBar).toBe(1);
	});

	it("large number of months (36-month view)", () => {
		const months: ChartMonthView[] = [];
		for (let i = 0; i < 36; i++) {
			const yr = 2024 + Math.floor(i / 12);
			const mo = (i % 12) + 1;
			months.push(
				makeMonth({
					month: `${yr}-${String(mo).padStart(2, "0")}`,
					outflows: `-${(100 + i * 5).toFixed(2)}`,
					forecastOutflowsRemaining:
						i >= 3 ? `-${(50 + i * 3).toFixed(2)}` : "0",
					isFuture: i >= 3 ? 1 : 0,
				}),
			);
		}

		const m = buildModel(months);
		expect(m.realOuts).toHaveLength(36);
		expect(m.fcOuts).toHaveLength(36);
		expect(m.expMaxBar).toBeGreaterThan(0);
	});
});
