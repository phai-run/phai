import { useEffect, useMemo, useState } from "react";
import { api, type CardRow } from "../bridge/api";
import { Card, CardGridSkeleton } from "../components/ui";
import { formatMoneyCompact, formatMoneyNumber, numeric } from "../lib/format";
import { CardDetailPanel } from "./cards/CardDetailPanel";

/**
 * Per-credit-card cycle panel: shows whether each card's bill is open (aberta)
 * or settled (em dia), the current cycle total, the due date, open balance, and
 * credit-limit usage — so the user can see how much hits the cash and when.
 * Cash-flow basis: the cycle total is what leaves the cash in the month the bill
 * is paid (ADR-0025). Clicking a tile opens a full-width detail panel *below*
 * the grid (parcelas breakdown). Data: GET /api/cards.
 *
 * The panel is centred in its own bounded column with a summary band on top;
 * on a month change it keeps the previous cards visible under a light "updating"
 * veil (instead of flashing back to a skeleton) so switching months feels calm.
 */

/** Scan order: open bills first, then closed cycles, settled last. */
const rankState = (s: CardRow["state"]): number =>
	s === "aberta" ? 0 : s === "fechada" ? 1 : 2;

export const CardsPanel = ({
	month,
	onViewCardTx,
}: {
	month: string | null;
	onViewCardTx?: (accountId: string) => void;
}) => {
	const [rows, setRows] = useState<CardRow[] | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const [expandedId, setExpandedId] = useState<string | null>(null);

	useEffect(() => {
		let alive = true;
		setLoading(true);
		setError(null);
		setExpandedId(null);
		api
			.cards(month ?? undefined)
			.then((r) => {
				if (!alive) return;
				setRows(r.rows);
				setLoading(false);
			})
			.catch((e: unknown) => {
				if (!alive) return;
				setError(e instanceof Error ? e.message : String(e));
				setLoading(false);
			});
		return () => {
			alive = false;
		};
	}, [month]);

	// Only real credit cards (a credit limit or an open bill), open bills first
	// then by nearest due date — the order a user scans them in.
	const cards = useMemo(() => {
		if (!rows) return null;
		return rows
			.filter((c) => c.creditLimit != null || c.state === "aberta")
			.sort(
				(a, b) =>
					rankState(a.state) - rankState(b.state) ||
					(a.dueDate ?? "9999").localeCompare(b.dueDate ?? "9999"),
			);
	}, [rows]);

	const summary = useMemo(() => {
		if (!cards) return null;
		let billsTotal = 0;
		let openTotal = 0;
		let endingSoon = 0;
		let usedSum = 0;
		let limitSum = 0;
		for (const c of cards) {
			billsTotal += numeric(c.total);
			if (c.state !== "em-dia") openTotal += numeric(c.total);
			endingSoon += numeric(c.installmentEndingAmount);
			if (c.creditLimit != null) limitSum += numeric(c.creditLimit);
			if (c.usedAmount != null) usedSum += numeric(c.usedAmount);
		}
		return {
			count: cards.length,
			billsTotal,
			openTotal,
			endingSoon,
			usedPct:
				limitSum > 0 ? Math.min(100, Math.round((usedSum / limitSum) * 100)) : null,
		};
	}, [cards]);

	if (error) return null; // non-critical panel; stay silent on failure

	// First load (no data yet) → skeleton.
	if (!cards) {
		return (
			<section style={{ maxWidth: 980, margin: "24px auto 48px" }}>
				<CardsHeader />
				<CardGridSkeleton />
			</section>
		);
	}
	if (cards.length === 0) return null;

	const expanded = cards.find((c) => c.accountId === expandedId) ?? null;

	return (
		<section
			className="fade-in-soft"
			style={{ maxWidth: 980, margin: "24px auto 48px", position: "relative" }}
		>
			<CardsHeader count={summary?.count} loading={loading} />

			{summary && (
				<div
					style={{
						display: "grid",
						gridTemplateColumns: "repeat(auto-fit, minmax(150px, 1fr))",
						gap: 12,
						margin: "0 0 20px",
					}}
				>
					<SummaryKpi label="fatura total" value={summary.billsTotal} strong />
					<SummaryKpi
						label="ainda no caixa"
						value={summary.openTotal}
						accent="var(--amber)"
					/>
					<SummaryKpi
						label="parcelas que encerram"
						value={summary.endingSoon}
						accent="var(--green)"
					/>
					{summary.usedPct != null && (
						<div style={summaryBoxStyle}>
							<div style={summaryLabelStyle}>limite usado</div>
							<div
								className="mono"
								style={{
									fontSize: 20,
									fontWeight: 700,
									marginTop: 2,
									color:
										summary.usedPct >= 90 ? "var(--rose)" : "var(--white)",
								}}
							>
								{summary.usedPct}%
							</div>
						</div>
					)}
				</div>
			)}

			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit, minmax(320px, 440px))",
					justifyContent: "center",
					gap: 18,
					opacity: loading ? 0.55 : 1,
					transition: "opacity 160ms ease",
				}}
			>
				{cards.map((c) => (
					<CardTile
						key={c.accountId}
						card={c}
						expanded={c.accountId === expandedId}
						onToggle={() =>
							setExpandedId((id) => (id === c.accountId ? null : c.accountId))
						}
					/>
				))}
			</div>

			{expanded && (
				<CardDetailPanel
					card={expanded}
					onClose={() => setExpandedId(null)}
					onViewTransactions={onViewCardTx}
				/>
			)}
		</section>
	);
};

