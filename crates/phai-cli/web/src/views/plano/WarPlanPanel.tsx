import { queryDb } from "@livestore/livestore";
import { useQuery } from "@livestore/react";
import { useMemo, useState } from "react";
import { tables } from "../../livestore/schema";
import { categoryEmoji } from "../../lib/categoryEmoji";
import { formatMoneyNumber } from "../../lib/format";
import {
	buildOverlayMap,
	buildWarPlan,
	simulateWarPlan,
	type TxView,
	type WarPlanRow,
} from "../../lib/derivations";
import type { ForecastView } from "../types";

const txAll$ = queryDb(tables.transactions);
const overlay$ = queryDb(tables.reviewOverlay);

/**
 * Plano de guerra — the cost-cutting workbench for one month. Per parent
 * category it lines up the budget envelope, the realized spend, the 3-month
 * average and the projection (`max(realizado, orçamento)` — the same envelope
 * model the chart uses). Typing a target budget into the "simular" column
 * recomputes the month's projection client-side, floored at what's already
 * spent, and shows the monthly + rest-of-year saving. Nothing is persisted —
 * it's a sandbox; the real budgets stay in the forecast envelopes.
 */
export const WarPlanPanel = ({
	month,
	forecasts,
	isPast,
}: {
	month: string;
	forecasts: ForecastView[];
	isPast: boolean;
}) => {
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const overlay = useQuery(overlay$);
	const overlayMap = useMemo(() => buildOverlayMap(overlay), [overlay]);

	const plan = useMemo(
		() =>
			buildWarPlan(
				txRows,
				month,
				forecasts.map((f) => ({
					amount: f.amount,
					categoryId: f.categoryId,
					kind: f.kind,
					status: f.status,
					month: f.month,
				})),
				overlayMap,
				isPast ? "past" : "open",
			),
		[txRows, month, forecasts, overlayMap, isPast],
	);

	const [targets, setTargets] = useState<Map<string, number>>(new Map());
	const sim = useMemo(() => simulateWarPlan(plan, targets), [plan, targets]);

	const setTarget = (parent: string, raw: string) => {
		setTargets((prev) => {
			const next = new Map(prev);
			const value = Number(raw.replace(",", "."));
			if (raw.trim() === "" || !Number.isFinite(value)) next.delete(parent);
			else next.set(parent, value);
			return next;
		});
	};

	const monthsLeftInYear = 12 - Number(month.slice(5, 7));
	const hasSim = targets.size > 0 && sim.economiaMes !== 0;
	const maxProjecao = Math.max(1, ...plan.rows.map((r) => r.projecao));

	return (
		<section aria-label={`Plano de guerra de ${month}`}>
			{/* Summary strip */}
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit, minmax(160px, 1fr))",
					gap: 12,
					padding: "12px 0",
				}}
			>
				<SummaryCard label="projeção do mês" value={plan.totalProjecao} />
				<SummaryCard label="já realizado" value={plan.totalRealizado} />
				<SummaryCard
					label="com cortes simulados"
					value={sim.projecaoSimulada}
					accent={hasSim ? "var(--purple)" : undefined}
				/>
				<SummaryCard
					label="economia / mês"
					value={sim.economiaMes}
					accent={hasSim ? "var(--green)" : undefined}
				/>
				<SummaryCard
					label={`economia até dez (×${monthsLeftInYear})`}
					value={sim.economiaMes * monthsLeftInYear}
					accent={hasSim ? "var(--green)" : undefined}
				/>
			</div>

			<div
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-md)",
					overflow: "auto",
					background: "var(--card)",
				}}
			>
				<table
					style={{
						width: "100%",
						// "separate": collapsed borders make body cells paint through
						// the sticky header while scrolling.
						borderCollapse: "separate",
						borderSpacing: 0,
						fontSize: 14,
					}}
				>
					<thead>
						<tr className="mono">
							<th style={thStyle}>categoria</th>
							<th style={{ ...thStyle, textAlign: "right" }}>orçamento</th>
							<th style={{ ...thStyle, textAlign: "right" }}>realizado</th>
							<th style={{ ...thStyle, width: "22%" }}>uso</th>
							<th style={{ ...thStyle, textAlign: "right" }}>média 3m</th>
							<th style={{ ...thStyle, textAlign: "right" }}>projeção</th>
							<th style={{ ...thStyle, textAlign: "right" }}>simular corte</th>
							<th style={{ ...thStyle, textAlign: "right" }}>Δ mês</th>
						</tr>
					</thead>
					<tbody>
						{plan.rows.map((row) => (
							<PlanRow
								key={row.parent}
								row={row}
								maxProjecao={maxProjecao}
								target={targets.get(row.parent)}
								onTarget={(v) => setTarget(row.parent, v)}
							/>
						))}
					</tbody>
				</table>
			</div>

			<div
				className="mono"
				style={{
					display: "flex",
					gap: 16,
					alignItems: "center",
					padding: "10px 4px",
					fontSize: 12,
					color: "var(--muted)",
					flexWrap: "wrap",
				}}
			>
				{plan.parcelasComprometidas > 0 && (
					<span>
						parcelas já comprometidas no mês:{" "}
						{formatMoneyNumber(plan.parcelasComprometidas)} (dentro das
						categorias quando pagas)
					</span>
				)}
				<span>
					faturas de cartão não entram aqui — as compras já estão nas categorias.
				</span>
				{targets.size > 0 && (
					<button
						onClick={() => setTargets(new Map())}
						className="mono"
						style={{
							marginLeft: "auto",
							background: "transparent",
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-full)",
							padding: "4px 12px",
							cursor: "pointer",
							fontSize: 11,
							color: "var(--muted)",
						}}
					>
						zerar simulação
					</button>
				)}
			</div>
		</section>
	);
};

