import React, { useMemo, useRef, useState, useEffect } from "react";
import { formatMoneyNumber, numeric } from "../lib/format";
import { CountMoney } from "../components/ui";
import { useDnd } from "../lib/dnd";
import type { ChartMonthView, ForecastView, ChartMode } from "./types";
import {
	BASELINE,
	H,
	PAD,
	W,
	bh,
	buildModel,
	cashY,
	currentMonthKey,
	extendScale,
	firstShortfallMonth,
	innerH,
	innerW,
	scenarioBarDeltas,
	scenarioSliceExtents,
	solveRequiredSaving,
	type ChartModel,
	type ScenarioBarDelta,
} from "./chart/model";
import {
	CashHoverCard,
	ExpensesHoverCard,
	HoverCardShell,
	SegmentHoverCard,
	type ChartHoverDatum,
} from "./chart/ChartHoverCard";

export { buildModel } from "./chart/model";
import type {
	CategoryMonthSeries,
	ScenarioMonthItem,
	SubSlice,
} from "../lib/derivations";

// Qualitative palette for the per-category "Despesas" stacked bars. Kept
// distinct from the semantic income (cyan/green) and expense (rose) hues so a
// category colour never reads as "income" or "balance". Last entry stays a
// neutral grey for the rolled-up "outros" bucket.
const CAT_COLORS = [
	"#6d4aff", // purple
	"#2563eb", // blue
	"#d97706", // amber
	"#0891b2", // teal
	"#db2777", // pink
	"#7c3aed", // violet
	"#9a9aae", // neutral grey ("outros")
];
const catColor = (index: number, total: number): string =>
	index === total - 1 && total > CAT_COLORS.length
		? "#9a9aae"
		: CAT_COLORS[index % CAT_COLORS.length];

// ── Public component ───────────────────────────────────────────────────────

export const PlanningChart = ({
	months,
	forecastsByMonth,
	categorySeries,
	subSeries,
	selectedMonth,
	onSelectMonth,
	onDropForecast,
	scenarioBalances = null,
	scenarioMonths = null,
	scenarioItemsByMonth = null,
}: {
	months: ReadonlyArray<ChartMonthView>;
	forecastsByMonth: Map<string, ForecastView[]>;
	categorySeries: CategoryMonthSeries;
	/** month → parent → subcategory slices, for the per-segment hover. */
	subSeries: Map<string, Map<string, SubSlice[]>>;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
	/**
	 * Active planning scenario's projected saldo per month (aligned with
	 * `months`; null = no data for that month). Renders as a second dashed
	 * line with a shaded wedge vs the baseline (ADR-0037).
	 */
	scenarioBalances?: ReadonlyArray<number | null> | null;
	/** The active scenario's full chart projection, for the bar slices. */
	scenarioMonths?: ReadonlyArray<ChartMonthView> | null;
	/** month → the scenario's changes hitting it, for the hover card. */
	scenarioItemsByMonth?: ReadonlyMap<string, ScenarioMonthItem[]> | null;
}) => {
	// Extra bar flow the active scenario adds per future month (teal slices).
	const scenarioDeltas = useMemo(
		() =>
			scenarioMonths && scenarioMonths.length > 0
				? scenarioBarDeltas(months, scenarioMonths)
				: null,
		[months, scenarioMonths],
	);
	const model = useMemo(() => {
		const base = buildModel(months);
		const extras = [
			...(scenarioBalances ?? []),
			...(scenarioDeltas ? scenarioSliceExtents(months, scenarioDeltas) : []),
		];
		return extras.length > 0 ? extendScale(base, extras) : base;
	}, [months, scenarioBalances, scenarioDeltas]);
	// The chart is a single year-overview (cash) view — the old Cash/Expenses
	// toggle was removed; the per-category breakdown lives in the "categorias" tab.
	const mode: ChartMode = "caixa";

	const handleKeyDown = (e: React.KeyboardEvent) => {
		const n = months.length;
		if (!n) return;
		const cur = selectedMonth
			? months.findIndex((m) => m.month === selectedMonth)
			: -1;
		let next = cur;
		switch (e.key) {
			case "ArrowLeft":
				next = cur > 0 ? cur - 1 : 0;
				break;
			case "ArrowRight":
				next = cur < n - 1 ? cur + 1 : n - 1;
				break;
			case "Home":
				next = 0;
				break;
			case "End":
				next = n - 1;
				break;
			default:
				return;
		}
		e.preventDefault();
		if (next !== cur && next >= 0) onSelectMonth(months[next].month);
	};

	if (months.length === 0) return null;

	return (
		<div
			tabIndex={0}
			role="application"
			aria-label="gráfico de caixa — use ←→ para mudar de mês"
			onKeyDown={handleKeyDown}
			style={{ outline: "none" }}
		>
			<FullChart
				months={months}
				model={model}
				mode={mode}
				forecastsByMonth={forecastsByMonth}
				categorySeries={categorySeries}
				subSeries={subSeries}
				selectedMonth={selectedMonth}
				onSelectMonth={onSelectMonth}
				onDropForecast={onDropForecast}
				scenarioBalances={scenarioBalances}
				scenarioDeltas={scenarioDeltas}
				scenarioItemsByMonth={scenarioItemsByMonth}
			/>
		</div>
	);
};