const CardsHeader = ({
	count,
	loading,
}: {
	count?: number;
	loading?: boolean;
}) => (
	<div
		style={{
			display: "flex",
			alignItems: "baseline",
			justifyContent: "center",
			gap: 10,
			marginBottom: 16,
		}}
	>
		<h2
			style={{
				fontFamily: "var(--font-display)",
				fontSize: "1.35rem",
				fontWeight: 700,
				letterSpacing: "-0.02em",
				margin: 0,
			}}
		>
			Cartões
		</h2>
		{count != null && (
			<span className="mono" style={{ fontSize: 12, color: "var(--muted)" }}>
				{count}
			</span>
		)}
		{loading && (
			<span
				className="mono"
				style={{ fontSize: 11, color: "var(--muted2)" }}
			>
				atualizando…
			</span>
		)}
	</div>
);

const summaryBoxStyle: React.CSSProperties = {
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-md)",
	padding: "10px 14px",
	background: "var(--surface)",
	minWidth: 0,
};

const summaryLabelStyle: React.CSSProperties = {
	fontFamily: "var(--font-body)",
	fontWeight: 600,
	fontSize: 10,
	letterSpacing: "0.12em",
	textTransform: "uppercase",
	color: "var(--muted)",
};

const SummaryKpi = ({
	label,
	value,
	accent = "var(--white)",
	strong,
}: {
	label: string;
	value: number;
	accent?: string;
	strong?: boolean;
}) => (
	<div style={summaryBoxStyle}>
		<div style={summaryLabelStyle}>{label}</div>
		<div
			className="mono"
			style={{
				fontSize: strong ? 22 : 18,
				fontWeight: 700,
				marginTop: 2,
				color: accent,
			}}
		>
			{formatMoneyNumber(value)}
		</div>
	</div>
);

const cardAccent = (state: CardRow["state"]): string =>
	state === "aberta"
		? "var(--amber)"
		: state === "fechada"
			? "var(--purple)"
			: "var(--green)";

