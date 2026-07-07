import type { CSSProperties } from "react";
import { CountMoney } from "../../components/ui";
import { toCents } from "../../lib/format";
import { monthTheme } from "../../lib/monthTheme";
import type { ChartMonthView } from "../types";

const MONTH_LABEL_FMT = new Intl.DateTimeFormat("pt-BR", {
	month: "long",
	year: "numeric",
});

/** "2026-07" → "Julho 2026" (capitalised) for the hero's themed month label. */
const monthLabel = (month: string): string => {
	const [y, m] = month.split("-").map(Number);
	if (!y || !m) return month;
	const s = MONTH_LABEL_FMT.format(new Date(y, m - 1, 1));
	return s.charAt(0).toUpperCase() + s.slice(1);
};

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

// Ghost arrows inside the grouped month-selector pill — the pill carries the
// border; the arrows stay quiet until hovered.
const monthNavBtn = (enabled: boolean): CSSProperties => ({
	border: "none",
	borderRadius: "var(--radius-full)",
	background: "transparent",
	color: enabled ? "var(--muted)" : "var(--muted2)",
	width: 24,
	height: 24,
	display: "inline-flex",
	alignItems: "center",
	justifyContent: "center",
	fontSize: 15,
	lineHeight: 1,
	cursor: enabled ? "pointer" : "default",
	opacity: enabled ? 1 : 0.35,
	padding: 0,
});

export const CashDecisionPanel = ({
	row,
	when,
	compact,
	onStepMonth,
	canStepPrev = false,
	canStepNext = false,
}: {
	row: ChartMonthView;
	when: CashWhen;
	compact: boolean;
	/** Step the selected month by -1 / +1 (arrows next to the themed month). */
	onStepMonth?: (dir: -1 | 1) => void;
	canStepPrev?: boolean;
	canStepNext?: boolean;
}) => {
	const s = cashSummary(row, when);
	const theme = monthTheme(row.month);
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
			{/* Context row — the month selector is the section's only control, so it
			    stands alone: arrows + themed month grouped into a single pill. */}
			<div style={{ display: "flex", alignItems: "center", marginBottom: 12 }}>
				<span
					style={{
						display: "inline-flex",
						alignItems: "center",
						gap: 2,
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-full)",
						background: "var(--card)",
						padding: "3px 6px",
					}}
				>
					{onStepMonth && (
						<button
							type="button"
							aria-label="mês anterior"
							title="mês anterior (Alt+←)"
							disabled={!canStepPrev}
							onClick={() => onStepMonth(-1)}
							style={monthNavBtn(canStepPrev)}
						>
							‹
						</button>
					)}
					<span
						style={{
							display: "inline-flex",
							alignItems: "center",
							gap: 7,
							padding: "0 8px",
						}}
					>
						<span
							aria-hidden
							title={theme.season}
							style={{ fontSize: "0.95rem", lineHeight: 1 }}
						>
							{theme.glyph}
						</span>
						<span
							style={{
								fontFamily: "var(--font-display)",
								fontSize: "0.98rem",
								fontWeight: 600,
								letterSpacing: "-0.01em",
								borderBottom: `2px solid ${theme.accent}`,
								paddingBottom: 1,
								whiteSpace: "nowrap",
							}}
						>
							{monthLabel(row.month)}
						</span>
					</span>
					{onStepMonth && (
						<button
							type="button"
							aria-label="próximo mês"
							title="próximo mês (Alt+→)"
							disabled={!canStepNext}
							onClick={() => onStepMonth(1)}
							style={monthNavBtn(canStepNext)}
						>
							›
						</button>
					)}
				</span>
			</div>

			{/* Headline zone — the balance owns the left, the KPI rail sits at its
			    baseline on the right. Two levels, nothing in between. */}
			<div
				style={{
					display: "flex",
					alignItems: "flex-end",
					justifyContent: "space-between",
					gap: "16px 48px",
					flexWrap: "wrap",
				}}
			>
				<div style={{ minWidth: 0 }}>
					<div
						style={{
							display: "flex",
							alignItems: "center",
							gap: 10,
							marginBottom: 2,
						}}
					>
						<span style={labelStyle}>{saldoLabel(when)}</span>
						{/* The badge qualifies the headline balance — it lives beside its
						    label, not orphaned across the screen. */}
						<span
							className="mono"
							style={{
								fontSize: 10,
								fontWeight: 600,
								padding: "1px 8px",
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
					<CountMoney
						value={s.saldo}
						style={{
							fontFamily: "var(--font-display)",
							fontSize: "clamp(1.8rem, 3.8vw, 2.5rem)",
							fontWeight: 700,
							letterSpacing: "-0.02em",
							lineHeight: 1.05,
							color: s.saldo < 0 ? "var(--rose)" : "var(--white)",
						}}
					/>
				</div>

				{/* Supporting KPI rail — hairline-separated columns instead of boxed
				    cards: quieter, reads as one unit subordinate to the balance. */}
				<div style={{ display: "flex", flexWrap: "wrap", rowGap: 12 }}>
					<Kpi label="entradas" prefix="↑ " value={s.entradas} color="var(--cyan)" />
					<Kpi label="saídas" prefix="↓ " value={s.saidas} color="var(--rose)" />
					<Kpi
						label="resultado"
						prefix={sign}
						value={s.resultado}
						color={resultColor}
					/>
					{when !== "past" && (
						<Kpi label="projetado" value={s.projetado} color="var(--muted)" />
					)}
				</div>
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
			borderLeft: "1px solid var(--border)",
			padding: "2px 22px 2px 14px",
			minWidth: 0,
		}}
	>
		<div style={{ ...labelStyle, fontSize: 10, marginBottom: 4 }}>{label}</div>
		<div
			className="mono"
			style={{
				display: "flex",
				alignItems: "baseline",
				fontSize: 15,
				fontWeight: 600,
				color,
				whiteSpace: "nowrap",
			}}
		>
			{prefix}
			<CountMoney value={value} />
		</div>
	</div>
);