// ── Full chart ─────────────────────────────────────────────────────────────

const FullChart = ({
	months,
	model,
	mode,
	forecastsByMonth,
	categorySeries,
	subSeries,
	selectedMonth,
	onSelectMonth,
	onDropForecast,
	scenarioBalances,
	scenarioDeltas,
	scenarioItemsByMonth,
}: {
	months: ReadonlyArray<ChartMonthView>;
	model: ChartModel;
	mode: ChartMode;
	forecastsByMonth: Map<string, ForecastView[]>;
	categorySeries: CategoryMonthSeries;
	subSeries: Map<string, Map<string, SubSlice[]>>;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
	scenarioBalances?: ReadonlyArray<number | null> | null;
	scenarioDeltas: ReadonlyMap<string, ScenarioBarDelta> | null;
	scenarioItemsByMonth?: ReadonlyMap<string, ScenarioMonthItem[]> | null;
}) => {
	const [hover, setHover] = useState<number | null>(null);
	// Per-segment hover in the expenses-bars mode: which (month, category) slice.
	const [hoverSeg, setHoverSeg] = useState<{ i: number; cat: string } | null>(
		null,
	);
	const n = months.length;
	const slot = innerW / n;
	const barW = Math.min(14, slot * 0.27);
	// Wide bar for expenses-only mode
	const expBarW = Math.min(24, slot * 0.55);

	// Bar X-centers: income (left of slot midpoint), expense (right)
	const incX = (i: number) => PAD.left + slot * i + slot * 0.3;
	const expX = (i: number) => PAD.left + slot * i + slot * 0.7;
	const midX = (i: number) => PAD.left + slot * i + slot * 0.5;

	const firstFuture = months.findIndex((m) => m.isFuture === 1);
	const realLine =
		firstFuture === -1
			? model.balances
			: model.balances.slice(0, firstFuture + 1);
	const fcLine = firstFuture === -1 ? [] : model.balances.slice(firstFuture);
	const zeroY = cashY(0, model.cashMin, model.cashSpan);

	// Magnitude of already-committed credit-card installments per month
	// (forecast kind === "installment"), to colour that slice of the forecast
	// expense bar distinctly from the soft budget envelope (N2).
	const committedMag = months.map((m) =>
		(forecastsByMonth.get(m.month) ?? []).reduce(
			(s, f) =>
				f.kind === "installment" && numeric(f.amount) < 0
					? s + Math.abs(numeric(f.amount))
					: s,
			0,
		),
	);

	const linePath = (vals: number[], offset = 0) =>
		vals
			.map(
				(b, k) =>
					`${k === 0 ? "M" : "L"} ${midX(offset + k)} ${cashY(b, model.cashMin, model.cashSpan)}`,
			)
			.join(" ");

	const isExpensesMode = mode.startsWith("despesas");

	// Structured per-month detail for the floating hover card: flows split into
	// realized + forecast, the baseline saldo, the category breakdown, and the
	// active scenario's saldo + changes for that month (ADR-0037).
	const hoverData = useMemo<ChartHoverDatum[]>(
		() =>
			months.map((m, i) => {
				const catMap = categorySeries.byMonth.get(m.month);
				const cats = catMap
					? Array.from(catMap.entries())
							.map(([name, mag]) => ({
								name,
								mag,
								color: catColor(
									categorySeries.categories.indexOf(name),
									categorySeries.categories.length,
								),
							}))
							.sort((a, b) => b.mag - a.mag)
					: [];
				return {
					label: m.label,
					isFuture: m.isFuture === 1,
					realIn: model.realIns[i],
					fcIn: model.fcIns[i],
					realOut: model.realOuts[i],
					fcOut: model.fcOuts[i],
					balance: model.balances[i],
					scenarioBalance: scenarioBalances?.[i] ?? null,
					scenarioItems: scenarioItemsByMonth?.get(m.month) ?? [],
					cats,
				};
			}),
		[months, model, categorySeries, scenarioBalances, scenarioItemsByMonth],
	);

	// Year totals
	const yearIn = model.realIns.reduce((s, v, i) => s + v + model.fcIns[i], 0);
	const yearOut = model.realOuts.reduce(
		(s, v, i) => s + v + model.fcOuts[i],
		0,
	);

	// Goal readout (ADR-0031): does the projected year stay solvent, and if not
	// what monthly cut would fix it?
	const goal = useMemo(() => {
		const shortfall = firstShortfallMonth(model, months);
		return {
			shortfall,
			label: shortfall
				? (months.find((m) => m.month === shortfall)?.label ?? shortfall)
				: null,
			solution: solveRequiredSaving(model, months),
		};
	}, [model, months]);

	return (
		<div>
			{/* Year totals strip */}
			<div
				className="mono"
				style={{
					display: "flex",
					gap: 20,
					fontSize: 11,
					paddingBottom: 8,
					flexWrap: "wrap",
				}}
			>
				<span style={{ color: "var(--muted)" }}>
					{new Date().getFullYear()}
				</span>
				{!isExpensesMode && (
					<span style={{ color: "var(--cyan)" }}>
						entradas <CountMoney value={yearIn} />
					</span>
				)}
				<span style={{ color: "var(--rose)" }}>
					saídas <CountMoney value={-yearOut} />
				</span>
				<span
					style={{
						color: yearIn - yearOut >= 0 ? "var(--green)" : "var(--rose)",
					}}
				>
					resultado {yearIn - yearOut >= 0 ? "+" : ""}
					<CountMoney value={yearIn - yearOut} />
				</span>
				{!isExpensesMode && goal.shortfall && (
					<span style={{ color: "var(--amber)" }}>
						⚠ fica no vermelho em {goal.label} ·{" "}
						{goal.solution.achievable ? (
							<>
								corte <CountMoney value={goal.solution.monthlySaving} />
								/mês para não ficar no vermelho
							</>
						) : (
							"inalcançável só cortando"
						)}
					</span>
				)}
			</div>

			{/* Category legend for the Despesas modes (D3) */}
			{isExpensesMode && categorySeries.categories.length > 0 && (
				<div
					className="mono"
					style={{
						display: "flex",
						gap: 12,
						flexWrap: "wrap",
						fontSize: 10,
						color: "var(--muted)",
						paddingBottom: 8,
					}}
				>
					{categorySeries.categories.map((cat, ci) => (
						<span
							key={cat}
							style={{ display: "flex", alignItems: "center", gap: 4 }}
						>
							<span
								style={{
									width: 8,
									height: 8,
									borderRadius: 2,
									background: catColor(ci, categorySeries.categories.length),
								}}
							/>
							{cat}
						</span>
					))}
				</div>
			)}

			<div className="contain-chart" style={{ position: "relative" }}>
				<svg
					viewBox={`0 0 ${W} ${H}`}
					width="100%"
					role="img"
					aria-label={
						mode === "caixa"
							? "gráfico mensal de caixa — barras de entradas e saídas, linha de saldo"
							: "gráfico mensal de despesas"
					}
					style={{ display: "block" }}
				>
					<defs>
						{/* Forecast expense — lighter solid (no hatch) */}
						<linearGradient id="fc-exp-bar" x1="0" x2="0" y1="0" y2="1">
							<stop offset="0%" stopColor="#fda4af" />
							<stop offset="100%" stopColor="#fecdd3" />
						</linearGradient>
						{/* Forecast expense area fill for line mode */}
						<linearGradient id="fc-exp-area" x1="0" x2="0" y1="0" y2="1">
							<stop offset="0%" stopColor="#e11d48" stopOpacity={0.08} />
							<stop offset="100%" stopColor="#e11d48" stopOpacity={0.02} />
						</linearGradient>
					</defs>

					{/* Baseline */}
					<line
						x1={PAD.left}
						x2={W - PAD.right}
						y1={mode === "caixa" ? zeroY : BASELINE}
						y2={mode === "caixa" ? zeroY : BASELINE}
						stroke="var(--border)"
						strokeWidth={mode === "caixa" ? 0.8 : 0.5}
					/>

					{/* ── Caixa mode: income + expense bars + balance line ── */}
					{mode === "caixa" &&
						months.map((m, i) => {
							const rInTop = cashY(
								model.realIns[i],
								model.cashMin,
								model.cashSpan,
							);
							const fInTop = cashY(
								model.realIns[i] + model.fcIns[i],
								model.cashMin,
								model.cashSpan,
							);
							const rOutBottom = cashY(
								-model.realOuts[i],
								model.cashMin,
								model.cashSpan,
							);
							const fOutBottom = cashY(
								-(model.realOuts[i] + model.fcOuts[i]),
								model.cashMin,
								model.cashSpan,
							);
							const rIn = zeroY - rInTop;
							const fIn = rInTop - fInTop;
							const rOut = rOutBottom - zeroY;
							const fOut = fOutBottom - rOutBottom;
							const isSel = m.month === selectedMonth;
							const isHov = hover === i && !isSel;
							const ix = incX(i) - barW / 2;
							const ox = expX(i) - barW / 2;
							const net =
								model.realIns[i] +
								model.fcIns[i] -
								model.realOuts[i] -
								model.fcOuts[i];
							const scenarioDelta = scenarioDeltas?.get(m.month) ?? null;

							return (
								<g key={m.label}>
									{/* Column background */}
									{(isSel || isHov) && (
										<rect
											x={PAD.left + slot * i}
											y={PAD.top}
											width={slot}
											height={innerH + 2}
											fill={
												isSel ? "rgba(13,148,136,0.08)" : "rgba(0,0,0,0.025)"
											}
										/>
									)}

									{/* Income bar — realized (solid cyan) */}
									{rIn > 0.5 && (
										<rect
											x={ix}
											y={rInTop}
											width={barW}
											height={rIn}
											rx={2}
											fill="var(--cyan)"
										/>
									)}
									{/* Income bar — forecast (lighter cyan) */}
									{fIn > 0.5 && (
										<rect
											x={ix}
											y={fInTop}
											width={barW}
											height={fIn}
											rx={2}
											fill="#99f6e4"
										/>
									)}

									{/* Expense bar — realized (solid rose) */}
									{rOut > 0.5 && (
										<rect
											x={ox}
											y={zeroY}
											width={barW}
											height={rOut}
											rx={2}
											fill="var(--rose)"
										/>
									)}
									{/* Expense bar — forecast. The already-committed credit-card
									    installment portion gets its own colour (N2); the soft
									    remainder is the budget envelope. */}
									{fOut > 0.5 &&
										(() => {
											const committed =
												model.fcOuts[i] > 0
													? Math.min(
															fOut,
															fOut * (committedMag[i] / model.fcOuts[i]),
														)
													: 0;
											const soft = fOut - committed;
											return (
												<>
													{soft > 0.5 && (
														<rect
															x={ox}
															y={rOutBottom}
															width={barW}
															height={soft}
															rx={2}
															fill="url(#fc-exp-bar)"
														/>
													)}
													{committed > 0.5 && (
														<rect
															x={ox}
															y={rOutBottom + soft}
															width={barW}
															height={committed}
															rx={2}
															fill="var(--amber)"
														/>
													)}
												</>
											);
										})()}

									{/* Scenario slices (ADR-0037): the extra flow the active
									    scenario adds this month, stacked on top of the bars. */}
									{scenarioDelta && (
										<ScenarioBarSlices
											delta={scenarioDelta}
											model={model}
											outBase={model.realOuts[i] + model.fcOuts[i]}
											inBase={model.realIns[i] + model.fcIns[i]}
											ix={ix}
											ox={ox}
											barW={barW}
										/>
									)}

									{/* Month label */}
									<text
										x={midX(i)}
										y={BASELINE + 14}
										textAnchor="middle"
										fontSize={9.5}
										fontFamily="var(--font-mono)"
										fill={isSel ? "var(--cyan)" : "var(--muted)"}
										fontWeight={isSel ? "600" : "400"}
									>
										{m.label}
									</text>

									{/* Net resultado indicator */}
									<text
										x={midX(i)}
										y={BASELINE + 30}
										textAnchor="middle"
										fontSize={8}
										fontFamily="var(--font-mono)"
										fill={net >= 0 ? "#15803d" : "#e11d48"}
									>
										{net >= 0 ? "+" : ""}
										{formatMoneyNumber(net)}
									</text>
								</g>
							);
						})}

					{/* ── Despesas-barras mode: expense bars only, wider ── */}
					{mode === "despesas-barras" &&
						months.map((m, i) => {
							const rOut = bh(model.realOuts[i], model.expMaxBar);
							const fOut = bh(model.fcOuts[i], model.expMaxBar);
							const isSel = m.month === selectedMonth;
							const isHov = hover === i && !isSel;
							const bx = midX(i) - expBarW / 2;

							return (
								<g key={m.label}>
									{(isSel || isHov) && (
										<rect
											x={PAD.left + slot * i}
											y={PAD.top}
											width={slot}
											height={innerH + 2}
											fill={
												isSel ? "rgba(225,29,72,0.06)" : "rgba(0,0,0,0.025)"
											}
										/>
									)}

									{/* Stacked expense segments by parent category (D3). Realized
									    distribution per month; future months (no realized txs)
									    fall back to the plain total bar. */}
									{(() => {
										const catMags = categorySeries.byMonth.get(m.month);
										if (!catMags || catMags.size === 0) {
											// No category data → plain total bar (e.g. future).
											const tot = rOut + fOut;
											return tot > 0.5 ? (
												<rect
													x={bx}
													y={BASELINE - tot}
													width={expBarW}
													height={tot}
													rx={3}
													fill="url(#fc-exp-bar)"
												/>
											) : null;
										}
										let yCursor = BASELINE;
										const nCats = categorySeries.categories.length;
										const catTotal = Array.from(
											catMags.values(),
										).reduce((a, b) => a + b, 0);
										return categorySeries.categories.map((cat, ci) => {
											const mag = catMags.get(cat) ?? 0;
											const h = bh(mag, model.expMaxBar);
											if (h < 0.5) return null;
											yCursor -= h;
											const pct =
												catTotal > 0
													? Math.round((mag / catTotal) * 100)
													: 0;
											const segHov =
												hoverSeg?.i === i && hoverSeg.cat === cat;
											return (
												<rect
													key={cat}
													x={bx}
													y={yCursor}
													width={expBarW}
													height={h}
													fill={catColor(ci, nCats)}
													opacity={isSel || isHov || segHov ? 1 : 0.85}
													stroke={segHov ? "var(--text)" : "none"}
													strokeWidth={segHov ? 1.5 : 0}
													style={{ cursor: "pointer" }}
													onMouseEnter={() => setHoverSeg({ i, cat })}
													onMouseLeave={() => setHoverSeg(null)}
													onClick={() => onSelectMonth(m.month)}
													aria-label={`${cat}: ${formatMoneyNumber(mag)} (${pct}%)`}
												/>
											);
										});
									})()}

									{/* Month label */}
									<text
										x={midX(i)}
										y={BASELINE + 14}
										textAnchor="middle"
										fontSize={9.5}
										fontFamily="var(--font-mono)"
										fill={isSel ? "var(--rose)" : "var(--muted)"}
										fontWeight={isSel ? "600" : "400"}
									>
										{m.label}
									</text>

									{/* Total expense label */}
									<text
										x={midX(i)}
										y={BASELINE + 30}
										textAnchor="middle"
										fontSize={8.5}
										fontFamily="var(--font-mono)"
										fill="var(--rose)"
									>
										{formatMoneyNumber(model.realOuts[i] + model.fcOuts[i])}
									</text>
								</g>
							);
						})}

					{/* Balance / resultado line — caixa mode only */}
					{mode === "caixa" && (
						<>
							{/* Goal line: saldo ≥ 0 — drawn only when the year dips red (ADR-0031) */}
							{goal.shortfall && (
								<>
									<line
										x1={PAD.left}
										x2={W - PAD.right}
										y1={zeroY}
										y2={zeroY}
										stroke="var(--amber)"
										strokeWidth={1}
										strokeDasharray="5 4"
										opacity={0.55}
									/>
									<text
										x={W - PAD.right}
										y={zeroY - 4}
										textAnchor="end"
										fontSize={9}
										fill="var(--amber)"
									>
										meta · saldo ≥ 0
									</text>
								</>
							)}
							<path
								d={linePath(realLine)}
								fill="none"
								stroke="var(--purple)"
								strokeWidth={1.5}
							/>
							{fcLine.length > 1 && (
								<path
									d={linePath(fcLine, Math.max(0, firstFuture))}
									fill="none"
									stroke="var(--purple)"
									strokeWidth={1.5}
									strokeDasharray="4 3"
									opacity={0.6}
								/>
							)}
							{/* Active planning scenario (ADR-0037): dashed cyan saldo
							    line + shaded wedge vs the baseline projection. */}
							{scenarioBalances && (
								<ScenarioSaldoOverlay
									months={months}
									model={model}
									scenarioBalances={scenarioBalances}
									midX={midX}
								/>
							)}
							{model.balances.map((b, i) => (
								<circle
									key={months[i].month}
									cx={midX(i)}
									cy={cashY(b, model.cashMin, model.cashSpan)}
									r={2.5}
									fill="var(--purple)"
									opacity={months[i].isFuture ? 0.5 : 1}
								/>
							))}
						</>
					)}
				</svg>

				{/* Interaction overlay (click/drag/hover) */}
				<ColumnOverlay
					months={months}
					selectedMonth={selectedMonth}
					onSelectMonth={onSelectMonth}
					onHover={setHover}
					onDropForecast={onDropForecast}
					interactive={!isExpensesMode}
				/>

				{/* Floating hover card: one per column, on top of the chart. */}
				{hover != null && hoverData[hover] && (
					<HoverCardShell index={hover} count={months.length}>
						{isExpensesMode ? (
							<ExpensesHoverCard d={hoverData[hover]} />
						) : (
							<CashHoverCard d={hoverData[hover]} />
						)}
					</HoverCardShell>
				)}

				{/* Per-segment card (expenses mode): one category + its top subs. */}
				{isExpensesMode && hoverSeg && (
					<HoverCardShell index={hoverSeg.i} count={months.length}>
						<SegmentHoverCard
							cat={hoverSeg.cat}
							color={catColor(
								categorySeries.categories.indexOf(hoverSeg.cat),
								categorySeries.categories.length,
							)}
							value={
								categorySeries.byMonth
									.get(months[hoverSeg.i].month)
									?.get(hoverSeg.cat) ?? 0
							}
							monthLabel={months[hoverSeg.i].label}
							subs={(
								subSeries.get(months[hoverSeg.i].month)?.get(hoverSeg.cat) ??
								[]
							).slice(0, 3)}
						/>
					</HoverCardShell>
				)}
			</div>
		</div>
	);
};

