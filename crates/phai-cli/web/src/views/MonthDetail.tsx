import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useEffect, useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { events, tables } from "../livestore/schema";
import {
	amountColor,
	formatMoney,
	formatMoneyNumber,
	isNegative,
	sumAmounts,
} from "../lib/format";
import { useDnd } from "../lib/dnd";
import type { ChartMonthView, ForecastView } from "./types";

// ── LiveStore queries (module-level for stable refs) ──────────────────────
const txAll$ = queryDb(tables.transactions.orderBy("postedAt", "desc"));
const overlay$ = queryDb(tables.reviewOverlay);
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));
const accounts$ = queryDb(tables.accounts.orderBy("label", "asc"));

// ── Types ──────────────────────────────────────────────────────────────────

interface TxView {
	id: string;
	accountId: string;
	postedAt: string;
	amount: string;
	rawDescription: string;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
	month: string;
	paymentStatus: string;
	reviewed: number;
	isInstallment: number;
	isSubscription: number;
}

interface ReviewPatch {
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
}

// ── Main component ─────────────────────────────────────────────────────────

export const MonthDetail = ({
	month,
	chart,
	forecasts,
	onForecastAdded,
}: {
	month: string;
	chart: ChartMonthView | null;
	forecasts: ForecastView[];
	onForecastAdded: () => void;
}) => {
	const { store } = useStore();
	const [ui, setUi] = useClientDocument(tables.ui);
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const overlay = useQuery(overlay$);
	const categories = useQuery(categories$);
	const accounts = useQuery(accounts$);

	const overlayById = useMemo(
		() => new Map(overlay.map((o) => [o.transactionId, o])),
		[overlay],
	);
	const accountById = useMemo(
		() => new Map(accounts.map((a) => [a.id, a])),
		[accounts],
	);
	const categoryIds = useMemo(() => categories.map((c) => c.id), [categories]);
	const owners = useMemo(
		() => Array.from(new Set(accounts.map((a) => a.owner).filter(Boolean))),
		[accounts],
	);

	// Effective category (overlay first, then seed)
	const effectiveCat = (tx: TxView) =>
		overlayById.get(tx.id)?.categoryId ?? tx.categoryId;

	// Transactions for this month
	const monthTxs = useMemo(
		() => txRows.filter((t) => t.month === month),
		[txRows, month],
	);

	// Apply filters
	const filtered = useMemo(() => {
		const cat = ui.categoryFilter?.trim().toLowerCase() ?? null;
		const text = ui.textFilter?.trim().toLowerCase() ?? null;
		return monthTxs.filter((tx) => {
			if (ui.installmentsOnly && !tx.isInstallment) return false;
			if (ui.subscriptionsOnly && !tx.isSubscription) return false;
			if (ui.unreviewedOnly && tx.reviewed) return false;
			if (ui.accountFilter && tx.accountId !== ui.accountFilter) return false;
			if (ui.ownerFilter) {
				if ((accountById.get(tx.accountId)?.owner ?? "") !== ui.ownerFilter)
					return false;
			}
			if (cat) {
				if (!(effectiveCat(tx) ?? "").toLowerCase().includes(cat)) return false;
			}
			if (text) {
				const haystack = [
					tx.description,
					tx.merchantName,
					tx.rawDescription,
					effectiveCat(tx),
				]
					.filter(Boolean)
					.join(" ")
					.toLowerCase();
				if (!haystack.includes(text)) return false;
			}
			return true;
		});
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [monthTxs, overlayById, accountById, ui]);

	// Group by income / expense-by-category
	const groups = useMemo(() => {
		const income: TxView[] = [];
		const expMap = new Map<string, TxView[]>();
		for (const tx of filtered) {
			if (!isNegative(tx.amount)) {
				income.push(tx);
			} else {
				const cat = effectiveCat(tx) ?? "—";
				if (!expMap.has(cat)) expMap.set(cat, []);
				expMap.get(cat)!.push(tx);
			}
		}
		// Sort expense categories by absolute sum desc
		const expEntries = Array.from(expMap.entries()).sort((a, b) => {
			const sumA = Math.abs(sumAmounts(a[1].map((t) => t.amount)));
			const sumB = Math.abs(sumAmounts(b[1].map((t) => t.amount)));
			return sumB - sumA;
		});
		return { income, expEntries };
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [filtered, overlayById]);

	// Filter sums
	const sums = useMemo(() => {
		const out = filtered.filter((t) => isNegative(t.amount)).map((t) => t.amount);
		const inc = filtered.filter((t) => !isNegative(t.amount)).map((t) => t.amount);
		return { saidas: Math.abs(sumAmounts(out)), entradas: sumAmounts(inc) };
	}, [filtered]);

	// Month summary (all month transactions, no filter)
	const monthSums = useMemo(() => {
		const out = monthTxs.filter((t) => isNegative(t.amount)).map((t) => t.amount);
		const inc = monthTxs.filter((t) => !isNegative(t.amount)).map((t) => t.amount);
		return { saidas: Math.abs(sumAmounts(out)), entradas: sumAmounts(inc) };
	}, [monthTxs]);

	// Modal state
	const [modalTx, setModalTx] = useState<TxView | null>(null);

	const submit = (transactionId: string, patch: ReviewPatch) => {
		store.commit(
			events.reviewSubmitted({
				writeId: crypto.randomUUID(),
				transactionId,
				patch,
				submittedAt: Date.now(),
			}),
		);
	};

	const hasFilters =
		ui.installmentsOnly ||
		ui.subscriptionsOnly ||
		ui.unreviewedOnly ||
		!!ui.accountFilter ||
		!!ui.ownerFilter ||
		!!ui.categoryFilter ||
		!!ui.textFilter;

	// Installment stats for the month
	const installments = useMemo(
		() => monthTxs.filter((t) => t.isInstallment === 1),
		[monthTxs],
	);
	const installmentSum = Math.abs(
		sumAmounts(installments.map((t) => t.amount)),
	);

	const projectedClose = chart
		? chart.isFuture
			? Number(chart.projectedClosingBalance)
			: Number(chart.closingBalance)
		: null;

	return (
		<div style={{ paddingBottom: 80 }}>
			{/* ── Month summary strip ── */}
			<MonthSummary
				month={month}
				isFuture={chart?.isFuture === 1}
				entradas={monthSums.entradas}
				saidas={monthSums.saidas}
				projectedClose={projectedClose}
				forecastCount={forecasts.length}
				installmentCount={installments.length}
				installmentSum={installmentSum}
			/>

			{/* ── Forecasts section ── */}
			{forecasts.length > 0 || true ? (
				<ForecastSection
					month={month}
					forecasts={forecasts}
					onAdded={onForecastAdded}
				/>
			) : null}

			{/* ── Filter bar ── */}
			<FilterBar
				ui={ui}
				setUi={setUi}
				owners={owners}
				accounts={accounts}
				hasFilters={hasFilters}
			/>

			{/* ── Filter summary strip ── */}
			<AnimatePresence>
				{hasFilters && (
					<motion.div
						initial={{ opacity: 0, height: 0 }}
						animate={{ opacity: 1, height: "auto" }}
						exit={{ opacity: 0, height: 0 }}
						style={{ overflow: "hidden" }}
					>
						<FilterSummary
							count={filtered.length}
							saidas={sums.saidas}
							entradas={sums.entradas}
						/>
					</motion.div>
				)}
			</AnimatePresence>

			{/* ── Transaction groups ── */}
			<div style={{ marginTop: 20 }}>
				{/* Income */}
				{groups.income.length > 0 && (
					<CategoryGroup
						label="Entradas"
						isIncome
						txs={groups.income}
						overlayById={overlayById}
						onEdit={setModalTx}
					/>
				)}

				{/* Expenses by category */}
				{groups.expEntries.map(([cat, txs]) => (
					<CategoryGroup
						key={cat}
						label={cat}
						isIncome={false}
						txs={txs}
						overlayById={overlayById}
						onEdit={setModalTx}
					/>
				))}

				{filtered.length === 0 && (
					<div
						className="mono"
						style={{
							color: "var(--muted)",
							fontSize: 13,
							padding: "32px 0",
							textAlign: "center",
						}}
					>
						{hasFilters
							? "Nenhuma transação para este filtro."
							: "Sem transações neste mês."}
					</div>
				)}
			</div>

			{/* ── Edit modal ── */}
			<AnimatePresence>
				{modalTx && (
					<TransactionModal
						tx={modalTx}
						overlay={overlayById.get(modalTx.id)}
						similarTxs={txRows.filter(
							(t) =>
								t.id !== modalTx.id &&
								(effectiveCat(t) === effectiveCat(modalTx) ||
									(t.merchantName &&
										t.merchantName === modalTx.merchantName)),
						)}
						overlayById={overlayById}
						onSubmit={(patch) => {
							submit(modalTx.id, patch);
							setModalTx(null);
						}}
						onClose={() => setModalTx(null)}
					/>
				)}
			</AnimatePresence>

			{/* Category datalist */}
			<datalist id="phai-cats">
				{categoryIds.map((c) => (
					<option key={c} value={c} />
				))}
			</datalist>
		</div>
	);
};

// ── Month summary strip ────────────────────────────────────────────────────

const MonthSummary = ({
	month,
	isFuture,
	entradas,
	saidas,
	projectedClose,
	forecastCount,
	installmentCount,
	installmentSum,
}: {
	month: string;
	isFuture: boolean;
	entradas: number;
	saidas: number;
	projectedClose: number | null;
	forecastCount: number;
	installmentCount: number;
	installmentSum: number;
}) => {
	const resultado = entradas - saidas;
	const monthName = new Date(month + "-15").toLocaleString("pt-BR", {
		month: "long",
		year: "numeric",
	});

	return (
		<div
			style={{
				padding: "18px 0 14px",
				borderBottom: "1px solid var(--border)",
				marginBottom: 0,
			}}
		>
			<div
				style={{
					display: "flex",
					alignItems: "baseline",
					gap: 10,
					marginBottom: 12,
				}}
			>
				<h2
					style={{
						fontFamily: "var(--font-display)",
						fontSize: "1.3rem",
						margin: 0,
						textTransform: "capitalize",
					}}
				>
					{monthName}
				</h2>
				{isFuture && (
					<span
						className="mono"
						style={{
							fontSize: 11,
							color: "var(--muted)",
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-full)",
							padding: "1px 8px",
						}}
					>
						previsto
					</span>
				)}
			</div>

			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit, minmax(120px, 1fr))",
					gap: 8,
				}}
			>
				<SumCard label="entradas" value={entradas} color="var(--cyan)" positive />
				<SumCard label="saídas" value={-saidas} color="var(--rose)" />
				<SumCard
					label="resultado"
					value={resultado}
					color={resultado >= 0 ? "var(--green)" : "var(--rose)"}
					positive={resultado >= 0}
				/>
				{projectedClose !== null && (
					<SumCard
						label="saldo proj."
						value={projectedClose}
						color="var(--purple)"
						positive={projectedClose >= 0}
					/>
				)}
			</div>

			{installmentCount > 0 && (
				<div
					className="mono"
					style={{
						marginTop: 8,
						fontSize: 11,
						color: "var(--amber)",
					}}
				>
					{installmentCount} parcela{installmentCount !== 1 ? "s" : ""} ·{" "}
					{formatMoneyNumber(-installmentSum)} ·{" "}
					{forecastCount > 0 ? `${forecastCount} previsões` : ""}
				</div>
			)}
		</div>
	);
};

