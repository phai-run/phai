import type { CardRow } from "../../bridge/api";
import { formatMoneyNumber, numeric } from "../../lib/format";

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
	onClose,
}: {
	card: CardRow;
	onClose: () => void;
}) => {
	const accent = cardAccent(card.state);
	const stateLabel =
		card.state === "aberta"
			? "ABERTA"
			: card.state === "fechada"
				? "FECHADA"
				: "EM DIA";

	const figures: Array<{ label: string; value: number; strong?: boolean }> = [
		{ label: "fatura", value: numeric(card.total), strong: true },
		{ label: "em aberto", value: numeric(card.openAmount) },
		{ label: "parcelado", value: numeric(card.installmentDebt) },
		{ label: "este mês", value: numeric(card.installmentMonthAmount) },
		{ label: "termina", value: numeric(card.installmentEndingAmount) },
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
					aria-label="fechar detalhes do cartão"
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
								style={{
									display: "flex",
									justifyContent: "space-between",
									alignItems: "baseline",
									gap: 10,
									padding: "8px 12px",
									borderRadius: "var(--radius-sm)",
									border: "1px solid var(--border)",
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
										{row.endingThisMonth ? " · termina este mês" : ""}
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
		</div>
	);
};
