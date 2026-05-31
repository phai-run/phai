import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { events, tables } from "../livestore/schema";
import {
	useChartSeed,
	useForecastsSeed,
	useTransactionsSeed,
} from "../bridge/sync";

import { ErrorNote, LoadingNote } from "../components/ui";
import { PlanningChart } from "./PlanningChart";
import { MonthDetail } from "./MonthDetail";
import type { ChartMonthView, ForecastView } from "./types";

// Seeding window: the 12 months of the current calendar year.
const _now = new Date();
const MONTHS_BACK = _now.getMonth(); // months before current → Jan
const MONTHS_AHEAD = 11 - _now.getMonth(); // months after current → Dec

const chart$ = queryDb(tables.chartMonths.orderBy("ordinal", "asc"));
const forecasts$ = queryDb(tables.forecasts.orderBy("dueDate", "asc"));
const forecastOverlay$ = queryDb(tables.forecastOverlay);

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

	// Seed: current year
	const chartSeed = useChartSeed(MONTHS_BACK, MONTHS_AHEAD);
	const forecastSeed = useForecastsSeed(null);
	useTransactionsSeed(MONTHS_BACK, MONTHS_AHEAD);

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
				return { ...f, dueDate, month: monthOf(dueDate) };
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

	// Sticky / compact detection: IntersectionObserver on a sentinel div
	// placed right after the full-height chart area. When sentinel leaves
	// viewport (user scrolled past it), the chart goes compact.
	const sentinelRef = useRef<HTMLDivElement>(null);
	const [isCompact, setIsCompact] = useState(false);

	useEffect(() => {
		const sentinel = sentinelRef.current;
		if (!sentinel) return;
		const obs = new IntersectionObserver(
			([entry]) => setIsCompact(!entry.isIntersecting),
			{ threshold: 0 },
		);
		obs.observe(sentinel);
		return () => obs.disconnect();
	}, []);

	const error = chartSeed.error ?? forecastSeed.error;
	const loading = chartSeed.loading && months.length === 0;

	return (
		<div>
			{/* ── Sticky chart header ── */}
			<div
				style={{
					position: "sticky",
					top: 0,
					zIndex: 20,
					background: "var(--bg)",
					borderBottom: isCompact ? "1px solid var(--border)" : "none",
					boxShadow: isCompact ? "0 2px 12px rgba(21,19,31,0.06)" : "none",
					transition: "box-shadow 200ms, border-color 200ms",
				}}
			>
				<div
					style={{
						maxWidth: "var(--container)",
						margin: "0 auto",
						padding: isCompact
							? "6px clamp(24px,3vw,32px)"
							: "16px clamp(24px,3vw,32px) 0",
						transition: "padding 200ms",
					}}
				>
					{error && !loading && <ErrorNote error={error} />}
					{loading ? (
						<LoadingNote message="carregando caixa…" />
					) : months.length === 0 ? null : (
						<PlanningChart
							months={months}
							forecastsByMonth={forecastsByMonth}
							selectedMonth={selected}
							onSelectMonth={(m) => setUi({ selectedMonth: m })}
							onDropForecast={moveForecast}
							compact={isCompact}
						/>
					)}
				</div>
			</div>

			{/* Sentinel: when this scrolls offscreen, chart → compact */}
			<div ref={sentinelRef} style={{ height: 0 }} />

			{/* ── Month detail ── */}
			<div
				style={{
					maxWidth: "var(--container)",
					margin: "0 auto",
					padding: "0 clamp(24px,3vw,32px)",
				}}
			>
				{months.length === 0 && !loading ? (
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
							↻ tentar novamente
						</button>
					</div>
				) : (
					<MonthDetail
						month={selected}
						chart={months.find((m) => m.month === selected) ?? null}
						forecasts={forecastsByMonth.get(selected) ?? []}
						onForecastAdded={() => forecastSeed.reload()}
						months={months}
						onMoveForecast={moveForecast}
					/>
				)}
			</div>
		</div>
	);
};
