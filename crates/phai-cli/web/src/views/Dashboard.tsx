import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useEffect, useMemo, useState } from "react";
import { events, tables } from "../livestore/schema";
import {
	useChartSeed,
	useForecastsSeed,
	useScenarioDetailSeed,
	useScenariosSeed,
	useTransactionsSeed,
} from "../bridge/sync";

import {
	buildOverlayMap,
	expensesByMonthCategory,
	scenarioChangesByMonth,
	subExpensesByMonthCategory,
	type ScenarioChangeLike,
	type TxView as TxViewD,
} from "../lib/derivations";
import {
	ChartSkeleton,
	ErrorNote,
	HeroSkeleton,
	ListSkeleton,
} from "../components/ui";
import { PlanningChart } from "./PlanningChart";
import { MonthDetail } from "./MonthDetail";
import { CardsPanel } from "./CardsPanel";
import { CashDecisionPanel, type CashWhen } from "./cash/CashDecisionPanel";
import { AccountBalances } from "./cash/AccountBalances";
import { PlanilhaView } from "./planilha/PlanilhaView";
import { numeric } from "../lib/format";
import type { ChartMonthView, ForecastView } from "./types";

// Seeding window: the 12 months of the current calendar year.
export const planningYearWindow = (date: Date) => {
	const monthIndex = date.getMonth();
	return {
		chartMonthsBack: monthIndex + 1, // chart expects a count including current
		transactionMonthsBack: monthIndex, // transaction API expects an offset
		monthsAhead: 11 - monthIndex,
	};
};

const YEAR_WINDOW = planningYearWindow(new Date());

const chart$ = queryDb(tables.chartMonths.orderBy("ordinal", "asc"));
const forecasts$ = queryDb(tables.forecasts.orderBy("dueDate", "asc"));
const forecastOverlay$ = queryDb(tables.forecastOverlay);
const txAll$ = queryDb(tables.transactions);
const reviewOverlay$ = queryDb(tables.reviewOverlay);
const scenarioChart$ = queryDb(
	tables.scenarioChartMonths.orderBy("ordinal", "asc"),
);
const scenarioChanges$ = queryDb(tables.scenarioChanges);
const pendingWrites$ = queryDb(tables.pendingWrites);

const monthOf = (date: string | null): string | null =>
	date ? date.slice(0, 7) : null;

