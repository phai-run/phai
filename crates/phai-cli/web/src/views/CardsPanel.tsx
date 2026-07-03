import { useEffect, useMemo, useState } from "react";
import { api, type CardRow } from "../bridge/api";
import { CardGridSkeleton } from "../components/ui";
import { formatMoneyNumber, numeric } from "../lib/format";
import { CardDetailPanel } from "./cards/CardDetailPanel";
import { SkeuomorphicCard } from "./cards/SkeuomorphicCard";

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
					gridTemplateColumns: "repeat(auto-fit, minmax(300px, 360px))",
					justifyContent: "center",
					gap: 22,
					opacity: loading ? 0.55 : 1,
					transition: "opacity 160ms ease",
				}}
			>
				{cards.map((c) => (
					<SkeuomorphicCard
						key={c.accountId}
						card={c}
						flipped={c.accountId === expandedId}
						onToggle={() =>
							setExpandedId((id) => (id === c.accountId ? null : c.accountId))
						}
					/>
				))}
			</div>

			{expanded && (
				<CardDetailPanel
					card={expanded}
					month={month}
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
