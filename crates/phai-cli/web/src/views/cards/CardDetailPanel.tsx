import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { useCallback, useMemo, useState } from "react";
import { AnimatePresence } from "framer-motion";
import type { CardRow } from "../../bridge/api";
import { events, tables } from "../../livestore/schema";
import { formatMoneyNumber, numeric } from "../../lib/format";
import {
	buildOverlayMap,
	effectiveCategory,
	sheetLabel,
	type TxView,
} from "../../lib/derivations";
import {
	TransactionModal,
	type ReviewPatch,
} from "../../components/TransactionModal";

const txAll$ = queryDb(tables.transactions);
const overlay$ = queryDb(tables.reviewOverlay);
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));

/**
 * Full-width detail panel for one credit card, rendered *below* the card grid
 * (not inline inside the narrow tile — the chosen expand UX). It restates the
 * cycle figures with room to breathe and lays the installment purchases out in
 * a multi-column grid, highlighting the ones ending this month (cash that frees
 * up next month). "Fechamento" (cycle closing date) is not yet exposed by
 * /api/cards — surfacing it needs a bridge change (tracked separately).
 */

const cardAccent = (state: CardRow["state"]): string =>
	state === "aberta"
		? "var(--amber)"
		: state === "fechada"
			? "var(--purple)"
			: "var(--green)";

const ddmm = (d: string | null): string =>
	d ? `${d.slice(8, 10)}/${d.slice(5, 7)}` : "—";

