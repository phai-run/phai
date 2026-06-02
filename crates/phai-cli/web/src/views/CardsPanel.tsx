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
export const CardsPanel = () => {
	const [rows, setRows] = useState<CardRow[] | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let alive = true;
		api
			.cards()
			.then((r) => {
				if (alive) setRows(r.rows);
			})
			.catch((e: unknown) => {
				if (alive) setError(e instanceof Error ? e.message : String(e));
			});
		return () => {
			alive = false;
		};
	}, []);

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
	const open = card.state === "aberta";
	const total = numeric(card.total);
	const limit = card.creditLimit != null ? numeric(card.creditLimit) : null;
	const used = card.usedAmount != null ? numeric(card.usedAmount) : null;
	const usedPct =
		limit && limit > 0 && used != null
			? Math.min(100, Math.round((used / limit) * 100))
			: null;
	const accent = open ? "var(--amber, #d97706)" : "var(--green, #16a34a)";

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
							: "Sem fatura em aberto"
					}
				>
					{open ? "ABERTA" : "EM DIA"}
				</span>
			</div>
			<div style={{ fontSize: 22, fontWeight: 700, marginTop: 6 }}>
				{formatMoneyNumber(total)}
			</div>
			<div style={{ fontSize: 11, color: "var(--muted)", marginTop: 2 }}>
				{open && card.cycleMonth ? `Ciclo ${card.cycleMonth}` : "—"}
				{open && card.dueDate ? ` · vence ${card.dueDate.slice(8, 10)}/${card.dueDate.slice(5, 7)}` : ""}
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
		</Card>
	);
};
