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

export function buildModel(months: ReadonlyArray<ChartMonthView>): ChartModel {
	const realIns = months.map((m) => Math.max(0, numeric(m.inflows)));
	const fcIns = months.map((m) =>
		Math.max(0, numeric(m.forecastInflowsRemaining)),
	);
	const realOuts = months.map((m) => Math.abs(numeric(m.outflows)));
	const fcOuts = months.map((m) =>
		Math.abs(numeric(m.forecastOutflowsRemaining)),
	);
	const balances = months.map((m) =>
		m.isFuture ? numeric(m.projectedClosingBalance) : numeric(m.closingBalance),
	);
	const maxBar = Math.max(
		1,
		...months.map((_, i) => realIns[i] + fcIns[i]),
		...months.map((_, i) => realOuts[i] + fcOuts[i]),
	);
	const expMaxBar = Math.max(
		1,
		...months.map((_, i) => realOuts[i] + fcOuts[i]),
	);
	const minBal = Math.min(0, ...balances);
	const maxBal = Math.max(1, ...balances);
	const balSpan = maxBal - minBal || 1;
	const cashMax = Math.max(
		1,
		...balances,
		...months.map((_, i) => realIns[i] + fcIns[i]),
	);
	const cashMin = Math.min(
		0,
		...balances,
		...months.map((_, i) => -(realOuts[i] + fcOuts[i])),
	);
	const cashSpan = cashMax - cashMin || 1;
	return {
		realIns,
		fcIns,
		realOuts,
		fcOuts,
		balances,
		maxBar,
		expMaxBar,
		minBal,
		balSpan,
		cashMin,
		cashSpan,
	};
}

// Convert bar magnitude → SVG height
export const bh = (v: number, maxBar: number) => (v / maxBar) * BAR_MAX;
export const cashY = (v: number, min: number, span: number) =>
	PAD.top + (1 - (v - min) / span) * innerH;