// ── Scenario bar slices (ADR-0037) ─────────────────────────────────────────

/**
 * Teal slices stacked on the bar ends: the extra expense/income the active
 * scenario adds to one future month vs. the baseline projection.
 */
const ScenarioBarSlices = ({
	delta,
	model,
	outBase,
	inBase,
	ix,
	ox,
	barW,
}: {
	delta: ScenarioBarDelta;
	model: ChartModel;
	/** Baseline expense magnitude (realized + forecast) for the month. */
	outBase: number;
	/** Baseline income magnitude (realized + forecast) for the month. */
	inBase: number;
	ix: number;
	ox: number;
	barW: number;
}) => {
	const y = (v: number) => cashY(v, model.cashMin, model.cashSpan);
	const outTop = y(-outBase);
	const outH = y(-(outBase + delta.extraOut)) - outTop;
	const inTop = y(inBase + delta.extraIn);
	const inH = y(inBase) - inTop;
	return (
		<>
			{outH > 0.5 && (
				<rect
					x={ox}
					y={outTop}
					width={barW}
					height={outH}
					rx={2}
					fill="var(--cyan)"
					opacity={0.5}
				/>
			)}
			{inH > 0.5 && (
				<rect
					x={ix}
					y={inTop}
					width={barW}
					height={inH}
					rx={2}
					fill="var(--cyan)"
					opacity={0.5}
				/>
			)}
		</>
	);
};

