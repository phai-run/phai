import { useMemo, useRef, useState, useEffect } from "react";
import { formatMoneyNumber, numeric } from "../lib/format";
import { CountMoney } from "../components/ui";
import { useDnd } from "../lib/dnd";
import type { ChartMonthView, ForecastView, ChartMode } from "./types";
import type { CategoryMonthSeries } from "../lib/derivations";

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

// ── SVG dimensions for the full chart ─────────────────────────────────────
const W = 960;
const H = 290;
const PAD = { top: 12, right: 8, bottom: 68, left: 8 };
const innerW = W - PAD.left - PAD.right; // 944
const innerH = H - PAD.top - PAD.bottom; // 210
// Y where bars are rooted (baseline)
const BASELINE = PAD.top + innerH; // 222
// Max bar height (bars use 75% of innerH)
const BAR_MAX = innerH * 0.75; // ~157.5

const currentMonthKey = () => {
	const d = new Date();
	return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

// ── Model ──────────────────────────────────────────────────────────────────

interface ChartModel {
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
const bh = (v: number, maxBar: number) => (v / maxBar) * BAR_MAX;
const cashY = (v: number, min: number, span: number) =>
	PAD.top + (1 - (v - min) / span) * innerH;

// ── Public component ───────────────────────────────────────────────────────

export const PlanningChart = ({
	months,
	forecastsByMonth,
	categorySeries,
	selectedMonth,
	onSelectMonth,
	onDropForecast,
}: {
	months: ReadonlyArray<ChartMonthView>;
	forecastsByMonth: Map<string, ForecastView[]>;
	categorySeries: CategoryMonthSeries;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
}) => {
	const model = useMemo(() => buildModel(months), [months]);
	const [mode, setMode] = useState<ChartMode>("caixa");

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
			aria-label="gráfico de caixa — use ←→ para navegar entre meses"
			onKeyDown={handleKeyDown}
			style={{ outline: "none" }}
		>
			<ModeSelector mode={mode} onChange={setMode} />

			<FullChart
				months={months}
				model={model}
				mode={mode}
				forecastsByMonth={forecastsByMonth}
				categorySeries={categorySeries}
				selectedMonth={selectedMonth}
				onSelectMonth={onSelectMonth}
				onDropForecast={onDropForecast}
			/>
		</div>
	);
};

// ── Mode selector ──────────────────────────────────────────────────────────

const ModeSelector = ({
	mode,
	onChange,
}: {
	mode: ChartMode;
	onChange: (m: ChartMode) => void;
}) => (
	<div
		role="radiogroup"
		aria-label="Modo do gráfico"
		style={{
			display: "inline-flex",
			background: "var(--surface)",
			borderRadius: "var(--radius-full)",
			padding: 3,
			marginBottom: 14,
			border: "1px solid var(--border)",
		}}
	>
		<ModeChip
			label="Caixa"
			active={mode === "caixa"}
			onClick={() => onChange("caixa")}
		/>
		<ModeChip
			label="Despesas"
			active={mode === "despesas-barras"}
			onClick={() => onChange("despesas-barras")}
		/>
	</div>
);

const ModeChip = ({
	label,
	active,
	onClick,
}: {
	label: string;
	active: boolean;
	onClick: () => void;
}) => (
	<button
		type="button"
		role="radio"
		aria-checked={active}
		aria-label={label}
		onClick={onClick}
		className="mono"
		style={{
			padding: "4px 14px",
			fontSize: 11,
			fontWeight: active ? 600 : 400,
			background: active ? "var(--white)" : "transparent",
			color: active ? "#ffffff" : "var(--muted)",
			border: "none",
			borderRadius: "var(--radius-full)",
			cursor: "pointer",
			transition: "all 140ms ease",
			lineHeight: "1.5",
		}}
	>
		{label}
	</button>
);

// ── Full chart ─────────────────────────────────────────────────────────────

const FullChart = ({
	months,
	model,
	mode,
	forecastsByMonth,
	categorySeries,
	selectedMonth,
	onSelectMonth,
	onDropForecast,
}: {
	months: ReadonlyArray<ChartMonthView>;
	model: ChartModel;
	mode: ChartMode;
	forecastsByMonth: Map<string, ForecastView[]>;
	categorySeries: CategoryMonthSeries;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
}) => {
	const [hover, setHover] = useState<number | null>(null);
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

	// Year totals
	const yearIn = model.realIns.reduce((s, v, i) => s + v + model.fcIns[i], 0);
	const yearOut = model.realOuts.reduce(
		(s, v, i) => s + v + model.fcOuts[i],
		0,
	);

	const isExpensesMode = mode.startsWith("despesas");

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

			<div style={{ position: "relative" }}>
				<svg
					viewBox={`0 0 ${W} ${H}`}
					width="100%"
					role="img"
					aria-label={
						mode === "caixa"
							? "gráfico de caixa mensal — barras de entradas e saídas, linha de saldo"
							: "gráfico de despesas mensais"
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
									{/* Income bar — forecast (lighter cyan); fades on hover
									    so the realized picture reads clean (N4). */}
									{fIn > 0.5 && (
										<rect
											x={ix}
											y={fInTop}
											width={barW}
											height={fIn}
											rx={2}
											fill="#99f6e4"
											opacity={hover !== null ? 0.15 : 1}
											style={{ transition: "opacity 120ms" }}
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
									    remainder is the budget envelope. Both fade on hover (N4). */}
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
											const op = hover !== null ? 0.15 : 1;
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
															opacity={op}
															style={{ transition: "opacity 120ms" }}
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
															opacity={op}
															style={{ transition: "opacity 120ms" }}
														>
															<title>parcelas comprometidas</title>
														</rect>
													)}
												</>
											);
										})()}

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
											return (
												<rect
													key={cat}
													x={bx}
													y={yCursor}
													width={expBarW}
													height={h}
													fill={catColor(ci, nCats)}
													opacity={isSel || isHov ? 1 : 0.85}
												>
													<title>{`${cat}: ${formatMoneyNumber(mag)} (${pct}%)`}</title>
												</rect>
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
					{/* Balance dots left of expense bars in despesas-barras mode — removed */}
				</svg>

				{/* Interaction overlay */}
				<ColumnOverlay
					months={months}
					selectedMonth={selectedMonth}
					onSelectMonth={onSelectMonth}
					onHover={setHover}
					onDropForecast={onDropForecast}
				/>

				{/* Hover popover */}
				{hover != null && (
					<BarPopover
						month={months[hover]}
						forecasts={forecastsByMonth.get(months[hover].month) ?? []}
						leftPct={((hover + 0.5) / n) * 100}
					/>
				)}
			</div>

			{/* Legend */}
			<ChartLegend mode={mode} months={months} />
		</div>
	);
};

