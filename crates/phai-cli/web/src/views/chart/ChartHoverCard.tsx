import type React from "react";
import { formatMoneyNumber } from "../../lib/format";
import type { ScenarioMonthItem } from "../../lib/derivations";

/**
 * Floating hover cards for the cash chart — one HTML card per hovered column,
 * replacing the old native `<title>` tooltips (which never fit the content and
 * rendered as unstyled OS bubbles).
 */

// ── Positioning shell ───────────────────────────────────────────────────────

/**
 * Absolute-positioned card anchored to column `index` of `count`. Flips to the
 * left of the column once it sits in the right half of the chart, so the card
 * never overflows the viewport.
 */
export const HoverCardShell = ({
	index,
	count,
	children,
}: {
	index: number;
	count: number;
	children: React.ReactNode;
}) => (
	<div
		style={{
			position: "absolute",
			top: 6,
			left: `${((index + 0.5) / count) * 100}%`,
			transform:
				index < count / 2
					? "translateX(10px)"
					: "translateX(calc(-100% - 10px))",
			pointerEvents: "none",
			zIndex: 30,
			background: "var(--card)",
			border: "1px solid var(--border)",
			borderRadius: "var(--radius-md)",
			boxShadow: "0 8px 28px rgba(0,0,0,0.16)",
			padding: "10px 12px",
			minWidth: 210,
			maxWidth: 300,
		}}
	>
		{children}
	</div>
);

// ── Cash-mode card ──────────────────────────────────────────────────────────

/** Everything the cash-mode hover card shows for one month. */
export interface ChartHoverDatum {
	label: string;
	isFuture: boolean;
	realIn: number;
	fcIn: number;
	realOut: number;
	fcOut: number;
	/** Baseline saldo (projected for future months). */
	balance: number;
	/** Active scenario's projected saldo (null = no scenario / not seeded). */
	scenarioBalance: number | null;
	/** The scenario's changes hitting this month, biggest movers first. */
	scenarioItems: ReadonlyArray<ScenarioMonthItem>;
	/** Category breakdown (expenses mode). */
	cats: ReadonlyArray<{ name: string; mag: number; color: string }>;
}

const MAX_SCENARIO_ITEMS = 6;

const money = formatMoneyNumber;
const signedMoney = (v: number): string =>
	`${v < 0 ? "-" : "+"}${money(Math.abs(v))}`;

const CardHeader = ({ d }: { d: ChartHoverDatum }) => (
	<div className="mono" style={{ fontWeight: 700, marginBottom: 6, fontSize: 12 }}>
		{d.label}
		<span style={{ color: "var(--muted)", fontWeight: 400 }}>
			{" "}
			· {d.isFuture ? "projetado" : "realizado"}
		</span>
	</div>
);

const cellStyle = (muted: boolean): React.CSSProperties => ({
	textAlign: "right",
	color: muted ? "var(--muted)" : "var(--text)",
	fontVariantNumeric: "tabular-nums",
});

/** A tiny colour chip matching the chart series — makes the card double as legend. */
const Swatch = ({ color, dashed }: { color: string; dashed?: boolean }) => (
	<span
		aria-hidden
		style={{
			display: "inline-block",
			width: dashed ? 12 : 9,
			height: dashed ? 0 : 9,
			borderRadius: dashed ? 0 : 2,
			background: dashed ? "transparent" : color,
			border: dashed ? `1.5px dashed ${color}` : "none",
			marginRight: 6,
			verticalAlign: "middle",
		}}
	/>
);

/**
 * One flow row of the aligned table: swatch+label | realized | forecast. The
 * "previsto" cell carries the lighter forecast hue so the card also reads as the
 * chart's legend (real vs. forecast bars).
 */
const FlowCells = ({
	label,
	color,
	fcColor,
	real,
	fc,
}: {
	label: string;
	color: string;
	fcColor: string;
	real: number;
	fc: number;
}) => (
	<>
		<span style={{ color: "var(--text)" }}>
			<Swatch color={color} />
			{label}
		</span>
		<span style={cellStyle(false)}>{money(real)}</span>
		<span style={{ ...cellStyle(true), color: fc > 0 ? fcColor : "var(--muted)" }}>
			{fc > 0 ? `+${money(fc)}` : "—"}
		</span>
	</>
);

const SaldoRow = ({
	label,
	value,
	color,
	swatch,
	dashed,
}: {
	label: string;
	value: number;
	color: string;
	/** Swatch colour (defaults to the text colour); paints the legend chip. */
	swatch?: string;
	dashed?: boolean;
}) => (
	<div
		className="mono"
		style={{
			display: "flex",
			justifyContent: "space-between",
			gap: 16,
			fontSize: 12,
			color,
			fontWeight: 700,
			fontVariantNumeric: "tabular-nums",
		}}
	>
		<span>
			<Swatch color={swatch ?? color} dashed={dashed} />
			{label}
		</span>
		<span>{money(value)}</span>
	</div>
);