const CardTile = ({
	card,
	expanded,
	onToggle,
}: {
	card: CardRow;
	expanded: boolean;
	onToggle: () => void;
}) => {
	const open = card.state === "aberta";
	const closed = card.state === "fechada";
	const total = numeric(card.total);
	const openAmount = numeric(card.openAmount);
	const installmentDebt = numeric(card.installmentDebt);
	const installmentMonthAmount = numeric(card.installmentMonthAmount);
	const installmentEndingAmount = numeric(card.installmentEndingAmount);
	const limit = card.creditLimit != null ? numeric(card.creditLimit) : null;
	const used = card.usedAmount != null ? numeric(card.usedAmount) : null;
	const usedPct =
		limit && limit > 0 && used != null
			? Math.min(100, Math.round((used / limit) * 100))
			: null;
	const accent = cardAccent(card.state);
	const canExpand = card.installmentCount > 0;

	return (
		<Card accent={accent} selected={expanded} style={{ minWidth: 0 }}>
			<div
				className="lift"
				role="button"
				tabIndex={0}
				aria-expanded={expanded}
				onClick={onToggle}
				onKeyDown={(e) => {
					if (e.key === "Enter" || e.key === " ") {
						e.preventDefault();
						onToggle();
					}
				}}
				style={{ cursor: "pointer", outline: "none" }}
			>
				<div
					style={{
						display: "flex",
						justifyContent: "space-between",
						alignItems: "baseline",
						gap: 8,
					}}
				>
					<span style={{ fontWeight: 600, fontSize: 13 }}>{card.label}</span>
					<span
						className="mono"
						style={{
							fontSize: 10,
							fontWeight: 600,
							color: accent,
							border: `1px solid ${accent}`,
							borderRadius: "var(--radius-full)",
							padding: "1px 8px",
						}}
						title={
							open
								? "Open bill — still leaving your cash"
								: closed
									? "Bill closed for the selected month"
									: "No bill in the selected month"
						}
					>
						{open ? "ABERTA" : closed ? "FECHADA" : "EM DIA"}
					</span>
				</div>
				<div style={{ fontSize: 24, fontWeight: 700, marginTop: 8 }}>
					{formatMoneyNumber(total)}
				</div>
				<div
					className="mono"
					style={{ fontSize: 11, color: "var(--muted)", marginTop: 2 }}
				>
					{card.cycleMonth
						? `ciclo ${card.cycleMonth.slice(5, 7)}/${card.cycleMonth.slice(2, 4)}`
						: "—"}
					{card.dueDate
						? ` · vence ${card.dueDate.slice(8, 10)}/${card.dueDate.slice(5, 7)}`
						: ""}
					{open && openAmount > 0
						? ` · em aberto ${formatMoneyCompact(openAmount)}`
						: ""}
				</div>
				<div
					style={{
						display: "grid",
						gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
						gap: 8,
						marginTop: 12,
					}}
				>
					<CardMetric label="parcelado" value={installmentDebt} />
					<CardMetric label="este mês" value={installmentMonthAmount} />
					<CardMetric label="encerrando" value={installmentEndingAmount} />
				</div>
				{usedPct != null && limit != null && (
					<div style={{ marginTop: 12 }}>
						<div
							style={{
								height: 6,
								background: "var(--border)",
								borderRadius: "var(--radius-full)",
								overflow: "hidden",
							}}
						>
							<div
								style={{
									width: `${usedPct}%`,
									height: "100%",
									background: usedPct >= 90 ? "var(--rose)" : accent,
									transition: "width 200ms ease",
								}}
							/>
						</div>
						<div style={{ fontSize: 10, color: "var(--muted)", marginTop: 4 }}>
							{usedPct}% do limite ({formatMoneyNumber(limit)})
						</div>
					</div>
				)}
				{canExpand && (
					<div
						className="mono"
						style={{
							marginTop: 12,
							fontSize: 11,
							color: "var(--muted)",
							display: "flex",
							justifyContent: "space-between",
						}}
					>
						<span>
							{card.installmentCount} parcela
							{card.installmentCount !== 1 ? "s" : ""}
						</span>
						<span style={{ color: accent }}>
							{expanded ? "▾ fechar" : "▸ detalhes"}
						</span>
					</div>
				)}
			</div>
		</Card>
	);
};

const CardMetric = ({ label, value }: { label: string; value: number }) => (
	<div
		title={formatMoneyNumber(value)}
		style={{
			border: "1px solid var(--border)",
			borderRadius: "var(--radius-sm)",
			padding: "6px 7px",
			minWidth: 0,
		}}
	>
		<div
			className="mono"
			style={{
				fontSize: 9,
				color: "var(--muted)",
				whiteSpace: "nowrap",
				overflow: "hidden",
				textOverflow: "ellipsis",
			}}
		>
			{label}
		</div>
		<div
			className="mono"
			style={{
				fontSize: 11,
				fontWeight: 600,
				color: value > 0 ? "var(--rose)" : "var(--muted)",
				marginTop: 2,
				whiteSpace: "nowrap",
				overflow: "hidden",
				textOverflow: "ellipsis",
			}}
		>
			{formatMoneyCompact(value)}
		</div>
	</div>
);
