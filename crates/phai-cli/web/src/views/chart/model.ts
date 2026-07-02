import { numeric } from "../../lib/format";
import type { ChartMonthView } from "../types";

// ── SVG dimensions for the full chart ─────────────────────────────────────
export const W = 960;
export const H = 290;
export const PAD = { top: 12, right: 8, bottom: 68, left: 8 };
export const innerW = W - PAD.left - PAD.right; // 944
export const innerH = H - PAD.top - PAD.bottom; // 210
// Y where bars are rooted (baseline)
export const BASELINE = PAD.top + innerH; // 222
// Max bar height (bars use 75% of innerH)
export const BAR_MAX = innerH * 0.75; // ~157.5

export const currentMonthKey = () => {
	const d = new Date();
	return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

// ── Model ──────────────────────────────────────────────────────────────────

export interface ChartModel {
	realIns: number[];
	fcIns: number[];
	realOuts: number[];
	fcOuts: number[];
	balances: number[];
	maxBar: number;
	expMaxBar: number;
	minBal: number;
	balSpan: number;
	cashMin: number;
	cashSpan: number;
}

/** Series → axis/scale fields, shared by buildModel and the goal overlay. */
const withScale = (series: {
	realIns: number[];
	fcIns: number[];
	realOuts: number[];
	fcOuts: number[];
	balances: number[];
}): ChartModel => {
	const { realIns, fcIns, realOuts, fcOuts, balances } = series;
	const maxBar = Math.max(
		1,
		...realIns.map((v, i) => v + fcIns[i]),
		...realOuts.map((v, i) => v + fcOuts[i]),
	);
	const expMaxBar = Math.max(1, ...realOuts.map((v, i) => v + fcOuts[i]));
	const minBal = Math.min(0, ...balances);
	const maxBal = Math.max(1, ...balances);
	const balSpan = maxBal - minBal || 1;
	const cashMax = Math.max(1, ...balances, ...realIns.map((v, i) => v + fcIns[i]));
	const cashMin = Math.min(
		0,
		...balances,
		...realOuts.map((v, i) => -(v + fcOuts[i])),
	);
	const cashSpan = cashMax - cashMin || 1;
	return { ...series, maxBar, expMaxBar, minBal, balSpan, cashMin, cashSpan };
};

export function buildModel(months: ReadonlyArray<ChartMonthView>): ChartModel {
	return withScale({
		realIns: months.map((m) => Math.max(0, numeric(m.inflows))),
		fcIns: months.map((m) => Math.max(0, numeric(m.forecastInflowsRemaining))),
		realOuts: months.map((m) => Math.abs(numeric(m.outflows))),
		fcOuts: months.map((m) => Math.abs(numeric(m.forecastOutflowsRemaining))),
		balances: months.map((m) =>
			m.isFuture
				? numeric(m.projectedClosingBalance)
				: numeric(m.closingBalance),
		),
	});
}

/** A live war-plan goal simulation projected onto the cash chart. */
export interface ChartSimulation {
	/** First month ("YYYY-MM") the monthly saving applies to. */
	fromMonth: string;
	/** Saving vs. the baseline projection (negative = goals above it). */
	monthlySaving: number;
}

/**
 * Overlay a war-plan goal simulation on a chart model: from `fromMonth` on,
 * each month's forecast outflow shrinks by the monthly saving (a positive
 * saving never cuts below zero — realized spend is untouchable), and every
 * FUTURE month's balance shifts by the savings accumulated up to it. Realized
 * balances (past + current month) stay as observed. Scale is recomputed so
 * the overlay renders like any other model.
 */
export function applySimulationToModel(
	model: ChartModel,
	months: ReadonlyArray<ChartMonthView>,
	sim: ChartSimulation,
): ChartModel {
	const fcOuts = [...model.fcOuts];
	const balances = [...model.balances];
	let accumulated = 0;
	for (let i = 0; i < months.length; i++) {
		if (months[i].month < sim.fromMonth) continue;
		const applied =
			sim.monthlySaving > 0
				? Math.min(sim.monthlySaving, fcOuts[i])
				: sim.monthlySaving;
		fcOuts[i] -= applied;
		accumulated += applied;
		if (months[i].isFuture) balances[i] += accumulated;
	}
	return withScale({
		realIns: model.realIns,
		fcIns: model.fcIns,
		realOuts: model.realOuts,
		fcOuts,
		balances,
	});
}

/**
 * Widen a model's cash scale so extra series (e.g. the scenario saldo line,
 * ADR-0037) fit inside the viewport without re-deriving the bars.
 */
export function extendScale(
	model: ChartModel,
	extra: ReadonlyArray<number | null>,
): ChartModel {
	const values = extra.filter((v): v is number => v != null);
	if (values.length === 0) return model;
	const cashMax = model.cashMin + model.cashSpan;
	const nextMin = Math.min(model.cashMin, ...values);
	const nextMax = Math.max(cashMax, ...values);
	return { ...model, cashMin: nextMin, cashSpan: nextMax - nextMin || 1 };
}

// ── Goal solving (ADR-0031) ────────────────────────────────────────────────

/** The first FUTURE month whose projected balance dips below `target`. */
export function firstShortfallMonth(
	model: ChartModel,
	months: ReadonlyArray<ChartMonthView>,
	target = 0,
): string | null {
	for (let i = 0; i < months.length; i++) {
		if (months[i].isFuture && model.balances[i] < target) return months[i].month;
	}
	return null;
}

/** Result of the inverse solver: the cut needed to reach a balance goal. */
export interface GoalSolution {
	/** Minimal constant monthly saving that keeps every future balance ≥ target. */
	monthlySaving: number;
	/** False when even cutting all forecast outflows cannot reach the goal. */
	achievable: boolean;
}

/**
 * Inverse of {@link applySimulationToModel}: find the smallest constant monthly
 * saving (a forecast-outflow cut from `fromMonth` on) that keeps every future
 * month's balance at or above `target`. Binary-searches over the real clamped
 * simulation, so the answer respects "you can't cut more than you spend".
 * Returns `achievable: false` (with the maximal cut) when the goal is out of
 * reach by cutting forecast alone.
 */
export function solveRequiredSaving(
	model: ChartModel,
	months: ReadonlyArray<ChartMonthView>,
	opts: { target?: number; fromMonth?: string } = {},
): GoalSolution {
	const target = opts.target ?? 0;
	const fromMonth =
		opts.fromMonth ?? months.find((m) => m.isFuture)?.month ?? null;
	if (fromMonth == null) return { monthlySaving: 0, achievable: true };

	const futureIdx = months
		.map((m, i) => (m.isFuture ? i : -1))
		.filter((i) => i >= 0);
	const meets = (s: number): boolean => {
		const sim = applySimulationToModel(model, months, {
			fromMonth,
			monthlySaving: s,
		});
		return futureIdx.every((i) => sim.balances[i] >= target - 1e-6);
	};

	if (meets(0)) return { monthlySaving: 0, achievable: true };
	const hi = Math.max(0, ...model.fcOuts);
	if (!meets(hi)) return { monthlySaving: hi, achievable: false };

	let lo = 0;
	let high = hi;
	for (let k = 0; k < 40; k++) {
		const mid = (lo + high) / 2;
		if (meets(mid)) high = mid;
		else lo = mid;
	}
	return { monthlySaving: Math.ceil(high), achievable: true };
}

// Convert bar magnitude → SVG height
export const bh = (v: number, maxBar: number) => (v / maxBar) * BAR_MAX;
export const cashY = (v: number, min: number, span: number) =>
	PAD.top + (1 - (v - min) / span) * innerH;