const currentMonthKey = () => {
	const d = new Date();
	return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

const daysInMonth = (month: string): number => {
	const [year, monthNum] = month.split("-").map(Number);
	if (!year || !monthNum) return 1;
	return new Date(year, monthNum, 0).getDate();
};

const dueDateInTargetMonth = (dueDate: string | null, targetMonth: string) => {
	const currentDay = dueDate ? Number(dueDate.slice(8, 10)) : 1;
	const day = Math.min(
		Number.isFinite(currentDay) && currentDay > 0 ? currentDay : 1,
		daysInMonth(targetMonth),
	);
	return `${targetMonth}-${String(day).padStart(2, "0")}`;
};

/**
 * Dashboard — the unified phai view. The cash-evolution chart sits at the top
 * (position:sticky, compresses on scroll to a mini nav strip). Below it, the
 * selected month's transactions are shown grouped by category, with filters,
 * editing, and a modal for bulk/raw operations. Everything runs on LiveStore
 * (client-only): filtering, sums, grouping are all computed locally with zero
 * network round-trips.
 */
export const Dashboard = () => {
	const { store } = useStore();
	const [ui, setUi] = useClientDocument(tables.ui);
	const chartRows = useQuery(chart$) as ReadonlyArray<ChartMonthView>;
	const forecastsRaw = useQuery(forecasts$);
	const fOverlay = useQuery(forecastOverlay$);
	const txRows = useQuery(txAll$) as ReadonlyArray<TxViewD>;
	const rOverlay = useQuery(reviewOverlay$);

	// Per-month expense distribution by parent category for the chart's
	// "Despesas" modes (stacked bars / multi-line) — derived client-side from
	// the already-seeded transactions (D3).
	const categorySeries = useMemo(
		() => expensesByMonthCategory(txRows, buildOverlayMap(rOverlay as never)),
		[txRows, rOverlay],
	);
	// Subcategory detail per month/parent for the chart's per-segment hover.
	const subSeries = useMemo(
		() =>
			subExpensesByMonthCategory(txRows, buildOverlayMap(rOverlay as never)),
		[txRows, rOverlay],
	);

	// Seed: current year
	const chartSeed = useChartSeed(
		YEAR_WINDOW.chartMonthsBack,
		YEAR_WINDOW.monthsAhead,
	);
	const forecastSeed = useForecastsSeed(null);
	const txSeed = useTransactionsSeed(
		YEAR_WINDOW.transactionMonthsBack,
		YEAR_WINDOW.monthsAhead,
	);

	// ── Planning scenarios (ADR-0037) ────────────────────────────────────────
	const activeScenarioId = ui.activeScenarioId ?? null;
	useScenariosSeed();
	// Re-fetch the scenario projection once this session's scenario writes
	// finish flushing (pending scenario rows transition to zero).
	const pendingRows = useQuery(pendingWrites$) as ReadonlyArray<{
		type: string;
	}>;
	const pendingScenarioWrites = useMemo(
		() => pendingRows.filter((r) => r.type.startsWith("scenario")).length,
		[pendingRows],
	);
	const [scenarioSeedNonce, setScenarioSeedNonce] = useState(0);
	const [prevPending, setPrevPending] = useState(0);
	useEffect(() => {
		if (prevPending > 0 && pendingScenarioWrites === 0) {
			setScenarioSeedNonce((n) => n + 1);
		}
		setPrevPending(pendingScenarioWrites);
	}, [pendingScenarioWrites, prevPending]);
	useScenarioDetailSeed(
		activeScenarioId,
		YEAR_WINDOW.chartMonthsBack,
		YEAR_WINDOW.monthsAhead,
		scenarioSeedNonce,
	);
	const scenarioChartRows = useQuery(scenarioChart$) as ReadonlyArray<
		ChartMonthView & { scenarioId: string }
	>;
	const scenarioChangeRows = useQuery(scenarioChanges$) as ReadonlyArray<
		ScenarioChangeLike & { scenarioId: string; orphaned: number }
	>;

	// Apply forecast re-dating overlay
	const overlayById = useMemo(
		() => new Map(fOverlay.map((o) => [o.forecastId, o.dueDate])),
		[fOverlay],
	);
	const forecasts: ForecastView[] = useMemo(
		() =>
			forecastsRaw.map((f) => {
				const dueDate = overlayById.has(f.forecastId)
					? (overlayById.get(f.forecastId) ?? f.dueDate)
					: f.dueDate;
				return {
					...f,
					dueDate,
					month: monthOf(dueDate),
					metadataJson:
						f.metadataJson && typeof f.metadataJson === "object"
							? (f.metadataJson as Record<string, unknown>)
							: {},
				};
			}),
		[forecastsRaw, overlayById],
	);

	const forecastsByMonth = useMemo(() => {
		const map = new Map<string, ForecastView[]>();
		for (const f of forecasts) {
			if (!f.month) continue;
			const list = map.get(f.month) ?? [];
			list.push(f);
			map.set(f.month, list);
		}
		return map;
	}, [forecasts]);

	// Selected month — default to current month
	const months = chartRows;
	const currentMonth = currentMonthKey();
	const selected = ui.selectedMonth ?? currentMonth;

	// Global keyboard shortcuts:
	//   Alt+←/→  → previous / next month
	//   1 / 2 / 3 → planilha / categorias / cartões (when not typing in a field)
	useEffect(() => {
		const isTyping = (el: EventTarget | null) => {
			const t = el as HTMLElement | null;
			const tag = t?.tagName;
			return (
				tag === "INPUT" ||
				tag === "TEXTAREA" ||
				tag === "SELECT" ||
				t?.isContentEditable === true
			);
		};
		const onKeyDown = (e: KeyboardEvent) => {
			if (isTyping(e.target)) return;
			// Month navigation.
			if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
				if (months.length === 0) return;
				e.preventDefault();
				const idx = months.findIndex((m) => m.month === selected);
				if (idx === -1) return;
				const next = e.key === "ArrowLeft" ? months[idx - 1] : months[idx + 1];
				if (next) setUi({ selectedMonth: next.month });
				return;
			}
			// Mode switch (no modifiers).
			if (e.metaKey || e.ctrlKey || e.altKey) return;
			const mode = { "1": "planilha", "2": "categorias", "3": "cartoes" }[e.key];
			if (mode) {
				e.preventDefault();
				setUi({ detailMode: mode });
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [months, selected, setUi]);

	// The cash-decision hero shows the selected month (falling back to the
	// current month). `when` drives the headline label/value: realized closing
	// balance for past/current, projected for future.
	const heroRow =
		months.find((m) => m.month === selected) ??
		months.find((m) => m.month === currentMonth) ??
		null;
	const heroWhen: CashWhen = heroRow
		? heroRow.isFuture
			? "future"
			: heroRow.month === currentMonth
				? "current"
				: "past"
		: "current";

	// Drag-drop: move forecast to another month
	const moveForecast = (forecastId: string, targetMonth: string) => {
		const f = forecasts.find((x) => x.forecastId === forecastId);
		if (!f || !f.draggable) return;
		if (targetMonth < currentMonth) return;
		const dueDate = dueDateInTargetMonth(f.dueDate, targetMonth);
		if (dueDate === f.dueDate) return;
		store.commit(
			events.forecastMoved({
				writeId: crypto.randomUUID(),
				forecastId,
				dueDate,
				movedAt: Date.now(),
			}),
		);
	};

	// The active scenario's chart projection rows (null = baseline).
	const scenarioMonths = useMemo(
		() =>
			activeScenarioId
				? scenarioChartRows.filter((r) => r.scenarioId === activeScenarioId)
				: null,
		[activeScenarioId, scenarioChartRows],
	);

	// Scenario saldo per chart month (aligned with `months`; null = no data).
	const scenarioBalances = useMemo(() => {
		if (!scenarioMonths || scenarioMonths.length === 0) return null;
		const byMonth = new Map(
			scenarioMonths.map((r) => [r.month, Number(r.projectedClosingBalance)]),
		);
		return months.map((m) => {
			const v = byMonth.get(m.month);
			return v != null && Number.isFinite(v) ? v : null;
		});
	}, [scenarioMonths, months]);

	// The active scenario's changes bucketed per month, for the chart's rich
	// hover card (label + signed delta per item). Orphaned deltas (target
	// realized/removed) are excluded — they no longer move the projection.
	const scenarioItemsByMonth = useMemo(() => {
		if (!activeScenarioId) return null;
		const changes = scenarioChangeRows.filter(
			(c) => c.scenarioId === activeScenarioId && c.orphaned !== 1,
		);
		if (changes.length === 0) return null;
		return scenarioChangesByMonth(changes, forecasts);
	}, [activeScenarioId, scenarioChangeRows, forecasts]);

	// Selected-month projected-saldo delta of the active scenario vs. the
	// baseline (null when the scenario projection isn't seeded for it) — the
	// number the sheet's scenario strip shows next to the change count.
	const scenarioDelta = useMemo(() => {
		if (!scenarioBalances) return null;
		const idx = months.findIndex((m) => m.month === selected);
		if (idx === -1) return null;
		const scenario = scenarioBalances[idx];
		if (scenario == null) return null;
		const m = months[idx];
		const baseline = numeric(
			m.isFuture ? m.projectedClosingBalance : m.closingBalance,
		);
		return scenario - baseline;
	}, [scenarioBalances, months, selected]);

	const error = chartSeed.error ?? forecastSeed.error ?? txSeed.error;
	const loading = chartSeed.loading && months.length === 0;

	return (
		<div>
			{/* ── Cash-decision hero (normal flow — scrolls away naturally) ── */}
			<div
				style={{
					maxWidth: "var(--container)",
					margin: "0 auto",
					padding: "16px clamp(24px,3vw,32px) 12px",
				}}
			>
				{error && !loading && <ErrorNote error={error} />}
				{loading ? (
					<HeroSkeleton />
				) : heroRow ? (
					<div className="fade-in-soft">
						<CashDecisionPanel
							row={heroRow}
							when={heroWhen}
							compact={false}
							onStepMonth={(dir) => {
								const idx = months.findIndex((m) => m.month === selected);
								if (idx === -1) return;
								const next = months[idx + dir];
								if (next) setUi({ selectedMonth: next.month });
							}}
							canStepPrev={
								months.findIndex((m) => m.month === selected) > 0
							}
							canStepNext={(() => {
								const idx = months.findIndex((m) => m.month === selected);
								return idx >= 0 && idx < months.length - 1;
							})()}
						/>
						<AccountBalances />
					</div>
				) : null}
			</div>

			{/* ── Cash chart (subordinate to the hero; scrolls normally) ── */}
			<div
				style={{
					maxWidth: "var(--container)",
					margin: "0 auto",
					padding: "12px clamp(24px,3vw,32px) 0",
				}}
			>
				{loading ? (
					<ChartSkeleton />
				) : months.length === 0 ? null : (
					<div className="fade-in-soft">
					<PlanningChart
						months={months}
						forecastsByMonth={forecastsByMonth}
						categorySeries={categorySeries}
						subSeries={subSeries}
						selectedMonth={selected}
						onSelectMonth={(m) => setUi({ selectedMonth: m })}
						onDropForecast={moveForecast}
						scenarioBalances={scenarioBalances}
						scenarioMonths={scenarioMonths}
						scenarioItemsByMonth={scenarioItemsByMonth}
					/>
					</div>
				)}
			</div>

			{/* ── Month detail (sheet | categories | cards) ── */}
			<div
				style={{
					maxWidth: "var(--container)",
					margin: "0 auto",
					padding: "0 clamp(24px,3vw,32px)",
				}}
			>
				{loading ? (
					<div style={{ marginTop: 16 }}>
						<ListSkeleton />
					</div>
				) : months.length === 0 ? (
					<div
						className="mono"
						style={{
							color: "var(--muted)",
							fontSize: 13,
							paddingTop: 32,
							textAlign: "center",
						}}
					>
						Sem dados de caixa.{" "}
						<button
							onClick={() => {
								chartSeed.reload();
								forecastSeed.reload();
							}}
							style={{
								background: "transparent",
								border: "none",
								color: "var(--purple)",
								cursor: "pointer",
								fontFamily: "var(--font-mono)",
								fontSize: 13,
							}}
						>
							↻ retry
						</button>
					</div>
				) : (ui.detailMode || "planilha") === "planilha" ? (
					<PlanilhaView
						month={selected}
						activeScenarioId={activeScenarioId}
						scenarioDelta={scenarioDelta}
						onActivateScenario={(id) => setUi({ activeScenarioId: id })}
						onScenarioMutated={() => setScenarioSeedNonce((n) => n + 1)}
					/>
				) : (ui.detailMode || "planilha") === "cartoes" ? (
					<div style={{ marginTop: 12 }}>
						<CardsPanel
							month={selected}
							onViewCardTx={(accountId) =>
								setUi({ accountFilter: accountId, detailMode: "planilha" })
							}
						/>
					</div>
				) : (
					<MonthDetail
						month={selected}
						chart={months.find((m) => m.month === selected) ?? null}
						forecasts={forecastsByMonth.get(selected) ?? []}
					/>
				)}
			</div>
		</div>
	);
};
