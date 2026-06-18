/**
 * Unit tests for the chart model + the war-plan goal simulation overlay.
 *
 * All data is synthetic (AGENTS.md §1). Amounts are decimal strings, as they
 * come out of the LiveStore chartMonths table.
 */
import { describe, expect, it } from "vitest";
import type { ChartMonthView } from "../../types";
import {
	applySimulationToModel,
	buildModel,
	firstShortfallMonth,
	solveRequiredSaving,
} from "../model";

const month = (over: Partial<ChartMonthView> & { month: string }): ChartMonthView => ({
	label: over.month.slice(5),
	inflows: "0",
	outflows: "0",
	forecastInflowsRemaining: "0",
	forecastOutflowsRemaining: "0",
	closingBalance: "0",
	projectedClosingBalance: "0",
	isFuture: 0,
	...over,
});

const months: ChartMonthView[] = [
	month({
		month: "2026-05",
		inflows: "5000",
		outflows: "4000",
		closingBalance: "1000",
	}),
	month({
		// Current month: realized + a remaining forecast tail.
		month: "2026-06",
		inflows: "5000",
		outflows: "2000",
		forecastOutflowsRemaining: "60",
		closingBalance: "1500",
		projectedClosingBalance: "1440",
	}),
	month({
		month: "2026-07",
		forecastInflowsRemaining: "5000",
		forecastOutflowsRemaining: "500",
		projectedClosingBalance: "5940",
		isFuture: 1,
	}),
	month({
		month: "2026-08",
		forecastInflowsRemaining: "5000",
		forecastOutflowsRemaining: "0",
		projectedClosingBalance: "10940",
		isFuture: 1,
	}),
];

describe("applySimulationToModel", () => {
	const base = buildModel(months);

	it("reduces forecast outflows from the simulated month on, clamped at zero", () => {
		const sim = applySimulationToModel(base, months, {
			fromMonth: "2026-06",
			monthlySaving: 100,
		});
		expect(sim.fcOuts).toEqual([0, 0, 400, 0]); // jun 60→0, jul 500→400, aug stays 0
		// Months before fromMonth and all other series are untouched.
		expect(sim.realOuts).toEqual(base.realOuts);
		expect(sim.realIns).toEqual(base.realIns);
		expect(sim.fcIns).toEqual(base.fcIns);
	});

	it("shifts only future balances by the cumulative applied saving", () => {
		const sim = applySimulationToModel(base, months, {
			fromMonth: "2026-06",
			monthlySaving: 100,
		});
		// Applied savings: jun 60 (clamped), jul 100, aug 0.
		expect(sim.balances[0]).toBe(base.balances[0]); // past
		expect(sim.balances[1]).toBe(base.balances[1]); // current = realized closing
		expect(sim.balances[2]).toBe(base.balances[2] + 60 + 100);
		expect(sim.balances[3]).toBe(base.balances[3] + 60 + 100 + 0);
	});

	it("a negative saving (goals above projection) raises outflows and lowers balances", () => {
		const sim = applySimulationToModel(base, months, {
			fromMonth: "2026-07",
			monthlySaving: -50,
		});
		expect(sim.fcOuts).toEqual([0, 60, 550, 50]);
		expect(sim.balances[2]).toBe(base.balances[2] - 50);
		expect(sim.balances[3]).toBe(base.balances[3] - 100);
	});

	it("recomputes the scale from the adjusted series", () => {
		const sim = applySimulationToModel(base, months, {
			fromMonth: "2026-06",
			monthlySaving: 100,
		});
		const rebuilt = Math.max(
			1,
			...sim.realOuts.map((v, i) => v + sim.fcOuts[i]),
		);
		expect(sim.expMaxBar).toBe(rebuilt);
	});

	it("is the identity for a zero saving", () => {
		const sim = applySimulationToModel(base, months, {
			fromMonth: "2026-06",
			monthlySaving: 0,
		});
		expect(sim).toEqual(base);
	});
});

// A year that dips negative in the future — the case the planner exists for.
const shortfallMonths: ChartMonthView[] = [
	month({ month: "2026-05", closingBalance: "1000" }),
	month({
		month: "2026-06",
		closingBalance: "500",
		forecastOutflowsRemaining: "100",
		projectedClosingBalance: "400",
	}),
	month({
		month: "2026-07",
		forecastInflowsRemaining: "1000",
		forecastOutflowsRemaining: "1600",
		projectedClosingBalance: "-200",
		isFuture: 1,
	}),
	month({
		month: "2026-08",
		forecastInflowsRemaining: "1000",
		forecastOutflowsRemaining: "1000",
		projectedClosingBalance: "-200",
		isFuture: 1,
	}),
];

describe("firstShortfallMonth", () => {
	it("returns the first future month below the target", () => {
		const model = buildModel(shortfallMonths);
		expect(firstShortfallMonth(model, shortfallMonths)).toBe("2026-07");
	});

	it("returns null when every future balance clears the target", () => {
		const model = buildModel(months);
		expect(firstShortfallMonth(model, months)).toBeNull();
	});
});

describe("solveRequiredSaving", () => {
	it("finds the minimal monthly cut that keeps every future balance ≥ 0", () => {
		const model = buildModel(shortfallMonths);
		const sol = solveRequiredSaving(model, shortfallMonths);
		expect(sol.achievable).toBe(true);
		expect(sol.monthlySaving).toBe(200);
		// Applying that saving actually clears the shortfall.
		const sim = applySimulationToModel(model, shortfallMonths, {
			fromMonth: "2026-07",
			monthlySaving: sol.monthlySaving,
		});
		expect(firstShortfallMonth(sim, shortfallMonths)).toBeNull();
	});

	it("needs no saving when the year already stays solvent", () => {
		const model = buildModel(months);
		expect(solveRequiredSaving(model, months)).toEqual({
			monthlySaving: 0,
			achievable: true,
		});
	});

	it("flags goals unreachable by cutting forecast alone", () => {
		const deep: ChartMonthView[] = [
			month({ month: "2026-06", closingBalance: "0" }),
			month({
				month: "2026-07",
				forecastOutflowsRemaining: "1000",
				projectedClosingBalance: "-3000",
				isFuture: 1,
			}),
		];
		const model = buildModel(deep);
		const sol = solveRequiredSaving(model, deep);
		expect(sol.achievable).toBe(false);
		expect(sol.monthlySaving).toBe(1000); // the maximal possible cut
	});
});