const SummaryCard = ({
	label,
	value,
	accent,
}: {
	label: string;
	value: number;
	accent?: string;
}) => (
	<div
		style={{
			border: "1px solid var(--border)",
			borderRadius: "var(--radius-md)",
			padding: "10px 14px",
			background: "var(--card)",
		}}
	>
		<div
			className="mono"
			style={{
				fontSize: 11,
				textTransform: "uppercase",
				letterSpacing: "0.08em",
				color: "var(--muted)",
			}}
		>
			{label}
		</div>
		<div
			style={{
				fontFamily: "var(--font-display)",
				fontSize: "1.25rem",
				color: accent ?? "var(--text)",
				fontVariantNumeric: "tabular-nums",
			}}
		>
			{formatMoneyNumber(value)}
		</div>
	</div>
);

const PlanRow = ({
	row,
	maxProjecao,
	target,
	onTarget,
}: {
	row: WarPlanRow;
	maxProjecao: number;
	target: number | undefined;
	onTarget: (raw: string) => void;
}) => {
	const overBudget = row.orcamento != null && row.realizado > row.orcamento;
	const usagePct =
		row.orcamento != null && row.orcamento > 0
			? Math.min(100, (row.realizado / row.orcamento) * 100)
			: null;
	const simulated =
		target != null ? Math.max(row.realizado, Math.max(0, target)) : null;
	const delta = simulated != null ? row.projecao - simulated : 0;
	const floored = target != null && target < row.realizado;

	return (
		<tr>
			<td style={tdStyle}>
				<span style={{ fontWeight: 500 }}>
					{categoryEmoji(row.parent)} {row.parent}
				</span>
			</td>
			<td className="mono" style={{ ...tdStyle, textAlign: "right" }}>
				{row.orcamento != null ? formatMoneyNumber(row.orcamento) : "—"}
			</td>
			<td
				className="mono"
				style={{
					...tdStyle,
					textAlign: "right",
					color: overBudget ? "var(--rose)" : "var(--text)",
				}}
			>
				{formatMoneyNumber(row.realizado)}
			</td>
			<td style={tdStyle}>
				<div
					aria-hidden
					style={{
						position: "relative",
						height: 8,
						borderRadius: 4,
						background: "var(--border)",
						overflow: "hidden",
					}}
				>
					{/* Scale bars against the biggest projection so categories compare visually. */}
					<div
						style={{
							position: "absolute",
							inset: 0,
							width: `${(row.projecao / maxProjecao) * 100}%`,
							background: "var(--chip, rgba(124,93,250,0.18))",
							borderRadius: 4,
						}}
					/>
					<div
						style={{
							position: "absolute",
							inset: 0,
							width: `${(row.realizado / maxProjecao) * 100}%`,
							background: overBudget ? "var(--rose)" : "var(--purple)",
							borderRadius: 4,
							opacity: 0.85,
						}}
					/>
				</div>
				{usagePct != null && (
					<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
						{Math.round((row.realizado / (row.orcamento || 1)) * 100)}% do
						orçamento
					</div>
				)}
			</td>
			<td className="mono" style={{ ...tdStyle, textAlign: "right", color: "var(--muted)" }}>
				{row.media3m > 0 ? formatMoneyNumber(row.media3m) : "—"}
			</td>
			<td className="mono" style={{ ...tdStyle, textAlign: "right", fontWeight: 600 }}>
				{formatMoneyNumber(row.projecao)}
			</td>
			<td style={{ ...tdStyle, textAlign: "right" }}>
				<input
					inputMode="decimal"
					aria-label={`novo orçamento para ${row.parent}`}
					placeholder={
						row.orcamento != null ? String(Math.round(row.orcamento)) : "—"
					}
					value={target ?? ""}
					onChange={(e) => onTarget(e.target.value)}
					className="mono"
					style={{
						width: 90,
						textAlign: "right",
						border: `1px solid ${floored ? "var(--amber)" : "var(--border)"}`,
						borderRadius: "var(--radius-sm)",
						padding: "4px 8px",
						fontSize: 12,
						background: "var(--bg)",
					}}
					title={
						floored
							? "abaixo do já realizado — o piso é o que já foi gasto"
							: "simule um novo orçamento mensal"
					}
				/>
			</td>
			<td
				className="mono"
				style={{
					...tdStyle,
					textAlign: "right",
					color: delta > 0 ? "var(--green)" : "var(--muted)",
				}}
			>
				{simulated != null && delta > 0 ? `−${formatMoneyNumber(delta)}` : "—"}
			</td>
		</tr>
	);
};

const thStyle: React.CSSProperties = {
	padding: "8px 12px",
	textAlign: "left",
	fontWeight: 500,
	fontSize: 12,
	textTransform: "uppercase",
	letterSpacing: "0.06em",
	color: "var(--muted)",
	// Sticky on the th (not the tr): collapsed-border sticky rows don't paint
	// their background reliably, so each header cell carries its own.
	position: "sticky",
	top: 0,
	zIndex: 2,
	background: "var(--card)",
	boxShadow: "0 1px 0 var(--border)",
};

const tdStyle: React.CSSProperties = {
	padding: "8px 12px",
	verticalAlign: "middle",
	// Row separator on the td: tr borders don't render with
	// border-collapse: separate (which the sticky header requires).
	borderBottom: "1px solid var(--border)",
};
