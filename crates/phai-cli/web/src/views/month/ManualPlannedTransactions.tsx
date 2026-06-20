import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { useMemo, useState } from "react";
import { events, tables } from "../../livestore/schema";
import { formatMoney, amountColor, isNegative, toCents } from "../../lib/format";
import { isBudgetEnvelope, sheetLabel, type TxView } from "../../lib/derivations";
import type { ForecastView } from "../types";

const forecasts$ = queryDb(tables.forecasts.orderBy("dueDate", "asc"));
const accounts$ = queryDb(tables.accounts.orderBy("label", "asc"));
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));
const txs$ = queryDb(tables.transactions.orderBy("postedAt", "desc"));

const STATUS_LABEL: Record<string, string> = {
	ativo: "Previsto",
	active: "Previsto",
	realizado: "Efetivado",
	descartado: "Excluido",
};

const isManualPlanned = (forecast: ForecastView): boolean =>
	forecast.kind === "manual" &&
	!isBudgetEnvelope(forecast) &&
	!["descartado", "inativo"].includes(forecast.status);

const metaString = (meta: Record<string, unknown>, key: string): string | null => {
	const value = meta[key];
	return typeof value === "string" && value.trim() ? value : null;
};

const monthOf = (date: string | null): string | null =>
	date && date.length >= 7 ? date.slice(0, 7) : null;

const predictedAmount = (forecast: ForecastView): string =>
	metaString(forecast.metadataJson, "predicted_amount") ?? forecast.amount;

const realizedAmount = (forecast: ForecastView): string | null =>
	metaString(forecast.metadataJson, "realized_amount") ??
	(forecast.status === "realizado" ? forecast.amount : null);

const scoreCandidate = (forecast: ForecastView, tx: TxView): number => {
	const amountGap = Math.abs(Math.abs(toCents(tx.amount)) - Math.abs(toCents(forecast.amount)));
	const due = Date.parse(forecast.dueDate ?? `${forecast.month ?? tx.month}-01`);
	const posted = Date.parse(tx.postedAt);
	const dateGap = Number.isFinite(due) && Number.isFinite(posted) ? Math.abs(due - posted) : 0;
	return amountGap + dateGap / 86400000;
};

