/**
 * Unit tests for the cash-decision hero's summary derivation.
 *
 * `cashSummary` powers the headline balance + entradas/saídas/resultado KPIs.
 * Arithmetic must run in exact cents (no float drift) and the headline balance
 * must follow realized-vs-projected rules by `when`.
 */
import { describe, it, expect } from "vitest";
import { cashSummary } from "../cash/CashDecisionPanel";
import type { ChartMonthView } from "../types";

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

describe("cashSummary", () => {
	it("sums realized + forecast for entradas and saídas", () => {
		const row = makeMonth({
			month: "2026-06",
			inflows: "1000.00",
			forecastInflowsRemaining: "200.00",
			outflows: "-300.00",
			forecastOutflowsRemaining: "-100.00",
			closingBalance: "5000.00",
			projectedClosingBalance: "5500.00",
		});
		const s = cashSummary(row, "current");
		expect(s.entradas).toBe(1200);
		expect(s.saidas).toBe(400);
		expect(s.resultado).toBe(800);
		expect(s.saldo).toBe(5000); // current → realized closing
		expect(s.projetado).toBe(5500);
		expect(s.positive).toBe(true);
	});

	it("uses projected closing balance as headline for future months", () => {
		const row = makeMonth({
			month: "2026-12",
			closingBalance: "0",
			projectedClosingBalance: "8200.00",
			isFuture: 1,
		});
		const s = cashSummary(row, "future");
		expect(s.saldo).toBe(8200);
		expect(s.projetado).toBe(8200);
	});

	it("uses realized closing balance as headline for past months", () => {
		const row = makeMonth({
			month: "2026-01",
			closingBalance: "3100.00",
			projectedClosingBalance: "3100.00",
		});
		expect(cashSummary(row, "past").saldo).toBe(3100);
	});

	it("flags a negative month when saídas exceed entradas", () => {
		const row = makeMonth({
			month: "2026-06",
			inflows: "500.00",
			outflows: "-900.00",
		});
		const s = cashSummary(row, "current");
		expect(s.resultado).toBe(-400);
		expect(s.positive).toBe(false);
	});

	it("the header badge tracks the balance, not the month result", () => {
		// Deficit month (saídas > entradas) but a positive cash balance: the
		// badge must read positive (it sits next to the balance), while the
		// result KPI stays negative.
		const row = makeMonth({
			month: "2026-06",
			inflows: "17353.29",
			outflows: "-35903.54",
			closingBalance: "7360.75",
		});
		const s = cashSummary(row, "current");
		expect(s.positive).toBe(false); // month net is in the red
		expect(s.balancePositive).toBe(true); // but the balance is positive
	});

	it("computes sums in exact cents (no float drift)", () => {
		const row = makeMonth({
			month: "2026-06",
			inflows: "0.10",
			forecastInflowsRemaining: "0.20",
			outflows: "-0.10",
		});
		const s = cashSummary(row, "current");
		expect(s.entradas).toBe(0.3);
		expect(s.saidas).toBe(0.1);
		expect(s.resultado).toBeCloseTo(0.2, 10);
	});
});
