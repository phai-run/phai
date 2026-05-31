import { useMemo, useRef, useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { formatMoneyNumber, numeric } from "../lib/format";
import { useDnd } from "../lib/dnd";
import type { ChartMonthView, ForecastView, ChartMode } from "./types";

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
	};
}

// Convert bar magnitude → SVG height
const bh = (v: number, maxBar: number) => (v / maxBar) * BAR_MAX;
// Convert closing balance → SVG y-coord (full innerH range)
const by = (v: number, minBal: number, balSpan: number) =>
	PAD.top + (1 - (v - minBal) / balSpan) * innerH;

// ── Public component ───────────────────────────────────────────────────────

export const PlanningChart = ({
	months,
	forecastsByMonth,
	selectedMonth,
	onSelectMonth,
	onDropForecast,
	compact,
}: {
	months: ReadonlyArray<ChartMonthView>;
	forecastsByMonth: Map<string, ForecastView[]>;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
	compact: boolean;
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

			<AnimatePresence mode="wait" initial={false}>
				{compact ? (
					<motion.div
						key="compact"
						initial={{ opacity: 0, y: -4 }}
						animate={{ opacity: 1, y: 0 }}
						exit={{ opacity: 0, y: -4 }}
						transition={{ duration: 0.18 }}
					>
						<CompactChart
							months={months}
							model={model}
							mode={mode}
							selectedMonth={selectedMonth}
							onSelectMonth={onSelectMonth}
							onDropForecast={onDropForecast}
						/>
					</motion.div>
				) : (
					<motion.div
						key="full"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: 0.18 }}
					>
						<FullChart
							months={months}
							model={model}
							mode={mode}
							forecastsByMonth={forecastsByMonth}
							selectedMonth={selectedMonth}
							onSelectMonth={onSelectMonth}
							onDropForecast={onDropForecast}
						/>
					</motion.div>
				)}
			</AnimatePresence>
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
			label="Despesas (barras)"
			active={mode === "despesas-barras"}
			onClick={() => onChange("despesas-barras")}
		/>
		<ModeChip
			label="Despesas (linha)"
			active={mode === "despesas-linha"}
			onClick={() => onChange("despesas-linha")}
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
	selectedMonth,
	onSelectMonth,
	onDropForecast,
}: {
	months: ReadonlyArray<ChartMonthView>;
	model: ChartModel;
	mode: ChartMode;
	forecastsByMonth: Map<string, ForecastView[]>;
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

	const linePath = (vals: number[], offset = 0) =>
		vals
			.map(
				(b, k) =>
					`${k === 0 ? "M" : "L"} ${midX(offset + k)} ${by(b, model.minBal, model.balSpan)}`,
			)
			.join(" ");

	// ── Expense line/area helpers ───────────────────────────────────────
	const expMax = mode === "caixa" ? model.maxBar : model.expMaxBar;

	const expLinePath = (vals: number[], offset = 0) =>
		vals
			.map(
				(v, k) =>
					`${k === 0 ? "M" : "L"} ${midX(offset + k)} ${BASELINE - bh(v, expMax)}`,
			)
			.join(" ");

	const expAreaPath = (vals: number[], offset = 0) => {
		if (vals.length === 0) return "";
		const top = vals
			.map((v, k) => `L ${midX(offset + k)} ${BASELINE - bh(v, expMax)}`)
			.join(" ");
		const lastX = midX(offset + vals.length - 1);
		const firstX = midX(offset);
		return `M ${firstX} ${BASELINE} ${top} L ${lastX} ${BASELINE} Z`;
	};

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
						entradas {formatMoneyNumber(yearIn)}
					</span>
				)}
				<span style={{ color: "var(--rose)" }}>
					saídas {formatMoneyNumber(-yearOut)}
				</span>
				<span
					style={{
						color: yearIn - yearOut >= 0 ? "var(--green)" : "var(--rose)",
					}}
				>
					resultado {yearIn - yearOut >= 0 ? "+" : ""}
					{formatMoneyNumber(yearIn - yearOut)}
				</span>
			</div>

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
						y1={BASELINE}
						y2={BASELINE}
						stroke="var(--border)"
						strokeWidth={0.5}
					/>

					{/* ── Caixa mode: income + expense bars + balance line ── */}
					{mode === "caixa" &&
						months.map((m, i) => {
							const rIn = bh(model.realIns[i], model.maxBar);
							const fIn = bh(model.fcIns[i], model.maxBar);
							const rOut = bh(model.realOuts[i], model.maxBar);
							const fOut = bh(model.fcOuts[i], model.maxBar);
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
											y={BASELINE - rIn}
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
											y={BASELINE - rIn - fIn}
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
											y={BASELINE - rOut}
											width={barW}
											height={rOut}
											rx={2}
											fill="var(--rose)"
										/>
									)}
									{/* Expense bar — forecast (lighter rose) */}
									{fOut > 0.5 && (
										<rect
											x={ox}
											y={BASELINE - rOut - fOut}
											width={barW}
											height={fOut}
											rx={2}
											fill="url(#fc-exp-bar)"
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

									{/* Realized expense bar — solid rose */}
									{rOut > 0.5 && (
										<rect
											x={bx}
											y={BASELINE - rOut}
											width={expBarW}
											height={rOut}
											rx={3}
											fill="var(--rose)"
										/>
									)}
									{/* Forecast expense bar — lighter solid */}
									{fOut > 0.5 && (
										<rect
											x={bx}
											y={BASELINE - rOut - fOut}
											width={expBarW}
											height={fOut}
											rx={3}
											fill="url(#fc-exp-bar)"
										/>
									)}

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

					{/* ── Despesas-linha mode: expense line/area chart ── */}
					{mode === "despesas-linha" && (
						<>
							{/* Realized area (past months only) */}
							{firstFuture === -1 || firstFuture > 0 ? (
								<path
									d={
										firstFuture === -1
											? expAreaPath(model.realOuts)
											: expAreaPath(model.realOuts.slice(0, firstFuture))
									}
									fill="#e11d48"
									fillOpacity={0.12}
								/>
							) : null}
							{/* Realized line */}
							{firstFuture === -1 || firstFuture > 0 ? (
								<path
									d={
										firstFuture === -1
											? expLinePath(model.realOuts)
											: expLinePath(model.realOuts.slice(0, firstFuture))
									}
									fill="none"
									stroke="var(--rose)"
									strokeWidth={2}
								/>
							) : null}

							{/* Forecast area (future months) */}
							{firstFuture !== -1 && firstFuture < n && (
								<path
									d={expAreaPath(model.fcOuts.slice(firstFuture), firstFuture)}
									fill="url(#fc-exp-area)"
								/>
							)}
							{/* Forecast line */}
							{firstFuture !== -1 && firstFuture < n && (
								<path
									d={expLinePath(model.fcOuts.slice(firstFuture), firstFuture)}
									fill="none"
									stroke="#fda4af"
									strokeWidth={2}
									strokeDasharray="5 3"
								/>
							)}

							{/* Data point circles: realized */}
							{model.realOuts.map(
								(v, i) =>
									!months[i].isFuture &&
									v > 0 && (
										<circle
											key={months[i].month}
											cx={midX(i)}
											cy={BASELINE - bh(v, expMax)}
											r={3}
											fill="var(--rose)"
										/>
									),
							)}
							{/* Data point circles: forecast */}
							{model.fcOuts.map(
								(v, i) =>
									months[i].isFuture &&
									v > 0 && (
										<circle
											key={`fc-${months[i].month}`}
											cx={midX(i)}
											cy={BASELINE - bh(v, expMax)}
											r={2.5}
											fill="#fda4af"
											stroke="#fff"
											strokeWidth={1}
										/>
									),
							)}

							{/* Month labels */}
							{months.map((m, i) => {
								const isSel = m.month === selectedMonth;
								return (
									<g key={m.label}>
										{(isSel || hover === i) && !isSel && (
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
						</>
					)}

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
									cy={by(b, model.minBal, model.balSpan)}
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
		if (hasFc) items.push({ color: "#99f6e4", label: "previsão" });
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
	} else if (mode === "despesas-linha") {
		items.push({ color: "var(--rose)", label: "realizado (área)" });
		if (hasFc)
			items.push({
				color: "#fda4af",
				label: "previsto",
				dashed: true,
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

// ── Compact chart ──────────────────────────────────────────────────────────

const CompactChart = ({
	months,
	model,
	mode,
	selectedMonth,
	onSelectMonth,
	onDropForecast,
}: {
	months: ReadonlyArray<ChartMonthView>;
	model: ChartModel;
	mode: ChartMode;
	selectedMonth: string | null;
	onSelectMonth: (month: string) => void;
	onDropForecast: (forecastId: string, targetMonth: string) => void;
}) => {
	const BAR_MAX_C = 30;
	const expMax = mode === "caixa" ? model.maxBar : model.expMaxBar;

	return (
		<div style={{ position: "relative", height: 56 }}>
			{/* Visual layer */}
			<div
				style={{
					position: "absolute",
					inset: 0,
					display: "flex",
					alignItems: "flex-end",
					paddingBottom: 16,
				}}
			>
				{months.map((m, i) => {
					const inH = Math.max(
						2,
						((model.realIns[i] + model.fcIns[i]) / model.maxBar) * BAR_MAX_C,
					);
					const outH = Math.max(
						2,
						((model.realOuts[i] + model.fcOuts[i]) / expMax) * BAR_MAX_C,
					);
					const rOutH = Math.max(1, (model.realOuts[i] / expMax) * BAR_MAX_C);
					const fOutH = Math.max(1, (model.fcOuts[i] / expMax) * BAR_MAX_C);
					const isSel = m.month === selectedMonth;

					return (
						<div
							key={m.month}
							style={{
								flex: 1,
								display: "flex",
								flexDirection: "column",
								alignItems: "center",
								justifyContent: "flex-end",
								height: "100%",
								borderRadius: 4,
								background: isSel
									? mode === "caixa"
										? "rgba(13,148,136,0.09)"
										: "rgba(225,29,72,0.07)"
									: "transparent",
								position: "relative",
							}}
						>
							{/* Mini bars */}
							<div
								style={{
									display: "flex",
									alignItems: "flex-end",
									gap: 1,
									marginBottom: 2,
								}}
							>
								{mode === "caixa" && (
									<>
										<div
											style={{
												width: 4,
												height: inH,
												background: "var(--cyan)",
												borderRadius: "1px 1px 0 0",
												opacity: model.fcIns[i] > 0 ? 0.7 : 1,
											}}
										/>
										<div
											style={{
												width: 4,
												height: outH,
												background: "var(--rose)",
												borderRadius: "1px 1px 0 0",
												opacity: model.fcOuts[i] > 0 ? 0.7 : 1,
											}}
										/>
									</>
								)}
								{mode === "despesas-barras" && (
									<>
										<div
											style={{
												width: 6,
												height: rOutH,
												background: "var(--rose)",
												borderRadius: "1px 1px 0 0",
											}}
										/>
										{model.fcOuts[i] > 0 && (
											<div
												style={{
													width: 6,
													height: fOutH,
													background: "#fda4af",
													borderRadius: "1px 1px 0 0",
												}}
											/>
										)}
									</>
								)}
								{mode === "despesas-linha" && (
									<div
										style={{
											width: 5,
											height: outH,
											background: "var(--rose)",
											borderRadius: "1px 1px 0 0",
											opacity: model.fcOuts[i] > 0 ? 0.5 : 0.85,
										}}
									/>
								)}
							</div>
							{/* Month label */}
							<span
								className="mono"
								style={{
									position: "absolute",
									bottom: 0,
									fontSize: 8,
									color: isSel
										? mode === "caixa"
											? "var(--cyan)"
											: "var(--rose)"
										: "var(--muted2)",
									fontWeight: isSel ? 600 : 400,
									lineHeight: 1,
								}}
							>
								{m.label.slice(0, 3)}
							</span>
						</div>
					);
				})}
			</div>

			{/* Interaction overlay */}
			<ColumnOverlay
				months={months}
				selectedMonth={selectedMonth}
				onSelectMonth={onSelectMonth}
				onHover={() => {}}
				onDropForecast={onDropForecast}
			/>
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
				if (payload.forecastId) onDropForecast(payload.forecastId, month);
			},
		});
	}, [month, registerTarget, onDropForecast]);

	const isDropTarget = dragging != null && hoverTargetId === `month:${month}`;

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