// ── Legend ─────────────────────────────────────────────────────────────────

const ChartLegend = ({
	mode,
	months,
}: {
	mode: ChartMode;
	months: ReadonlyArray<ChartMonthView>;
}) => {
	const hasFc = months.some((m) => m.isFuture === 1);

	const items: Array<{
		color: string;
		label: string;
		dashed?: boolean;
	}> = [];

	if (mode === "caixa") {
		items.push({ color: "var(--cyan)", label: "entradas" });
		items.push({ color: "var(--rose)", label: "saídas" });
		if (hasFc) {
			items.push({ color: "#99f6e4", label: "entrada prevista" });
			items.push({ color: "#fda4af", label: "saída prevista" });
			items.push({ color: "var(--amber)", label: "parcela prevista" });
		}
		items.push({
			color: "var(--purple)",
			label: "saldo",
			dashed: true,
		});
	} else if (mode === "despesas-barras") {
		items.push({ color: "var(--rose)", label: "realizado" });
		if (hasFc)
			items.push({
				color: "#fda4af",
				label: "previsto",
			});
	}

	return (
		<div
			className="mono"
			style={{
				display: "flex",
				flexWrap: "wrap",
				gap: 14,
				fontSize: 10,
				color: "var(--muted)",
				marginTop: 6,
			}}
		>
			{items.map((it) => (
				<LegendSwatch
					key={it.label}
					color={it.color}
					label={it.label}
					dashed={it.dashed}
				/>
			))}
		</div>
	);
};

// ── Shared interaction overlay ─────────────────────────────────────────────