/** The scenario section: divider + this month's changes (max 6 + "+n"). */
const ScenarioItemsSection = ({
	items,
}: {
	items: ReadonlyArray<ScenarioMonthItem>;
}) => (
	<div style={{ borderTop: "1px solid var(--border)", marginTop: 6, paddingTop: 6 }}>
		<div
			className="mono"
			style={{
				fontSize: 10,
				color: "var(--cyan)",
				textTransform: "uppercase",
				letterSpacing: "0.06em",
				marginBottom: 4,
			}}
		>
			cenário
		</div>
		<div style={{ display: "grid", gap: 3 }}>
			{items.slice(0, MAX_SCENARIO_ITEMS).map((it, idx) => (
				<div
					key={`${it.changeId}-${idx}`}
					className="mono"
					style={{
						display: "flex",
						justifyContent: "space-between",
						gap: 12,
						fontSize: 11,
					}}
				>
					<span
						style={{
							flex: 1,
							overflow: "hidden",
							textOverflow: "ellipsis",
							whiteSpace: "nowrap",
							color: "var(--muted)",
						}}
					>
						{it.label || "(sem descrição)"}
					</span>
					<span
						style={{
							color: it.delta < 0 ? "var(--rose)" : "var(--green)",
							fontVariantNumeric: "tabular-nums",
						}}
					>
						{signedMoney(it.delta)}
					</span>
				</div>
			))}
			{items.length > MAX_SCENARIO_ITEMS && (
				<span className="mono" style={{ fontSize: 10, color: "var(--muted)" }}>
					+{items.length - MAX_SCENARIO_ITEMS} mais
				</span>
			)}
		</div>
	</div>
);

/**
 * The cash-mode hover card: header ("mês · realizado|projetado"), an aligned
 * mono table (entradas/saídas split into realized + forecast), the baseline
 * saldo, and — with an active scenario — the scenario saldo in teal plus the
 * scenario's changes for that month.
 */
export const CashHoverCard = ({ d }: { d: ChartHoverDatum }) => (
	<div>
		<CardHeader d={d} />
		<div
			className="mono"
			style={{
				display: "grid",
				gridTemplateColumns: "auto 1fr 1fr",
				columnGap: 12,
				rowGap: 3,
				fontSize: 11,
				marginBottom: 6,
			}}
		>
			<span />
			<span style={{ ...cellStyle(true), fontSize: 9, textTransform: "uppercase" }}>
				real
			</span>
			<span style={{ ...cellStyle(true), fontSize: 9, textTransform: "uppercase" }}>
				previsto
			</span>
			<FlowCells
				label="entradas"
				color="var(--cyan)"
				fcColor="#99f6e4"
				real={d.realIn}
				fc={d.fcIn}
			/>
			<FlowCells
				label="saídas"
				color="var(--rose)"
				fcColor="#fda4af"
				real={d.realOut}
				fc={d.fcOut}
			/>
		</div>
		<SaldoRow
			label={d.isFuture ? "saldo projetado" : "saldo"}
			value={d.balance}
			color="var(--text)"
			swatch="var(--purple)"
			dashed
		/>
		{d.scenarioBalance != null && (
			<SaldoRow
				label="saldo com cenário"
				value={d.scenarioBalance}
				color="var(--cyan)"
				dashed
			/>
		)}
		{d.scenarioItems.length > 0 && <ScenarioItemsSection items={d.scenarioItems} />}
	</div>
);

// ── Expenses-mode card ──────────────────────────────────────────────────────

/** Column card for the expenses mode: month total + category breakdown. */
export const ExpensesHoverCard = ({ d }: { d: ChartHoverDatum }) => {
	const total = d.realOut + d.fcOut;
	return (
		<div>
			<CardHeader d={d} />
			<div style={{ display: "grid", gap: 4 }}>
				<div
					className="mono"
					style={{
						display: "flex",
						justifyContent: "space-between",
						fontSize: 12,
						color: "var(--rose)",
						fontWeight: 600,
					}}
				>
					<span>despesas</span>
					<span>{money(total)}</span>
				</div>
				{d.cats.slice(0, 8).map((c) => (
					<div
						key={c.name}
						className="mono"
						style={{
							display: "flex",
							alignItems: "center",
							gap: 6,
							fontSize: 11,
							color: "var(--muted)",
						}}
					>
						<span
							aria-hidden
							style={{
								width: 8,
								height: 8,
								borderRadius: 2,
								background: c.color,
								flexShrink: 0,
							}}
						/>
						<span
							style={{
								flex: 1,
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
							}}
						>
							{c.name}
						</span>
						<span style={{ color: "var(--text)" }}>{money(c.mag)}</span>
						<span style={{ width: 34, textAlign: "right" }}>
							{total > 0 ? Math.round((c.mag / total) * 100) : 0}%
						</span>
					</div>
				))}
			</div>
		</div>
	);
};

// ── Per-segment card (expenses mode) ────────────────────────────────────────

/** One category segment + its top subcategories, for the segment hover. */
export const SegmentHoverCard = ({
	cat,
	color,
	value,
	monthLabel,
	subs,
}: {
	cat: string;
	color: string;
	value: number;
	monthLabel: string;
	subs: ReadonlyArray<{ sub: string; mag: number }>;
}) => (
	<div>
		<div
			className="mono"
			style={{
				display: "flex",
				alignItems: "center",
				gap: 6,
				fontWeight: 700,
				fontSize: 12,
				marginBottom: 2,
			}}
		>
			<span
				aria-hidden
				style={{
					width: 9,
					height: 9,
					borderRadius: 2,
					background: color,
					flexShrink: 0,
				}}
			/>
			<span style={{ flex: 1 }}>{cat}</span>
			<span>{money(value)}</span>
		</div>
		<div className="mono" style={{ fontSize: 10, color: "var(--muted)", marginBottom: 6 }}>
			{monthLabel}
		</div>
		{subs.length > 0 && (
			<div style={{ display: "grid", gap: 3 }}>
				{subs.map((s) => (
					<div
						key={s.sub}
						className="mono"
						style={{
							display: "flex",
							justifyContent: "space-between",
							gap: 12,
							fontSize: 11,
							color: "var(--muted)",
						}}
					>
						<span
							style={{
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
							}}
						>
							↳ {s.sub}
						</span>
						<span style={{ color: "var(--text)" }}>{money(s.mag)}</span>
					</div>
				))}
			</div>
		)}
	</div>
);