const SumCard = ({
	label,
	value,
	color,
	positive,
}: {
	label: string;
	value: number;
	color: string;
	positive?: boolean;
}) => (
	<div
		style={{
			padding: "8px 12px",
			background: "var(--surface)",
			borderRadius: "var(--radius-sm)",
			border: "1px solid var(--border)",
		}}
	>
		<div className="mono" style={{ fontSize: 10, color: "var(--muted)", marginBottom: 2 }}>
			{label}
		</div>
		<div
			className="mono"
			style={{ fontSize: 14, fontWeight: 600, color }}
		>
			{positive && value > 0 ? "+" : ""}
			{formatMoneyNumber(value)}
		</div>
	</div>
);

// ── Forecast section ───────────────────────────────────────────────────────

const ForecastSection = ({
	month,
	forecasts,
	onAdded,
}: {
	month: string;
	forecasts: ForecastView[];
	onAdded: () => void;
}) => {
	const { store } = useStore();
	const [open, setOpen] = useState(false);
	const [addOpen, setAddOpen] = useState(false);
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [outflow, setOutflow] = useState(true);
	const { startDrag, dragging } = useDnd();

	const submitForecast = () => {
		const desc = description.trim();
		const mag = amount.replace(/^-/, "").trim();
		if (!desc || !mag) return;
		store.commit(
			events.forecastCreated({
				writeId: crypto.randomUUID(),
				description: desc,
				amount: outflow ? `-${mag}` : mag,
				dueDate: `${month}-01`,
				createdAt: Date.now(),
			}),
		);
		setDescription("");
		setAmount("");
		setAddOpen(false);
		onAdded();
	};

	if (forecasts.length === 0 && !addOpen) {
		return (
			<div style={{ padding: "10px 0" }}>
				<button
					onClick={() => setAddOpen(true)}
					className="mono"
					style={addBtnStyle}
				>
					+ nova previsão
				</button>
			</div>
		);
	}

	return (
		<div
			style={{
				borderBottom: "1px solid var(--border)",
				padding: "10px 0 12px",
			}}
		>
			<button
				onClick={() => setOpen((v) => !v)}
				className="mono"
				style={{
					background: "transparent",
					border: "none",
					cursor: "pointer",
					fontSize: 11,
					color: "var(--muted)",
					padding: 0,
					display: "flex",
					alignItems: "center",
					gap: 6,
				}}
			>
				<span>{open ? "▾" : "▸"}</span>
				<span style={{ color: "var(--cyan)" }}>
					{forecasts.length} previsão{forecasts.length !== 1 ? "ões" : ""}
				</span>
				<span>para {month}</span>
			</button>

			<AnimatePresence>
				{open && (
					<motion.div
						initial={{ opacity: 0, height: 0 }}
						animate={{ opacity: 1, height: "auto" }}
						exit={{ opacity: 0, height: 0 }}
						style={{ overflow: "hidden" }}
					>
						<div
							style={{
								display: "flex",
								flexDirection: "column",
								gap: 6,
								marginTop: 10,
							}}
						>
							{forecasts.map((f) => {
								const locked = f.draggable !== 1;
								const isDragging = dragging?.forecastId === f.forecastId;
								return (
									<div
										key={f.forecastId}
										onPointerDown={(e) => {
											if (locked || e.button !== 0) return;
											startDrag(
												{
													forecastId: f.forecastId,
													label: f.description,
													amount: formatMoney(f.amount),
												},
												e,
											);
										}}
										title={
											locked
												? "parcela/assinatura — bloqueada"
												: "arraste para outro mês no gráfico"
										}
										style={{
											display: "flex",
											justifyContent: "space-between",
											alignItems: "center",
											gap: 10,
											padding: "6px 10px",
											borderRadius: "var(--radius-sm)",
											border: "1px dashed var(--border)",
											background: f.kind === "manual"
												? "transparent"
												: "var(--surface)",
											cursor: locked ? "default" : "grab",
											opacity: isDragging ? 0.35 : 1,
											touchAction: "none",
											userSelect: "none",
										}}
									>
										<span
											style={{
												display: "flex",
												gap: 6,
												alignItems: "center",
												minWidth: 0,
											}}
										>
											<span
												className="mono"
												style={{ color: "var(--muted)", fontSize: 11 }}
											>
												{locked ? "⊘" : "⠿"}
											</span>
											<span
												style={{
													fontSize: 13,
													overflow: "hidden",
													textOverflow: "ellipsis",
													whiteSpace: "nowrap",
												}}
											>
												{f.description}
											</span>
										</span>
										<span
											className="mono"
											style={{
												color: amountColor(f.amount),
												fontSize: 13,
												whiteSpace: "nowrap",
											}}
										>
											{formatMoney(f.amount)}
										</span>
									</div>
								);
							})}
						</div>
					</motion.div>
				)}
			</AnimatePresence>

			{/* Add forecast inline form */}
			<AnimatePresence>
				{addOpen ? (
					<motion.div
						initial={{ opacity: 0, height: 0 }}
						animate={{ opacity: 1, height: "auto" }}
						exit={{ opacity: 0, height: 0 }}
						style={{ overflow: "hidden", marginTop: 8 }}
					>
						<div
							style={{
								display: "flex",
								flexDirection: "column",
								gap: 8,
								padding: 10,
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
							}}
						>
							<input
								autoFocus
								placeholder="descrição da previsão"
								value={description}
								onChange={(e) => setDescription(e.target.value)}
								className="mono"
								style={inputStyle}
							/>
							<div
								style={{
									display: "flex",
									gap: 8,
									alignItems: "center",
								}}
							>
								<ToggleBtn
									active={outflow}
									color="var(--rose)"
									onClick={() => setOutflow(true)}
								>
									saída
								</ToggleBtn>
								<ToggleBtn
									active={!outflow}
									color="var(--green)"
									onClick={() => setOutflow(false)}
								>
									entrada
								</ToggleBtn>
								<input
									inputMode="decimal"
									placeholder="0,00"
									value={amount}
									onChange={(e) => setAmount(e.target.value)}
									onKeyDown={(e) => e.key === "Enter" && submitForecast()}
									className="mono"
									style={{ ...inputStyle, width: 100 }}
								/>
							</div>
							<div style={{ display: "flex", gap: 8 }}>
								<button
									onClick={submitForecast}
									disabled={!description.trim() || !amount.trim()}
									className="mono"
									style={{
										...pillStyle,
										background: "var(--cyan)",
										color: "#fff",
										opacity:
											!description.trim() || !amount.trim() ? 0.4 : 1,
									}}
								>
									adicionar →
								</button>
								<button
									onClick={() => setAddOpen(false)}
									className="mono"
									style={pillStyle}
								>
									cancelar
								</button>
							</div>
						</div>
					</motion.div>
				) : (
					<motion.div
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						style={{ marginTop: 8 }}
					>
						<button
							onClick={() => setAddOpen(true)}
							className="mono"
							style={addBtnStyle}
						>
							+ nova previsão
						</button>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
};

// ── Filter bar ─────────────────────────────────────────────────────────────

const FilterBar = ({
	ui,
	setUi,
	owners,
	accounts,
	hasFilters,
}: {
	ui: {
		textFilter: string | null;
		accountFilter: string | null;
		ownerFilter: string | null;
		categoryFilter: string | null;
		installmentsOnly: boolean;
		subscriptionsOnly: boolean;
		unreviewedOnly: boolean;
	};
	setUi: (patch: Partial<typeof ui>) => void;
	owners: string[];
	accounts: ReadonlyArray<{ id: string; label: string; owner: string }>;
	hasFilters: boolean;
}) => (
	<div
		style={{
			display: "flex",
			flexWrap: "wrap",
			gap: 8,
			alignItems: "center",
			padding: "12px 0 8px",
		}}
	>
		{/* Text search */}
		<div style={{ position: "relative", flexGrow: 1, maxWidth: 260 }}>
			<span
				style={{
					position: "absolute",
					left: 9,
					top: "50%",
					transform: "translateY(-50%)",
					color: "var(--muted2)",
					fontSize: 12,
					pointerEvents: "none",
				}}
			>
				⌕
			</span>
			<input
				placeholder="buscar transações…"
				value={ui.textFilter ?? ""}
				onChange={(e) => setUi({ textFilter: e.target.value || null })}
				className="mono"
				style={{ ...inputStyle, paddingLeft: 26, width: "100%" }}
				aria-label="busca textual"
			/>
		</div>

		{/* Category filter */}
		<input
			list="phai-cats"
			placeholder="categoria…"
			value={ui.categoryFilter ?? ""}
			onChange={(e) => setUi({ categoryFilter: e.target.value || null })}
			className="mono"
			style={{ ...inputStyle, color: "var(--cyan)", width: 150 }}
			aria-label="filtrar por categoria"
		/>

		{/* Account filter */}
		{accounts.length > 0 && (
			<select
				value={ui.accountFilter ?? ""}
				onChange={(e) => setUi({ accountFilter: e.target.value || null })}
				className="mono"
				style={selectStyle}
				aria-label="conta"
			>
				<option value="">todas · conta</option>
				{accounts.map((a) => (
					<option key={a.id} value={a.id}>
						{a.label || a.id}
					</option>
				))}
			</select>
		)}

		{/* Owner filter */}
		{owners.length > 1 && (
			<select
				value={ui.ownerFilter ?? ""}
				onChange={(e) => setUi({ ownerFilter: e.target.value || null })}
				className="mono"
				style={selectStyle}
				aria-label="responsável"
			>
				<option value="">todos · responsável</option>
				{owners.map((o) => (
					<option key={o} value={o}>
						{o}
					</option>
				))}
			</select>
		)}

		{/* Toggle pills */}
		<ToggleBtn
			active={ui.installmentsOnly}
			color="var(--amber)"
			onClick={() =>
				setUi({ installmentsOnly: !ui.installmentsOnly })
			}
		>
			parcelas
		</ToggleBtn>
		<ToggleBtn
			active={ui.subscriptionsOnly}
			color="var(--cyan)"
			onClick={() =>
				setUi({ subscriptionsOnly: !ui.subscriptionsOnly })
			}
		>
			assinaturas
		</ToggleBtn>
		<ToggleBtn
			active={ui.unreviewedOnly}
			color="var(--purple)"
			onClick={() => setUi({ unreviewedOnly: !ui.unreviewedOnly })}
		>
			não revisadas
		</ToggleBtn>

		{/* Clear filters */}
		{hasFilters && (
			<button
				onClick={() =>
					setUi({
						textFilter: null,
						categoryFilter: null,
						accountFilter: null,
						ownerFilter: null,
						installmentsOnly: false,
						subscriptionsOnly: false,
						unreviewedOnly: false,
					})
				}
				className="mono"
				style={{
					...pillStyle,
					color: "var(--rose)",
					borderColor: "var(--rose)",
				}}
			>
				× limpar
			</button>
		)}
	</div>
);

const FilterSummary = ({
	count,
	saidas,
	entradas,
}: {
	count: number;
	saidas: number;
	entradas: number;
}) => (
	<div
		className="mono"
		style={{
			fontSize: 11,
			color: "var(--muted)",
			padding: "6px 0 8px",
			display: "flex",
			gap: 14,
			flexWrap: "wrap",
		}}
	>
		<span>{count} transação{count !== 1 ? "ões" : ""}</span>
		{saidas > 0 && (
			<span style={{ color: "var(--rose)" }}>
				saídas {formatMoneyNumber(-saidas)}
			</span>
		)}
		{entradas > 0 && (
			<span style={{ color: "var(--cyan)" }}>
				entradas {formatMoneyNumber(entradas)}
			</span>
		)}
		{(saidas > 0 || entradas > 0) && (
			<span
				style={{
					color: entradas - saidas >= 0 ? "var(--green)" : "var(--rose)",
				}}
			>
				líquido{" "}
				{entradas - saidas >= 0 ? "+" : ""}
				{formatMoneyNumber(entradas - saidas)}
			</span>
		)}
	</div>
);

// ── Category group ─────────────────────────────────────────────────────────

const CategoryGroup = ({
	label,
	isIncome,
	txs,
	overlayById,
	onEdit,
}: {
	label: string;
	isIncome: boolean;
	txs: TxView[];
	overlayById: Map<
		string,
		{ categoryId: string | null; description: string | null; merchantName: string | null; purpose: string | null }
	>;
	onEdit: (tx: TxView) => void;
}) => {
	const [expanded, setExpanded] = useState(true);
	const total = sumAmounts(txs.map((t) => t.amount));
	const installmentTxs = txs.filter((t) => t.isInstallment === 1);

	return (
		<div
			style={{
				marginBottom: 8,
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-md)",
				overflow: "hidden",
			}}
		>
			{/* Group header */}
			<button
				onClick={() => setExpanded((v) => !v)}
				style={{
					width: "100%",
					display: "flex",
					alignItems: "center",
					gap: 10,
					padding: "10px 14px",
					background: "var(--surface)",
					border: "none",
					cursor: "pointer",
					textAlign: "left",
				}}
			>
				<span style={{ color: "var(--muted)", fontSize: 11, minWidth: 12 }}>
					{expanded ? "▾" : "▸"}
				</span>
				<span
					style={{
						flex: 1,
						fontWeight: 500,
						fontSize: 13,
						color: isIncome ? "var(--green)" : "var(--white)",
						overflow: "hidden",
						textOverflow: "ellipsis",
						whiteSpace: "nowrap",
					}}
				>
					{isIncome ? "↑ Entradas" : label}
				</span>
				{installmentTxs.length > 0 && (
					<span
						className="mono"
						style={{
							fontSize: 10,
							color: "var(--amber)",
							border: "1px solid var(--amber)",
							borderRadius: "var(--radius-full)",
							padding: "1px 6px",
						}}
					>
						{installmentTxs.length}× parcela
					</span>
				)}
				<span
					className="mono"
					style={{
						fontSize: 12,
						fontWeight: 600,
						color: isIncome ? "var(--green)" : "var(--rose)",
						whiteSpace: "nowrap",
					}}
				>
					{formatMoney(String(total))}
				</span>
				<span
					className="mono"
					style={{ fontSize: 10, color: "var(--muted2)" }}
				>
					{txs.length}
				</span>
			</button>

			{/* Transactions */}
			<AnimatePresence initial={false}>
				{expanded && (
					<motion.div
						initial={{ height: 0, opacity: 0 }}
						animate={{ height: "auto", opacity: 1 }}
						exit={{ height: 0, opacity: 0 }}
						transition={{ duration: 0.18, ease: "easeInOut" }}
						style={{ overflow: "hidden" }}
					>
						<div style={{ display: "flex", flexDirection: "column" }}>
							{txs.map((tx) => {
								const o = overlayById.get(tx.id);
								return (
									<TxRow
										key={tx.id}
										tx={tx}
										overlay={o}
										onEdit={() => onEdit(tx)}
									/>
								);
							})}
						</div>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
};

// ── Transaction row ────────────────────────────────────────────────────────

const TxRow = ({
	tx,
	overlay,
	onEdit,
}: {
	tx: TxView;
	overlay?: {
		categoryId: string | null;
		description: string | null;
		merchantName: string | null;
		purpose: string | null;
	};
	onEdit: () => void;
}) => {
	const display =
		overlay?.description ??
		tx.description ??
		overlay?.merchantName ??
		tx.merchantName ??
		tx.rawDescription;
	const cat = overlay?.categoryId ?? tx.categoryId;

	return (
		<button
			onClick={onEdit}
			style={{
				width: "100%",
				display: "flex",
				alignItems: "center",
				gap: 12,
				padding: "9px 14px",
				background: "transparent",
				border: "none",
				borderTop: "1px solid var(--border)",
				cursor: "pointer",
				textAlign: "left",
				transition: "background 80ms",
			}}
			onMouseEnter={(e) => {
				(e.currentTarget as HTMLButtonElement).style.background =
					"rgba(0,0,0,0.02)";
			}}
			onMouseLeave={(e) => {
				(e.currentTarget as HTMLButtonElement).style.background =
					"transparent";
			}}
		>
			{/* Date + badges */}
			<div
				className="mono"
				style={{ fontSize: 10, color: "var(--muted2)", minWidth: 50 }}
			>
				{tx.postedAt.slice(5, 10)}
			</div>

			{/* Description */}
			<div style={{ flex: 1, minWidth: 0 }}>
				<div
					style={{
						fontSize: 13,
						overflow: "hidden",
						textOverflow: "ellipsis",
						whiteSpace: "nowrap",
						display: "flex",
						gap: 6,
						alignItems: "center",
					}}
				>
					<span>{display}</span>
					{tx.isInstallment === 1 && (
						<TagBadge label="parcela" color="var(--amber)" />
					)}
					{tx.isSubscription === 1 && (
						<TagBadge label="assinatura" color="var(--cyan)" />
					)}
					{tx.reviewed === 1 && (
						<TagBadge label="✓" color="var(--green)" />
					)}
				</div>
				{cat && (
					<div
						className="mono"
						style={{ fontSize: 10, color: "var(--cyan)", marginTop: 1 }}
					>
						{cat}
					</div>
				)}
			</div>

			{/* Amount */}
			<span
				className="mono"
				style={{
					color: amountColor(tx.amount),
					fontSize: 13,
					fontWeight: 500,
					whiteSpace: "nowrap",
				}}
			>
				{formatMoney(tx.amount)}
			</span>

			{/* Edit hint */}
			<span style={{ color: "var(--muted2)", fontSize: 10 }}>›</span>
		</button>
	);
};

const TagBadge = ({ label, color }: { label: string; color: string }) => (
	<span
		className="mono"
		style={{
			fontSize: 9,
			color,
			border: `1px solid ${color}`,
			borderRadius: "var(--radius-full)",
			padding: "0 5px",
			whiteSpace: "nowrap",
			lineHeight: 1.6,
		}}
	>
		{label}
	</span>
);

// ── Transaction modal ──────────────────────────────────────────────────────

const TransactionModal = ({
	tx,
	overlay,
	similarTxs,
	overlayById,
	onSubmit,
	onClose,
}: {
	tx: TxView;
	overlay?: {
		description: string | null;
		merchantName: string | null;
		purpose: string | null;
		categoryId: string | null;
	};
	similarTxs: ReadonlyArray<TxView>;
	overlayById: Map<
		string,
		{ categoryId: string | null; description: string | null; merchantName: string | null; purpose: string | null }
	>;
	onSubmit: (patch: ReviewPatch) => void;
	onClose: () => void;
}) => {
	type Tab = "edit" | "raw" | "similar";
	const [tab, setTab] = useState<Tab>("edit");
	const [description, setDescription] = useState(
		overlay?.description ?? tx.description ?? "",
	);
	const [merchantName, setMerchantName] = useState(
		overlay?.merchantName ?? tx.merchantName ?? "",
	);
	const [purpose, setPurpose] = useState(overlay?.purpose ?? tx.purpose ?? "");
	const [category, setCategory] = useState(
		overlay?.categoryId ?? tx.categoryId ?? "",
	);

	// Keep fields in sync if overlay changes while modal is open
	useEffect(() => {
		setDescription(overlay?.description ?? tx.description ?? "");
		setMerchantName(overlay?.merchantName ?? tx.merchantName ?? "");
		setPurpose(overlay?.purpose ?? tx.purpose ?? "");
		setCategory(overlay?.categoryId ?? tx.categoryId ?? "");
	}, [tx.id]); // reset on tx change only

	// Bulk edit: selected similar txs
	const [selectedSimilar, setSelectedSimilar] = useState<Set<string>>(
		new Set(),
	);
	const { store } = useStore();

	const applyBulk = (newCategory: string) => {
		for (const id of selectedSimilar) {
			store.commit(
				events.reviewSubmitted({
					writeId: crypto.randomUUID(),
					transactionId: id,
					patch: {
						description: null,
						merchantName: null,
						purpose: null,
						categoryId: newCategory || null,
					},
					submittedAt: Date.now(),
				}),
			);
		}
		setSelectedSimilar(new Set());
	};

	return (
		<>
			{/* Backdrop — also the flex centering container for the panel. We center
			    via flexbox (not transform) because Framer Motion drives the panel's
			    `transform` via scale/y; a transform-based translate would be
			    clobbered on every animation frame. */}
			<motion.div
				key="modal-backdrop"
				initial={{ opacity: 0 }}
				animate={{ opacity: 1 }}
				exit={{ opacity: 0 }}
				transition={{ duration: 0.15 }}
				onClick={onClose}
				style={{
					position: "fixed",
					inset: 0,
					background: "rgba(21,19,31,0.35)",
					backdropFilter: "blur(2px)",
					zIndex: 50,
					display: "flex",
					alignItems: "center",
					justifyContent: "center",
					padding: 20,
				}}
			>
				{/* Modal panel */}
				<motion.div
					key="modal-panel"
					onClick={(e) => e.stopPropagation()}
					initial={{ opacity: 0, scale: 0.97, y: 8 }}
					animate={{ opacity: 1, scale: 1, y: 0 }}
					exit={{ opacity: 0, scale: 0.97, y: 8 }}
					transition={{ duration: 0.16, ease: "easeOut" }}
					style={{
						width: "100%",
						maxWidth: tab === "similar" ? 900 : 520,
						maxHeight: "85vh",
						overflowY: "auto",
						background: "var(--bg)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-xl)",
						padding: 24,
						boxShadow: "0 20px 60px rgba(21,19,31,0.14)",
					}}
				>
				{/* Header */}
				<div
					style={{
						display: "flex",
						alignItems: "center",
						gap: 10,
						marginBottom: 16,
					}}
				>
					<span
						className="mono"
						style={{
							color: amountColor(tx.amount),
							fontWeight: 600,
							fontSize: 15,
						}}
					>
						{formatMoney(tx.amount)}
					</span>
					<span
						style={{
							flex: 1,
							overflow: "hidden",
							textOverflow: "ellipsis",
							whiteSpace: "nowrap",
							fontSize: 13,
						}}
					>
						{tx.description ?? tx.merchantName ?? tx.rawDescription}
					</span>
					<button
						onClick={onClose}
						className="mono"
						style={{
							background: "transparent",
							border: "none",
							cursor: "pointer",
							color: "var(--muted)",
							fontSize: 16,
							padding: "0 4px",
						}}
					>
						×
					</button>
				</div>

				{/* Tabs */}
				<div
					style={{
						display: "flex",
						gap: 4,
						marginBottom: 18,
						borderBottom: "1px solid var(--border)",
						paddingBottom: 8,
					}}
				>
					{(["edit", "raw", "similar"] as Tab[]).map((t) => (
						<button
							key={t}
							onClick={() => setTab(t)}
							className="mono"
							style={{
								background:
									tab === t ? "rgba(109,74,255,0.08)" : "transparent",
								color: tab === t ? "var(--purple)" : "var(--muted)",
								border: `1px solid ${tab === t ? "rgba(109,74,255,0.3)" : "transparent"}`,
								borderRadius: "var(--radius-full)",
								padding: "4px 14px",
								cursor: "pointer",
								fontSize: 12,
							}}
						>
							{t === "edit"
								? "Editar"
								: t === "raw"
									? "JSON"
									: `Similares (${similarTxs.length})`}
						</button>
					))}
				</div>

				{/* Tab content */}
				<AnimatePresence mode="wait" initial={false}>
					{tab === "edit" && (
						<motion.div
							key="edit"
							initial={{ opacity: 0, x: -8 }}
							animate={{ opacity: 1, x: 0 }}
							exit={{ opacity: 0, x: 8 }}
							transition={{ duration: 0.12 }}
						>
							<EditForm
								description={description}
								setDescription={setDescription}
								merchantName={merchantName}
								setMerchantName={setMerchantName}
								purpose={purpose}
								setPurpose={setPurpose}
								category={category}
								setCategory={setCategory}
								onSave={() =>
									onSubmit({
										description: description.trim() || null,
										merchantName: merchantName.trim() || null,
										purpose: purpose.trim() || null,
										categoryId: category.trim() || null,
									})
								}
								onCancel={onClose}
								postedAt={tx.postedAt}
								accountId={tx.accountId}
							/>
						</motion.div>
					)}

					{tab === "raw" && (
						<motion.div
							key="raw"
							initial={{ opacity: 0, x: -8 }}
							animate={{ opacity: 1, x: 0 }}
							exit={{ opacity: 0, x: 8 }}
							transition={{ duration: 0.12 }}
						>
							<pre
								className="mono"
								style={{
									background: "var(--surface)",
									border: "1px solid var(--border)",
									borderRadius: "var(--radius-sm)",
									padding: 14,
									fontSize: 11,
									overflowX: "auto",
									whiteSpace: "pre-wrap",
									wordBreak: "break-all",
									lineHeight: 1.6,
								}}
							>
								{JSON.stringify(
									{
										id: tx.id,
										accountId: tx.accountId,
										postedAt: tx.postedAt,
										amount: tx.amount,
										rawDescription: tx.rawDescription,
										description: tx.description,
										merchantName: tx.merchantName,
										purpose: tx.purpose,
										categoryId: tx.categoryId,
										month: tx.month,
										paymentStatus: tx.paymentStatus,
										reviewed: tx.reviewed,
										isInstallment: tx.isInstallment,
										isSubscription: tx.isSubscription,
										_overlay: overlayById.get(tx.id) ?? null,
									},
									null,
									2,
								)}
							</pre>
						</motion.div>
					)}

					{tab === "similar" && (
						<motion.div
							key="similar"
							initial={{ opacity: 0, x: -8 }}
							animate={{ opacity: 1, x: 0 }}
							exit={{ opacity: 0, x: 8 }}
							transition={{ duration: 0.12 }}
						>
							<SimilarPanel
								similarTxs={similarTxs}
								overlayById={overlayById}
								selected={selectedSimilar}
								onToggle={(id) => {
									setSelectedSimilar((prev) => {
										const next = new Set(prev);
										if (next.has(id)) next.delete(id);
										else next.add(id);
										return next;
									});
								}}
								onSelectAll={() =>
									setSelectedSimilar(
										new Set(similarTxs.map((t) => t.id)),
									)
								}
								onClearAll={() => setSelectedSimilar(new Set())}
								onApplyBulk={applyBulk}
							/>
						</motion.div>
					)}
				</AnimatePresence>
				</motion.div>
			</motion.div>
		</>
	);
};

const EditForm = ({
	description,
	setDescription,
	merchantName,
	setMerchantName,
	purpose,
	setPurpose,
	category,
	setCategory,
	onSave,
	onCancel,
	postedAt,
	accountId,
}: {
	description: string;
	setDescription: (v: string) => void;
	merchantName: string;
	setMerchantName: (v: string) => void;
	purpose: string;
	setPurpose: (v: string) => void;
	category: string;
	setCategory: (v: string) => void;
	onSave: () => void;
	onCancel: () => void;
	postedAt: string;
	accountId: string;
}) => (
	<div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
		<div
			className="mono"
			style={{ fontSize: 11, color: "var(--muted)", marginBottom: 4 }}
		>
			{postedAt} · {accountId}
		</div>

		<FieldRow label="categoria">
			<input
				list="phai-cats"
				value={category}
				onChange={(e) => setCategory(e.target.value)}
				placeholder="categoria"
				className="mono"
				style={{ ...inputStyle, color: "var(--cyan)", flex: 1 }}
				autoFocus
			/>
		</FieldRow>
		<FieldRow label="descrição">
			<input
				value={description}
				onChange={(e) => setDescription(e.target.value)}
				placeholder="descrição"
				className="mono"
				style={{ ...inputStyle, flex: 1 }}
			/>
		</FieldRow>
		<FieldRow label="merchant">
			<input
				value={merchantName}
				onChange={(e) => setMerchantName(e.target.value)}
				placeholder="merchant"
				className="mono"
				style={{ ...inputStyle, flex: 1 }}
			/>
		</FieldRow>
		<FieldRow label="propósito">
			<input
				value={purpose}
				onChange={(e) => setPurpose(e.target.value)}
				placeholder="propósito"
				className="mono"
				style={{ ...inputStyle, flex: 1 }}
			/>
		</FieldRow>

		<div style={{ display: "flex", gap: 8, marginTop: 4 }}>
			<button
				onClick={onSave}
				className="mono"
				style={{
					...pillStyle,
					background: "var(--purple)",
					color: "#fff",
					borderColor: "transparent",
				}}
			>
				salvar →
			</button>
			<button onClick={onCancel} className="mono" style={pillStyle}>
				cancelar
			</button>
		</div>
	</div>
);

const FieldRow = ({
	label,
	children,
}: {
	label: string;
	children: React.ReactNode;
}) => (
	<div
		style={{
			display: "grid",
			gridTemplateColumns: "90px 1fr",
			gap: 10,
			alignItems: "center",
		}}
	>
		<span
			className="mono"
			style={{
				fontSize: 10,
				color: "var(--muted)",
				textTransform: "uppercase",
				letterSpacing: "0.06em",
			}}
		>
			{label}
		</span>
		{children}
	</div>
);

const SimilarPanel = ({
	similarTxs,
	overlayById,
	selected,
	onToggle,
	onSelectAll,
	onClearAll,
	onApplyBulk,
}: {
	similarTxs: ReadonlyArray<TxView>;
	overlayById: Map<
		string,
		{ categoryId: string | null; description: string | null; merchantName: string | null; purpose: string | null }
	>;
	selected: Set<string>;
	onToggle: (id: string) => void;
	onSelectAll: () => void;
	onClearAll: () => void;
	onApplyBulk: (cat: string) => void;
}) => {
	const [bulkCat, setBulkCat] = useState("");

	if (similarTxs.length === 0) {
		return (
			<p className="mono" style={{ color: "var(--muted)", fontSize: 13 }}>
				Sem transações similares nesta janela.
			</p>
		);
	}

	return (
		<div>
			<div
				style={{
					display: "flex",
					gap: 8,
					alignItems: "center",
					marginBottom: 12,
					flexWrap: "wrap",
				}}
			>
				<button onClick={onSelectAll} className="mono" style={pillStyle}>
					selecionar todas ({similarTxs.length})
				</button>
				{selected.size > 0 && (
					<button onClick={onClearAll} className="mono" style={pillStyle}>
						limpar ({selected.size})
					</button>
				)}
				{selected.size > 0 && (
					<>
						<input
							list="phai-cats"
							placeholder="nova categoria…"
							value={bulkCat}
							onChange={(e) => setBulkCat(e.target.value)}
							className="mono"
							style={{ ...inputStyle, color: "var(--cyan)", width: 160 }}
						/>
						<button
							onClick={() => onApplyBulk(bulkCat)}
							disabled={!bulkCat.trim()}
							className="mono"
							style={{
								...pillStyle,
								background: "var(--purple)",
								color: "#fff",
								opacity: !bulkCat.trim() ? 0.4 : 1,
							}}
						>
							aplicar em {selected.size} →
						</button>
					</>
				)}
			</div>

			<div
				style={{
					display: "flex",
					flexDirection: "column",
					gap: 0,
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-sm)",
					overflow: "hidden",
				}}
			>
				{similarTxs.map((tx, idx) => {
					const o = overlayById.get(tx.id);
					const cat = o?.categoryId ?? tx.categoryId;
					const display =
						o?.description ??
						tx.description ??
						tx.merchantName ??
						tx.rawDescription;
					const isSelected = selected.has(tx.id);

					return (
						<div
							key={tx.id}
							onClick={() => onToggle(tx.id)}
							style={{
								display: "flex",
								alignItems: "center",
								gap: 10,
								padding: "8px 12px",
								borderTop: idx > 0 ? "1px solid var(--border)" : "none",
								background: isSelected
									? "rgba(109,74,255,0.06)"
									: "transparent",
								cursor: "pointer",
								transition: "background 80ms",
							}}
						>
							<span
								style={{
									width: 14,
									height: 14,
									borderRadius: 3,
									border: `2px solid ${isSelected ? "var(--purple)" : "var(--border)"}`,
									background: isSelected
										? "var(--purple)"
										: "transparent",
									flexShrink: 0,
									display: "flex",
									alignItems: "center",
									justifyContent: "center",
								}}
							>
								{isSelected && (
									<span style={{ color: "#fff", fontSize: 9, fontWeight: 700 }}>
										✓
									</span>
								)}
							</span>
							<span
								className="mono"
								style={{ fontSize: 10, color: "var(--muted2)", minWidth: 50 }}
							>
								{tx.postedAt.slice(0, 7)}
							</span>
							<span
								style={{
									flex: 1,
									fontSize: 12,
									overflow: "hidden",
									textOverflow: "ellipsis",
									whiteSpace: "nowrap",
								}}
							>
								{display}
							</span>
							{cat && (
								<span
									className="mono"
									style={{ fontSize: 10, color: "var(--cyan)" }}
								>
									{cat}
								</span>
							)}
							<span
								className="mono"
								style={{
									color: amountColor(tx.amount),
									fontSize: 12,
									whiteSpace: "nowrap",
								}}
							>
								{formatMoney(tx.amount)}
							</span>
						</div>
					);
				})}
			</div>
		</div>
	);
};

// ── Shared micro-styles ────────────────────────────────────────────────────

const inputStyle: React.CSSProperties = {
	background: "var(--bg)",
	color: "var(--white)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "5px 9px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
	outline: "none",
};

const pillStyle: React.CSSProperties = {
	background: "transparent",
	color: "var(--muted)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "4px 12px",
	cursor: "pointer",
	fontSize: 11,
	fontFamily: "var(--font-mono)",
};

const addBtnStyle: React.CSSProperties = {
	...pillStyle,
	borderStyle: "dashed",
	color: "var(--muted)",
};

const ToggleBtn = ({
	active,
	color,
	onClick,
	children,
}: {
	active: boolean;
	color: string;
	onClick: () => void;
	children: React.ReactNode;
}) => (
	<button
		onClick={onClick}
		className="mono"
		style={{
			...pillStyle,
			color: active ? color : "var(--muted)",
			border: `1px solid ${active ? color : "var(--border)"}`,
			background: active ? `${color}14` : "transparent",
		}}
	>
		{children}
	</button>
);

const selectStyle: React.CSSProperties = {
	...inputStyle,
	cursor: "pointer",
	paddingRight: 6,
};