export const CardDetailPanel = ({
	card,
	month,
	onClose,
	onViewTransactions,
}: {
	card: CardRow;
	/** Selected sheet month ("YYYY-MM"); scopes the "transações do mês" list. */
	month: string | null;
	onClose: () => void;
	onViewTransactions?: (accountId: string) => void;
}) => {
	const accent = cardAccent(card.state);
	const stateLabel =
		card.state === "aberta"
			? "ABERTA"
			: card.state === "fechada"
				? "FECHADA"
				: "EM DIA";

	// Clicking an installment opens the same edit modal as the planilha /
	// categorias views — the row's transactionId maps into the seeded window.
	const { store } = useStore();
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const overlay = useQuery(overlay$);
	const categories = useQuery(categories$);
	const overlayMap = useMemo(() => buildOverlayMap(overlay), [overlay]);
	const categoryIds = useMemo(() => categories.map((c) => c.id), [categories]);
	const txById = useMemo(
		() => new Map(txRows.map((t) => [t.id, t])),
		[txRows],
	);
	const [modalTx, setModalTx] = useState<TxView | null>(null);

	// Every real transaction on this card in the selected month — the "compras do
	// mês" list the expanded card promises (installments are a subset shown above).
	const monthTxs = useMemo(() => {
		if (!month) return [] as TxView[];
		return txRows
			.filter((t) => t.accountId === card.accountId && t.postedAt.slice(0, 7) === month)
			.sort((a, b) => b.postedAt.localeCompare(a.postedAt));
	}, [txRows, card.accountId, month]);

	const similarTxs = useMemo(() => {
		if (!modalTx) return [] as TxView[];
		const cat = effectiveCategory(modalTx, overlayMap);
		return txRows.filter(
			(t) =>
				t.id !== modalTx.id &&
				(effectiveCategory(t, overlayMap) === cat ||
					(t.merchantName && t.merchantName === modalTx.merchantName)),
		);
	}, [modalTx, txRows, overlayMap]);

	const submitModal = useCallback(
		(transactionId: string, patch: ReviewPatch) => {
			store.commit(
				events.reviewSubmitted({
					writeId: crypto.randomUUID(),
					transactionId,
					patch,
					submittedAt: Date.now(),
				}),
			);
			setModalTx(null);
		},
		[store],
	);

	const figures: Array<{ label: string; value: number; strong?: boolean }> = [
		{ label: "fatura", value: numeric(card.total), strong: true },
		{ label: "em aberto", value: numeric(card.openAmount) },
		{ label: "parcelado", value: numeric(card.installmentDebt) },
		{ label: "este mês", value: numeric(card.installmentMonthAmount) },
		{ label: "encerrando", value: numeric(card.installmentEndingAmount) },
	];

	return (
		<div
			style={{
				marginTop: 12,
				border: "1px solid var(--border)",
				borderLeft: `3px solid ${accent}`,
				borderRadius: "var(--radius-lg)",
				background: "var(--surface)",
				padding: "16px 20px 20px",
			}}
		>
			{/* Header */}
			<div
				style={{
					display: "flex",
					alignItems: "baseline",
					gap: 12,
					flexWrap: "wrap",
				}}
			>
				<strong style={{ fontSize: 15 }}>{card.label}</strong>
				<span className="mono" style={{ fontSize: 11, color: accent }}>
					{stateLabel}
				</span>
				<span className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
					{card.cycleMonth
						? `ciclo ${card.cycleMonth.slice(5, 7)}/${card.cycleMonth.slice(2, 4)}`
						: ""}
					{card.dueDate ? ` · vence ${ddmm(card.dueDate)}` : ""}
				</span>
				<button
					type="button"
					onClick={onClose}
					className="mono"
					aria-label="close card details"
					style={{
						marginLeft: "auto",
						background: "transparent",
						border: "none",
						color: "var(--muted)",
						cursor: "pointer",
						fontSize: 13,
					}}
				>
					× fechar
				</button>
			</div>

			{/* Key figures */}
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit, minmax(130px, 1fr))",
					gap: 10,
					marginTop: 14,
					maxWidth: 760,
				}}
			>
				{figures.map((f) => (
					<div
						key={f.label}
						style={{
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-md)",
							padding: "8px 12px",
							background: "var(--bg)",
						}}
					>
						<div
							className="mono"
							style={{ fontSize: 10, color: "var(--muted)", marginBottom: 3 }}
						>
							{f.label}
						</div>
						<div
							className="mono"
							style={{
								fontSize: f.strong ? 18 : 14,
								fontWeight: 600,
								color: f.value > 0 ? "var(--rose)" : "var(--muted)",
							}}
						>
							{formatMoneyNumber(f.value)}
						</div>
					</div>
				))}
			</div>

			{onViewTransactions && (
				<button
					type="button"
					onClick={() => onViewTransactions(card.accountId)}
					className="mono"
					style={{
						marginTop: 14,
						background: "transparent",
						border: `1px solid ${accent}`,
						color: accent,
						borderRadius: "var(--radius-full)",
						padding: "6px 14px",
						cursor: "pointer",
						fontSize: 12,
					}}
				>
					ver transações do cartão →
				</button>
			)}

			{/* Installments */}
			{card.installments.length > 0 && (
				<>
					<div
						className="mono"
						style={{
							fontSize: 11,
							color: "var(--muted)",
							margin: "18px 0 8px",
						}}
					>
						parcelas em aberto ({card.installmentCount})
					</div>
					<div
						style={{
							display: "grid",
							gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))",
							gap: 8,
						}}
					>
						{card.installments.map((row) => (
							<div
								key={row.transactionId}
								role="button"
								tabIndex={0}
								title={
									txById.has(row.transactionId)
										? "editar transação"
										: "fora da janela carregada"
								}
								onClick={() => {
									const tx = txById.get(row.transactionId);
									if (tx) setModalTx(tx);
								}}
								onKeyDown={(e) => {
									if (e.key !== "Enter") return;
									const tx = txById.get(row.transactionId);
									if (tx) setModalTx(tx);
								}}
								style={{
									display: "flex",
									justifyContent: "space-between",
									alignItems: "baseline",
									gap: 10,
									padding: "8px 12px",
									borderRadius: "var(--radius-sm)",
									border: "1px solid var(--border)",
									cursor: txById.has(row.transactionId)
										? "pointer"
										: "default",
									background: row.endingThisMonth
										? "rgba(180,83,9,0.08)"
										: "var(--bg)",
								}}
							>
								<div style={{ minWidth: 0 }}>
									<div
										style={{
											fontSize: 12,
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
										{row.endingThisMonth ? " · encerra este mês" : ""}
									</div>
								</div>
								<span
									className="mono"
									style={{
										fontSize: 12,
										fontWeight: 600,
										color: row.endingThisMonth ? "var(--amber)" : "var(--rose)",
										whiteSpace: "nowrap",
									}}
								>
									{formatMoneyNumber(numeric(row.amount))}
								</span>
							</div>
						))}
					</div>
				</>
			)}

			{/* Transactions on this card in the selected month */}
			{monthTxs.length > 0 && (
				<>
					<div
						className="mono"
						style={{ fontSize: 11, color: "var(--muted)", margin: "18px 0 8px" }}
					>
						transações do mês ({monthTxs.length})
					</div>
					<div
						style={{
							display: "grid",
							gridTemplateColumns: "repeat(auto-fill, minmax(300px, 1fr))",
							gap: 8,
						}}
					>
						{monthTxs.map((tx) => (
							<div
								key={tx.id}
								role="button"
								tabIndex={0}
								title="editar transação"
								onClick={() => setModalTx(tx)}
								onKeyDown={(e) => e.key === "Enter" && setModalTx(tx)}
								style={{
									display: "flex",
									justifyContent: "space-between",
									alignItems: "baseline",
									gap: 10,
									padding: "8px 12px",
									borderRadius: "var(--radius-sm)",
									border: "1px solid var(--border)",
									cursor: "pointer",
									background: "var(--bg)",
								}}
							>
								<div style={{ minWidth: 0 }}>
									<div
										style={{
											fontSize: 12,
											whiteSpace: "nowrap",
											overflow: "hidden",
											textOverflow: "ellipsis",
										}}
									>
										{sheetLabel(tx)}
									</div>
									<div className="mono" style={{ fontSize: 10, color: "var(--muted)", marginTop: 1 }}>
										{ddmm(tx.postedAt)}
										{effectiveCategory(tx, overlayMap)
											? ` · ${effectiveCategory(tx, overlayMap)}`
											: ""}
									</div>
								</div>
								<span
									className="mono"
									style={{
										fontSize: 12,
										fontWeight: 600,
										color: numeric(tx.amount) < 0 ? "var(--rose)" : "var(--green)",
										whiteSpace: "nowrap",
									}}
								>
									{formatMoneyNumber(numeric(tx.amount))}
								</span>
							</div>
						))}
					</div>
				</>
			)}

			<AnimatePresence>
				{modalTx && (
					<TransactionModal
						tx={modalTx}
						overlay={overlayMap.get(modalTx.id)}
						similarTxs={similarTxs}
						overlayById={overlayMap}
						categories={categoryIds}
						onSubmit={(patch) => submitModal(modalTx.id, patch)}
						onClose={() => setModalTx(null)}
					/>
				)}
			</AnimatePresence>
		</div>
	);
};