const ColumnOverlay = ({
	months,
	selectedMonth,
	onSelectMonth,
	onHover,
	onDropForecast,
}: {
	months: ReadonlyArray<ChartMonthView>;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onHover: (i: number | null) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
}) => (
	<div
		style={{
			position: "absolute",
			inset: 0,
			display: "grid",
			gridTemplateColumns: `repeat(${months.length}, 1fr)`,
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

const MonthColumn = ({
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
			aria-label={`selecionar ${month}`}
		/>
	);
};

// ── Hover popover ──────────────────────────────────────────────────────────

const BarPopover = ({
	month,
	forecasts,
	leftPct,
}: {
	month: ChartMonthView;
	forecasts: ForecastView[];
	leftPct: number;
}) => {
	const ref = useRef<HTMLDivElement>(null);
	const [side, setSide] = useState<"left" | "right">(
		leftPct > 58 ? "right" : "left",
	);

	useEffect(() => {
		const el = ref.current;
		if (!el) return;
		const rect = el.getBoundingClientRect();
		if (side === "left" && rect.right > window.innerWidth - 8) setSide("right");
		else if (side === "right" && rect.left < 8) setSide("left");
	}, [side]);

	const totalIn =
		numeric(month.inflows) + numeric(month.forecastInflowsRemaining);
	const totalOut =
		Math.abs(numeric(month.outflows)) +
		Math.abs(numeric(month.forecastOutflowsRemaining));
	const close = month.isFuture
		? month.projectedClosingBalance
		: month.closingBalance;
	const manualForecasts = forecasts.filter((f) => f.kind === "manual");

	return (
		<div
			ref={ref}
			className="mono"
			style={
				{
					position: "absolute",
					top: 6,
					[side === "right" ? "right" : "left"]:
						side === "right" ? `${100 - leftPct + 2}%` : `${leftPct + 2}%`,
					background: "var(--surface)",
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-md)",
					padding: "10px 12px",
					fontSize: 11,
					lineHeight: 1.75,
					pointerEvents: "none",
					minWidth: 190,
					maxWidth: 250,
					zIndex: 6,
					boxShadow: "0 4px 16px rgba(21,19,31,0.08)",
				} as React.CSSProperties
			}
		>
			<div
				style={{
					fontWeight: 600,
					color: "var(--white)",
					marginBottom: 4,
				}}
			>
				{month.label}
				{month.isFuture ? (
					<span style={{ color: "var(--muted)", fontWeight: 400 }}>
						{" "}
						· previsto
					</span>
				) : null}
			</div>
			<div style={{ color: "var(--cyan)" }}>↑ {formatMoneyNumber(totalIn)}</div>
			<div style={{ color: "var(--rose)" }}>
				↓ {formatMoneyNumber(totalOut)}
			</div>
			<div
				style={{
					color: Number(close) >= 0 ? "var(--purple)" : "var(--rose)",
				}}
			>
				= {formatMoneyNumber(Number(close))}
			</div>
			{manualForecasts.length > 0 && (
				<div
					style={{
						marginTop: 6,
						paddingTop: 6,
						borderTop: "1px solid var(--border)",
					}}
				>
					<div
						style={{
							color: "var(--muted)",
							fontSize: 10,
							marginBottom: 2,
						}}
					>
						previsões manuais
					</div>
					{manualForecasts.slice(0, 5).map((f) => (
						<div
							key={f.forecastId}
							style={{
								display: "flex",
								justifyContent: "space-between",
								gap: 8,
							}}
						>
							<span
								style={{
									overflow: "hidden",
									textOverflow: "ellipsis",
									whiteSpace: "nowrap",
									color: "var(--muted)",
								}}
							>
								⠿ {f.description}
							</span>
							<span
								style={{
									color: Number(f.amount) < 0 ? "var(--rose)" : "var(--cyan)",
									whiteSpace: "nowrap",
								}}
							>
								{formatMoneyNumber(Math.abs(Number(f.amount)))}
							</span>
						</div>
					))}
					{manualForecasts.length > 5 && (
						<div
							style={{
								color: "var(--muted2)",
								fontSize: 10,
							}}
						>
							+{manualForecasts.length - 5} mais
						</div>
					)}
				</div>
			)}
		</div>
	);
};

// ── Legend swatch ──────────────────────────────────────────────────────────

const LegendSwatch = ({
	color,
	label,
	dashed,
}: {
	color: string;
	label: string;
	dashed?: boolean;
}) => (
	<span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
		<span
			style={{
				width: dashed ? 14 : 10,
				height: dashed ? 0 : 10,
				borderRadius: dashed ? 0 : 2,
				background: color,
				border: dashed ? `1.5px dashed ${color}` : "none",
			}}
		/>
		{label}
	</span>
);
