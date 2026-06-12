import { useEffect, useState } from "react";
import { api, type CardRow } from "../bridge/api";
import { Card } from "../components/ui";
import { formatMoneyCompact, formatMoneyNumber, numeric } from "../lib/format";
import { CardDetailPanel } from "./cards/CardDetailPanel";

/**
 * Per-credit-card cycle panel: shows whether each card's bill is open (aberta)
 * or settled (em dia), the current cycle total, the due date, open balance, and
 * credit-limit usage — so the user can see how much hits the cash and when.
 * Cash-flow basis: the cycle total is what leaves the cash in the month the bill
 * is paid (ADR-0025). Clicking a tile opens a full-width detail panel *below*
 * the grid (parcelas breakdown). Data: GET /api/cards.
 */
export const CardsPanel = ({
	month,
	onViewCardTx,
}: {
	month: string | null;
	onViewCardTx?: (accountId: string) => void;
}) => {
	const [rows, setRows] = useState<CardRow[] | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [expandedId, setExpandedId] = useState<string | null>(null);

	useEffect(() => {
		let alive = true;
		setRows(null);
		setError(null);
		setExpandedId(null);
		api
			.cards(month ?? undefined)
			.then((r) => {
				if (alive) setRows(r.rows);
			})
			.catch((e: unknown) => {
				if (alive) setError(e instanceof Error ? e.message : String(e));
			});
		return () => {
			alive = false;
		};
	}, [month]);

	if (error) return null; // non-critical panel; stay silent on failure
	if (!rows) return null;
	// Only show real credit cards (those with a credit limit or an open bill).
	const cards = rows.filter(
		(c) => c.creditLimit != null || c.state === "aberta",
	);
	if (cards.length === 0) return null;
	const expanded = cards.find((c) => c.accountId === expandedId) ?? null;

	return (
		<section style={{ marginTop: 24 }}>
			<h2 style={{ fontSize: 14, color: "var(--muted)", margin: "0 0 12px" }}>
				Cards
			</h2>
			<div
				style={{
					display: "grid",
					// Only a couple of cards — give them real width (not 240px
					// slivers) and left-align so values never truncate.
					gridTemplateColumns: "repeat(auto-fill, minmax(340px, 460px))",
					justifyContent: "start",
					gap: 16,
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
		<Card accent={accent} selected={expanded}>
			<div
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
						style={{ fontSize: 11, color: accent }}
						title={
							open
								? "Open bill — still leaving your cash"
								: closed
									? "Bill closed for the selected month"
									: "No bill in the selected month"
						}
					>
						{open ? "OPEN" : closed ? "CLOSED" : "SETTLED"}
					</span>
				</div>
				<div style={{ fontSize: 22, fontWeight: 700, marginTop: 6 }}>
					{formatMoneyNumber(total)}
				</div>
				<div
					className="mono"
					style={{ fontSize: 11, color: "var(--muted)", marginTop: 2 }}
				>
					{card.cycleMonth
						? `cycle ${card.cycleMonth.slice(5, 7)}/${card.cycleMonth.slice(2, 4)}`
						: "—"}
					{card.dueDate
						? ` · due ${card.dueDate.slice(8, 10)}/${card.dueDate.slice(5, 7)}`
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
						marginTop: 10,
					}}
				>
					<CardMetric label="installments" value={installmentDebt} />
					<CardMetric label="this month" value={installmentMonthAmount} />
					<CardMetric label="ending" value={installmentEndingAmount} />
				</div>
				{usedPct != null && limit != null && (
					<div style={{ marginTop: 10 }}>
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
									background: usedPct >= 90 ? "var(--red, #dc2626)" : accent,
								}}
							/>
						</div>
						<div style={{ fontSize: 10, color: "var(--muted)", marginTop: 4 }}>
							{usedPct}% of limit ({formatMoneyNumber(limit)})
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
							{card.installmentCount} installment
							{card.installmentCount !== 1 ? "s" : ""}
						</span>
						<span style={{ color: accent }}>
							{expanded ? "▾ close" : "▸ details"}
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