// ── Shared interaction overlay ─────────────────────────────────────────────

const ColumnOverlay = ({
	months,
	selectedMonth,
	onSelectMonth,
	onHover,
	onDropForecast,
	interactive,
}: {
	months: ReadonlyArray<ChartMonthView>;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onHover: (i: number | null) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
	/** When false (expenses-bars mode) the SVG segments own hover/click instead. */
	interactive: boolean;
}) => (
	<div
		style={{
			position: "absolute",
			inset: 0,
			display: "grid",
			gridTemplateColumns: `repeat(${months.length}, 1fr)`,
			pointerEvents: interactive ? "auto" : "none",
		}}
	>
		{months.map((m, i) => (
			<MonthColumn
				key={m.month}
				month={m.month}
				index={i}
				selected={m.month === selectedMonth}
				onSelect={() => onSelectMonth(m.month)}
				onHover={onHover}
				onDropForecast={onDropForecast}
			/>
		))}
	</div>
);

const MonthColumn = React.memo(({
	month,
	index,
	onSelect,
	onHover,
	onDropForecast,
}: {
	month: string;
	index: number;
	selected: boolean;
	onSelect: () => void;
	onHover: (i: number | null) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
}) => {
	const { dragging, hoverTargetId, registerTarget } = useDnd();
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		return registerTarget({
			id: `month:${month}`,
			getRect: () => ref.current?.getBoundingClientRect() ?? null,
			onDrop: (payload) => {
				if (payload.kind === "forecast" && month < currentMonthKey()) return;
				if (payload.forecastId) onDropForecast(payload.forecastId, month);
			},
		});
	}, [month, registerTarget, onDropForecast]);

	const isBlockedForecastTarget =
		dragging?.kind === "forecast" && month < currentMonthKey();
	const isDropTarget =
		dragging != null &&
		!isBlockedForecastTarget &&
		hoverTargetId === `month:${month}`;

	return (
		<div
			ref={ref}
			onClick={onSelect}
			onMouseEnter={() => onHover(index)}
			onMouseLeave={() => onHover(null)}
			style={{
				cursor: "pointer",
				borderRadius: "var(--radius-sm)",
				outline: isDropTarget ? "2px solid var(--purple)" : "none",
				outlineOffset: -2,
				background: isDropTarget ? "rgba(109,74,255,0.10)" : "transparent",
				transition: "outline-color 100ms",
			}}
			aria-label={`select ${month}`}
		/>
	);
});


