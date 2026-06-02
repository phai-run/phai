import { useEffect, useState } from "react";
import { api, type CardRow } from "../bridge/api";
import { Card } from "../components/ui";
import { formatMoneyNumber, numeric } from "../lib/format";

/**
 * Per-credit-card cycle panel: shows whether each card's bill is open (aberta)
 * or settled (em dia), the current cycle total, the due date, and credit-limit
 * usage — so the user can compare the card total against their spending goal
 * and track the bill. Cash-flow basis: the cycle total is what will leave the
 * cash in the month the bill is paid (ADR-0025). Data: GET /api/cards.
 */
export const CardsPanel = ({ month }: { month: string | null }) => {
	const [rows, setRows] = useState<CardRow[] | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let alive = true;
		setRows(null);
		setError(null);
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
	const cards = rows.filter((c) => c.creditLimit != null || c.state === "aberta");
	if (cards.length === 0) return null;

	return (
		<section style={{ marginTop: 24 }}>
			<h2 style={{ fontSize: 14, color: "var(--muted)", margin: "0 0 12px" }}>
				Cartões
			</h2>
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))",
					gap: 12,
				}}
			>
				{cards.map((c) => (
					<CardTile key={c.accountId} card={c} />
				))}
			</div>
		</section>
	);
};

const CardTile = ({ card }: { card: CardRow }) => {
	const [expanded, setExpanded] = useState(false);
	const open = card.state === "aberta";
	const closed = card.state === "fechada";
	const total = numeric(card.total);
	const installmentDebt = numeric(card.installmentDebt);
	const installmentMonthAmount = numeric(card.installmentMonthAmount);
	const installmentEndingAmount = numeric(card.installmentEndingAmount);
	const limit = card.creditLimit != null ? numeric(card.creditLimit) : null;
	const used = card.usedAmount != null ? numeric(card.usedAmount) : null;
	const usedPct =
		limit && limit > 0 && used != null
			? Math.min(100, Math.round((used / limit) * 100))
			: null;
	const accent = open
		? "var(--amber, #d97706)"
		: closed
			? "var(--purple)"
			: "var(--green, #16a34a)";

	return (
		<Card accent={accent}>
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
							? "Fatura em aberto — ainda vai sair do caixa"
							: closed
								? "Fatura fechada para o mês selecionado"
								: "Sem fatura no mês selecionado"
					}
				>
					{open ? "ABERTA" : closed ? "FECHADA" : "EM DIA"}
				</span>
			</div>
			<div style={{ fontSize: 22, fontWeight: 700, marginTop: 6 }}>
				{formatMoneyNumber(total)}
			</div>
			<div style={{ fontSize: 11, color: "var(--muted)", marginTop: 2 }}>
				{card.cycleMonth ? `Ciclo ${card.cycleMonth}` : "—"}
				{card.dueDate ? ` · vence ${card.dueDate.slice(8, 10)}/${card.dueDate.slice(5, 7)}` : ""}
			</div>
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
					gap: 8,
					marginTop: 10,
				}}
			>
				<CardMetric label="parcelado" value={installmentDebt} />
				<CardMetric label="este mês" value={installmentMonthAmount} />
				<CardMetric label="termina" value={installmentEndingAmount} />
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
						{usedPct}% do limite ({formatMoneyNumber(limit)})
					</div>
				</div>
			)}
			{card.installmentCount > 0 && (
				<div style={{ marginTop: 10 }}>
					<button
						type="button"
						className="mono"
						onClick={() => setExpanded((v) => !v)}
						style={{
							width: "100%",
							border: "1px solid var(--border)",
							background: "transparent",
							borderRadius: "var(--radius-sm)",
							padding: "5px 8px",
							cursor: "pointer",
							color: "var(--muted)",
							fontSize: 11,
							textAlign: "left",
						}}
					>
						{expanded ? "▾" : "▸"} {card.installmentCount} parcela
						{card.installmentCount !== 1 ? "s" : ""}
					</button>
					{expanded && (
						<div
							style={{
								marginTop: 6,
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
								overflow: "hidden",
							}}
						>
							{card.installments.map((row, idx) => (
								<div
									key={row.transactionId}
									style={{
										display: "grid",
										gridTemplateColumns: "1fr auto",
										gap: 8,
										padding: "7px 8px",
										borderTop:
											idx > 0 ? "1px solid var(--border)" : "none",
										background: row.endingThisMonth
											? "rgba(245,158,11,0.10)"
											: "transparent",
									}}
								>
									<div style={{ minWidth: 0 }}>
										<div
											style={{
												fontSize: 11,
												whiteSpace: "nowrap",
												overflow: "hidden",
												textOverflow: "ellipsis",
											}}
										>
											{row.description}
										</div>
										<div
											className="mono"
											style={{
												fontSize: 10,
												color: row.endingThisMonth
													? "var(--amber)"
													: "var(--muted)",
												marginTop: 1,
											}}
										>
											{row.marker}
											{row.endingThisMonth ? " · termina este mês" : ""}
										</div>
									</div>
									<span
										className="mono"
										style={{
											fontSize: 11,
											fontWeight: 600,
											color: row.endingThisMonth
												? "var(--amber)"
												: "var(--rose)",
											whiteSpace: "nowrap",
										}}
									>
										{formatMoneyNumber(numeric(row.amount))}
									</span>
								</div>
							))}
						</div>
					)}
				</div>
			)}
		</Card>
	);
};

const CardMetric = ({ label, value }: { label: string; value: number }) => (
	<div
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
			}}
		>
			{formatMoneyNumber(value)}
		</div>
	</div>
);