export const ManualPlannedTransactions = ({ month }: { month: string }) => {
	const { store } = useStore();
	const forecastsRaw = useQuery(forecasts$);
	const accounts = useQuery(accounts$);
	const categories = useQuery(categories$);
	const txRows = useQuery(txs$) as ReadonlyArray<TxView>;

	const forecasts = useMemo(
		() =>
			(forecastsRaw as ReadonlyArray<Omit<ForecastView, "month">>).map((forecast) => ({
				...forecast,
				month: monthOf(forecast.dueDate),
			})).filter((forecast) => forecast.month === month && isManualPlanned(forecast)),
		[forecastsRaw, month],
	);
	const accountMap = useMemo(
		() => new Map(accounts.map((account) => [account.id, account.label])),
		[accounts],
	);
	const [open, setOpen] = useState(false);
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [isOutflow, setIsOutflow] = useState(true);
	const [accountId, setAccountId] = useState("");
	const [categoryId, setCategoryId] = useState("");
	const [settlingId, setSettlingId] = useState<string | null>(null);

	const submit = () => {
		const desc = description.trim();
		const mag = amount.replace(/^-/, "").trim();
		if (!desc || !mag) return;
		store.commit(
			events.forecastCreated({
				writeId: crypto.randomUUID(),
				description: desc,
				amount: isOutflow ? `-${mag}` : mag,
				dueDate: `${month}-01`,
				categoryId: categoryId || null,
				accountId: accountId || null,
				uiRole: "planned_transaction",
				createdAt: Date.now(),
			}),
		);
		setDescription("");
		setAmount("");
		setAccountId("");
		setCategoryId("");
		setOpen(false);
	};

	return (
		<section
			aria-label="Transacoes manuais planejadas"
			style={{
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-md)",
				background: "var(--card)",
				padding: 14,
				margin: "12px 0 16px",
			}}
		>
			<div
				style={{
					display: "flex",
					justifyContent: "space-between",
					alignItems: "center",
					gap: 12,
					flexWrap: "wrap",
				}}
			>
				<div>
					<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
						planejamento manual
					</div>
					<div style={{ fontSize: 14 }}>
						Receitas e despesas previstas para {month}
					</div>
				</div>
				<div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
					<button onClick={() => {
						setIsOutflow(true);
						setOpen((value) => !value);
					}} className="mono" style={actionBtn("var(--rose)")}>
						+ despesa
					</button>
					<button onClick={() => {
						setIsOutflow(false);
						setOpen((value) => !value);
					}} className="mono" style={actionBtn("var(--green)")}>
						+ receita
					</button>
				</div>
			</div>

			{open ? (
				<div
					style={{
						display: "grid",
						gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
						gap: 10,
						marginTop: 12,
					}}
				>
					<input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="descricao" style={inputStyle} />
					<input value={amount} onChange={(e) => setAmount(e.target.value)} placeholder={isOutflow ? "120.00" : "2500.00"} style={inputStyle} />
					<select value={accountId} onChange={(e) => setAccountId(e.target.value)} style={inputStyle}>
						<option value="">conta opcional</option>
						{accounts.map((account) => (
							<option key={account.id} value={account.id}>
								{account.label}
							</option>
						))}
					</select>
					<input
						value={categoryId}
						onChange={(e) => setCategoryId(e.target.value)}
						list="manual-plan-categories"
						placeholder="categoria opcional"
						style={inputStyle}
					/>
					<button onClick={submit} className="mono" style={primaryBtn}>
						Salvar previsao
					</button>
				</div>
			) : null}

			<datalist id="manual-plan-categories">
				{categories.map((category) => (
					<option key={category.id} value={category.id} />
				))}
			</datalist>

			<div style={{ display: "flex", flexDirection: "column", gap: 10, marginTop: 14 }}>
				{forecasts.length === 0 ? (
					<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
						Nenhuma transacao manual planejada neste mes.
					</div>
				) : (
					forecasts.map((forecast) => {
						const linkedAmount = realizedAmount(forecast);
						const linkedTxs = txRows
							.filter((tx) => tx.month === month)
							.filter((tx) => isNegative(tx.amount) === isNegative(forecast.amount))
							.filter((tx) => !forecast.accountId || tx.accountId === forecast.accountId)
							.sort((left, right) => scoreCandidate(forecast, left) - scoreCandidate(forecast, right))
							.slice(0, 6);
						return (
							<div
								key={forecast.forecastId}
								style={{
									border: "1px solid var(--border)",
									borderRadius: "var(--radius-md)",
									padding: 12,
									background:
										forecast.status === "realizado" ? "rgba(22,163,74,0.06)" : "transparent",
								}}
							>
								<div
									style={{
										display: "flex",
										justifyContent: "space-between",
										alignItems: "flex-start",
										gap: 12,
										flexWrap: "wrap",
									}}
								>
									<div style={{ minWidth: 0 }}>
										<div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
											<strong style={{ fontSize: 14 }}>{forecast.description}</strong>
											<span style={statusPill(forecast.status)}>{STATUS_LABEL[forecast.status] ?? forecast.status}</span>
											{forecast.status === "realizado" ? (
												<span style={statusPill("done")}>forecast manual efetivado</span>
											) : (
												<span style={statusPill("pending")}>previsao</span>
											)}
										</div>
										<div className="mono" style={{ fontSize: 11, color: "var(--muted)", marginTop: 4 }}>
											{forecast.accountId ? accountMap.get(forecast.accountId) ?? forecast.accountId : "sem conta"} · {forecast.categoryId ?? "sem categoria"} · {forecast.dueDate ?? month}
										</div>
									</div>
									<div style={{ textAlign: "right" }}>
										<div className="mono" style={{ color: amountColor(forecast.amount), fontSize: 14 }}>
											{formatMoney(forecast.amount)}
										</div>
										{linkedAmount && linkedAmount !== predictedAmount(forecast) ? (
											<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
												prev. {formatMoney(predictedAmount(forecast))} → real {formatMoney(linkedAmount)}
											</div>
										) : null}
									</div>
								</div>

								<div style={{ display: "flex", gap: 8, flexWrap: "wrap", marginTop: 10 }}>
									{forecast.status !== "realizado" ? (
										<button
											onClick={() =>
												setSettlingId((current) =>
													current === forecast.forecastId ? null : forecast.forecastId,
												)
											}
											className="mono"
											style={secondaryBtn}
										>
											Marcar como pago
										</button>
									) : null}
									<button
										onClick={() =>
											store.commit(
												events.forecastDeleted({
													writeId: crypto.randomUUID(),
													forecastId: forecast.forecastId,
													deletedAt: Date.now(),
												}),
											)
										}
										className="mono"
										style={secondaryBtn}
									>
										Excluir
									</button>
								</div>

								{settlingId === forecast.forecastId && forecast.status !== "realizado" ? (
									<div style={{ marginTop: 10, display: "grid", gap: 8 }}>
										<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
											Escolha a transacao real sincronizada para efetivar esta previsao.
										</div>
										{linkedTxs.length === 0 ? (
											<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
												Nenhuma candidata no mes atual com sinal compatível.
											</div>
										) : (
											linkedTxs.map((tx) => (
												<button
													key={tx.id}
													onClick={() => {
														store.commit(
															events.forecastSettled({
																writeId: crypto.randomUUID(),
																forecastId: forecast.forecastId,
																transactionId: tx.id,
																predictedAmount: predictedAmount(forecast),
																actualAmount: tx.amount,
																actualDate: tx.postedAt.slice(0, 10),
																actualDescription: sheetLabel(tx),
																settledAt: new Date().toISOString(),
																settledAtMs: Date.now(),
															}),
														);
														setSettlingId(null);
													}}
													className="mono"
													style={candidateBtn}
												>
													<span>{sheetLabel(tx)}</span>
													<span>{formatMoney(tx.amount)}</span>
												</button>
											))
										)}
									</div>
								) : null}

								<details style={{ marginTop: 10 }}>
									<summary className="mono" style={{ cursor: "pointer", color: "var(--muted)", fontSize: 11 }}>
										Propriedades
									</summary>
									<div className="mono" style={{ fontSize: 11, color: "var(--muted)", display: "grid", gap: 4, marginTop: 8 }}>
										<div>forecast_id: {forecast.forecastId}</div>
										<div>status: {forecast.status}</div>
										<div>valor_previsto: {predictedAmount(forecast)}</div>
										<div>valor_efetivado: {linkedAmount ?? "—"}</div>
										<div>realized_transaction_id: {forecast.realizedTransactionId ?? "—"}</div>
										<div>realized_at: {forecast.realizedAt ?? "—"}</div>
									</div>
								</details>
							</div>
						);
					})
				)}
			</div>
		</section>
	);
};