// ── Scenario saldo overlay (ADR-0037) ──────────────────────────────────────

/**
 * Dashed cyan line for the active scenario's projected saldo, plus a shaded
 * wedge between it and the baseline projection on the months where both
 * exist — the visual "cash freed / committed" gap.
 */
const ScenarioSaldoOverlay = ({
	months,
	model,
	scenarioBalances,
	midX,
}: {
	months: ReadonlyArray<ChartMonthView>;
	model: ChartModel;
	scenarioBalances: ReadonlyArray<number | null>;
	midX: (i: number) => number;
}) => {
	const y = (v: number) => cashY(v, model.cashMin, model.cashSpan);
	const points = months
		.map((m, i) => ({ i, value: scenarioBalances[i], isFuture: m.isFuture === 1 }))
		.filter((p): p is { i: number; value: number; isFuture: boolean } => p.value != null);
	if (points.length === 0) return null;

	const line = points
		.map((p, k) => `${k === 0 ? "M" : "L"} ${midX(p.i)} ${y(p.value)}`)
		.join(" ");

	// Wedge between baseline and scenario saldo across future months only.
	const wedgePoints = points.filter((p) => p.isFuture);
	const wedge =
		wedgePoints.length > 1
			? [
					...wedgePoints.map((p) => `${midX(p.i)},${y(model.balances[p.i])}`),
					...[...wedgePoints]
						.reverse()
						.map((p) => `${midX(p.i)},${y(p.value)}`),
				].join(" ")
			: null;

	return (
		<>
			{wedge && <polygon points={wedge} fill="var(--cyan)" opacity={0.12} />}
			<path
				d={line}
				fill="none"
				stroke="var(--cyan)"
				strokeWidth={1.5}
				strokeDasharray="6 3"
				opacity={0.85}
			/>
			{points.map((p) => (
				<circle
					key={months[p.i].month}
					cx={midX(p.i)}
					cy={y(p.value)}
					r={2}
					fill="var(--cyan)"
					opacity={0.85}
				/>
			))}
		</>
	);
};
