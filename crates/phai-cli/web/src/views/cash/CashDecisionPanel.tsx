import type { CSSProperties } from "react";
import { CountMoney } from "../../components/ui";
import { toCents } from "../../lib/format";
import type { ChartMonthView } from "../types";

/**
 * Painel de decisão do caixa — the headline band above the cash chart. It
 * answers "como está meu caixa?" before the chart is even read: the selected
 * month's balance is the dominant figure, with entradas / saídas / resultado /
 * saldo projetado as supporting KPIs and a positive/negative state badge.
 *
 * On scroll it collapses (via the `compact` prop, driven by Dashboard's scroll
 * hysteresis) to a thin sticky strip — replacing the old mini-bar CompactChart
 * that shrank to unreadable bars. The chart itself now lives below this panel
 * and scrolls away normally (it complements the reading, it doesn't lead it).
 */

export type CashWhen = "past" | "current" | "future";

export interface CashSummary {
	entradas: number;
	saidas: number;
	resultado: number;
	/** Headline balance: realized closing for past/current, projected for future. */
	saldo: number;
	/** Projected end-of-month balance (always projectedClosingBalance). */
	projetado: number;
	/** resultado >= 0 — the month's net is in the black (drives the result KPI). */
	positive: boolean;
	/** saldo >= 0 — the headline balance is positive (drives the header badge). */
	balancePositive: boolean;
}

/**
 * Derive the cash summary for a month. Arithmetic runs in integer cents
 * (`toCents`) so income/expense sums never drift; the server-computed string
 * totals are the source of truth.
 */
export function cashSummary(row: ChartMonthView, when: CashWhen): CashSummary {
	const entradasCents =
		toCents(row.inflows) + toCents(row.forecastInflowsRemaining);
	const saidasCents =
		Math.abs(toCents(row.outflows)) +
		Math.abs(toCents(row.forecastOutflowsRemaining));
	const resultadoCents = entradasCents - saidasCents;
	const saldoCents = toCents(
		when === "future" ? row.projectedClosingBalance : row.closingBalance,
	);
	return {
		entradas: entradasCents / 100,
		saidas: saidasCents / 100,
		resultado: resultadoCents / 100,
		saldo: saldoCents / 100,
		projetado: toCents(row.projectedClosingBalance) / 100,
		positive: resultadoCents >= 0,
		// The header badge describes the headline balance it sits next to — not
		// the month's net result (which has its own KPI). A positive balance with
		// a deficit month should read "positive", not "negative".
		balancePositive: saldoCents >= 0,
	};
}

const saldoLabel = (when: CashWhen): string =>
	when === "future"
		? "saldo projetado"
		: when === "current"
			? "saldo em caixa"
			: "saldo final";

const labelStyle: CSSProperties = {
	fontFamily: "var(--font-body)",
	fontWeight: 600,
	fontSize: 11,
	letterSpacing: "0.12em",
	textTransform: "uppercase",
	color: "var(--muted)",
};

export const CashDecisionPanel = ({
	row,
	when,
	compact,
}: {
	row: ChartMonthView;
	when: CashWhen;
	compact: boolean;
}) => {
	const s = cashSummary(row, when);
	const resultColor = s.positive ? "var(--green)" : "var(--rose)";
	const sign = s.positive ? "+" : "";

	if (compact) {
		// Thin sticky strip — month · saldo · entradas · saídas · resultado.
		return (
			<div
				className="mono"
				style={{
					display: "flex",
					alignItems: "baseline",
					gap: 16,
					fontSize: 12,
					flexWrap: "wrap",
				}}
			>
				<strong style={{ fontSize: 13 }}>{row.label}</strong>
				<span style={{ color: "var(--muted)" }}>
					{saldoLabel(when)}{" "}
					<CountMoney
						value={s.saldo}
						style={{ color: s.saldo < 0 ? "var(--rose)" : "var(--white)", fontWeight: 600 }}
					/>
				</span>
				<span style={{ color: "var(--cyan)" }}>
					↑ <CountMoney value={s.entradas} />
				</span>
				<span style={{ color: "var(--rose)" }}>
					↓ <CountMoney value={s.saidas} />
				</span>
				<span style={{ color: resultColor }}>
					= {sign}
					<CountMoney value={s.resultado} />
				</span>
			</div>
		);
	}

	return (
		<div style={{ paddingBottom: 4 }}>
			{/* Header: label + month + positive/negative badge */}
			<div
				style={{
					display: "flex",
					alignItems: "baseline",
					gap: 10,
					marginBottom: 2,
				}}
			>
				<span style={labelStyle}>{saldoLabel(when)}</span>
				<span className="mono" style={{ fontSize: 12, color: "var(--muted2)" }}>
					{row.label}
				</span>
				<span
					className="mono"
					style={{
						marginLeft: "auto",
						fontSize: 11,
						fontWeight: 600,
						padding: "2px 10px",
						borderRadius: "var(--radius-full)",
						color: s.balancePositive ? "var(--green)" : "var(--rose)",
						background: s.balancePositive
							? "rgba(21,128,61,0.10)"
							: "rgba(225,29,72,0.10)",
					}}
				>
					{s.balancePositive ? "positivo" : "negativo"}
				</span>
			</div>

			{/* Headline balance — the dominant figure on the screen */}
			<CountMoney
				value={s.saldo}
				style={{
					fontFamily: "var(--font-display)",
					fontSize: "clamp(1.7rem, 3.6vw, 2.3rem)",
					fontWeight: 700,
					letterSpacing: "-0.02em",
					lineHeight: 1.05,
					color: s.saldo < 0 ? "var(--rose)" : "var(--white)",
				}}
			/>

			{/* Supporting KPIs */}
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit, minmax(120px, 1fr))",
					gap: 10,
					marginTop: 10,
					maxWidth: 640,
				}}
			>
				<Kpi label="entradas" prefix="↑ " value={s.entradas} color="var(--cyan)" />
				<Kpi label="saídas" prefix="↓ " value={s.saidas} color="var(--rose)" />
				<Kpi
					label="resultado"
					prefix={sign}
					value={s.resultado}
					color={resultColor}
				/>
				{when !== "past" && (
					<Kpi label="saldo projetado" value={s.projetado} color="var(--muted)" />
				)}
			</div>
		</div>
	);
};

const Kpi = ({
	label,
	value,
	color,
	prefix = "",
}: {
	label: string;
	value: number;
	color: string;
	prefix?: string;
}) => (
	<div
		style={{
			border: "1px solid var(--border)",
			borderRadius: "var(--radius-md)",
			padding: "8px 12px",
			minWidth: 0,
		}}
	>
		<div style={{ ...labelStyle, fontSize: 10, marginBottom: 3 }}>{label}</div>
		<div
			className="mono"
			style={{
				display: "flex",
				alignItems: "baseline",
				fontSize: 16,
				fontWeight: 600,
				color,
			}}
		>
			{prefix}
			<CountMoney value={value} />
		</div>
	</div>
);