const inputStyle = {
	background: "var(--bg)",
	color: "var(--text)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "8px 10px",
	fontSize: 13,
};

const primaryBtn = {
	background: "var(--text)",
	color: "var(--bg)",
	border: "1px solid var(--text)",
	borderRadius: "var(--radius-full)",
	padding: "8px 12px",
	cursor: "pointer",
};

const secondaryBtn = {
	background: "transparent",
	color: "var(--text)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "6px 10px",
	cursor: "pointer",
	fontSize: 11,
};

const candidateBtn = {
	display: "flex",
	justifyContent: "space-between",
	gap: 12,
	background: "transparent",
	color: "var(--text)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "8px 10px",
	cursor: "pointer",
	textAlign: "left" as const,
};

const actionBtn = (color: string) => ({
	background: "transparent",
	color,
	border: `1px solid ${color}`,
	borderRadius: "var(--radius-full)",
	padding: "6px 12px",
	cursor: "pointer",
	fontSize: 11,
});

const statusPill = (status: string) => ({
	background:
		status === "realizado" || status === "done"
			? "rgba(22,163,74,0.12)"
			: "rgba(245,158,11,0.12)",
	color:
		status === "realizado" || status === "done"
			? "var(--green)"
			: "var(--amber)",
	border: "1px solid transparent",
	borderRadius: "var(--radius-full)",
	padding: "2px 8px",
	fontSize: 11,
});
