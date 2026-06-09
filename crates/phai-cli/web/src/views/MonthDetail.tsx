import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useCallback, useEffect, useMemo, useState } from "react";
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
import { useDebounce } from "../hooks/useDebounce";
import { CategoryTreemap } from "./categorias/CategoryTreemap";
import { TransactionModal } from "../components/TransactionModal";
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
	installmentMarker?: string | null;
	accountLabel?: string;
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
	months,
	onMoveForecast,
}: {
	month: string;
	chart: ChartMonthView | null;
	forecasts: ForecastView[];
	onForecastAdded: () => void;
	months: ReadonlyArray<ChartMonthView>;
	onMoveForecast: (forecastId: string, targetMonth: string) => void;
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
	const effectiveCat = useCallback(
		(tx: TxView) => overlayById.get(tx.id)?.categoryId ?? tx.categoryId,
		[overlayById],
	);

	// Transactions for this month
	const monthTxs = useMemo(
		() =>
			txRows
				.filter((t) => t.month === month)
				.map((t) => ({
					...t,
					accountLabel: accountById.get(t.accountId)?.label || t.accountId,
				})),
		[txRows, month, accountById],
	);

	// ── Debounced text filter ───────────────────────────────────────────
	const [textInput, setTextInput] = useState(ui.textFilter ?? "");
	const debouncedText = useDebounce(textInput, 200);

	// Sync debounced text back to LiveStore UI
	useEffect(() => {
		setUi({ textFilter: debouncedText || null });
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [debouncedText]);

	// Apply filters
	const filtered = useMemo(() => {
		const cat = ui.categoryFilter?.trim().toLowerCase() ?? null;
		const text = debouncedText.trim().toLowerCase() || null;
		return monthTxs.filter((tx) => {
			if (ui.installmentsOnly && !tx.isInstallment) return false;
			if (ui.subscriptionsOnly && !tx.isSubscription) return false;
			if (ui.unreviewedOnly && tx.reviewed) return false;
			if (ui.uncategorizedOnly && (effectiveCat(tx) ?? "") !== "") return false;
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
	}, [monthTxs, overlayById, accountById, ui, debouncedText, effectiveCat]);

	// Filter sums
	const sums = useMemo(() => {
		const out = filtered
			.filter((t) => isNegative(t.amount))
			.map((t) => t.amount);
		const inc = filtered
			.filter((t) => !isNegative(t.amount))
			.map((t) => t.amount);
		return { saidas: Math.abs(sumAmounts(out)), entradas: sumAmounts(inc) };
	}, [filtered]);

	// Modal state — stable setter
	const [modalTx, setModalTx] = useState<TxView | null>(null);

	const onEdit = useCallback((tx: TxView) => setModalTx(tx), []);

	const handleCloseModal = useCallback(() => setModalTx(null), []);

	const submit = useCallback(
		(transactionId: string, patch: ReviewPatch) => {
			store.commit(
				events.reviewSubmitted({
					writeId: crypto.randomUUID(),
					transactionId,
					patch,
					submittedAt: Date.now(),
				}),
			);
		},
		[store],
	);

	const handleModalSubmit = useCallback(
		(txId: string, patch: ReviewPatch) => {
			submit(txId, patch);
			setModalTx(null);
		},
		[submit],
	);

	const hasFilters =
		ui.installmentsOnly ||
		ui.subscriptionsOnly ||
		ui.unreviewedOnly ||
		ui.uncategorizedOnly ||
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

	return (
		<div style={{ paddingBottom: 80 }}>
			{/* ── Month header (the numeric synthesis lives in the sticky hero) ── */}
			<MonthSummary
				month={month}
				isFuture={chart?.isFuture === 1}
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
					months={months}
					onMoveForecast={onMoveForecast}
				/>
			) : null}

			{/* ── Filter bar ── */}
			<FilterBar
				ui={ui}
				textInput={textInput}
				setUi={setUi}
				onTextInput={setTextInput}
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
							selectedCount={0}
						/>
					</motion.div>
				)}
			</AnimatePresence>

			{/* ── Categories as a drillable treemap (parent → sub → txs) ── */}
			<div style={{ marginTop: 12 }}>
				<CategoryTreemap
					txs={filtered}
					overlayMap={overlayById}
					onEditTx={onEdit}
				/>
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
									(t.merchantName && t.merchantName === modalTx.merchantName)),
						)}
						overlayById={overlayById}
						categories={categoryIds}
						onSubmit={(patch) => handleModalSubmit(modalTx.id, patch)}
						onClose={handleCloseModal}
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
	forecastCount,
	installmentCount,
	installmentSum,
}: {
	month: string;
	isFuture: boolean;
	forecastCount: number;
	installmentCount: number;
	installmentSum: number;
}) => {
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

// ── Forecast section ───────────────────────────────────────────────────────

const ForecastSection = ({
	month,
	forecasts,
	onAdded,
	months,
	onMoveForecast,
}: {
	month: string;
	forecasts: ForecastView[];
	onAdded: () => void;
	months: ReadonlyArray<ChartMonthView>;
	onMoveForecast: (forecastId: string, targetMonth: string) => void;
}) => {
	const { store } = useStore();
	const [open, setOpen] = useState(false);
	const [addOpen, setAddOpen] = useState(false);
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [outflow, setOutflow] = useState(true);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [movingId, setMovingId] = useState<string | null>(null);
	const [pickerOpen, setPickerOpen] = useState(false);
	const { startDrag, dragging } = useDnd();

	// Allowed target months: current month + any future months (no past).
	const currentMonth = useMemo(() => {
		const d = new Date();
		return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
	}, []);
	const allowedMonths = useMemo(
		() => months.filter((m) => m.month >= currentMonth),
		[months, currentMonth],
	);

	const toggleOpen = useCallback(() => setOpen((v) => !v), []);
	const openAdd = useCallback(() => setAddOpen(true), []);
	const closeAdd = useCallback(() => setAddOpen(false), []);
	const setOut = useCallback(() => setOutflow(true), []);
	const setIn = useCallback(() => setOutflow(false), []);

	// Move selected forecast to target month. Animates briefly.
	const doMove = useCallback(
		(forecastId: string, targetMonth: string) => {
			setMovingId(forecastId);
			onMoveForecast(forecastId, targetMonth);
			setSelectedId(null);
			setPickerOpen(false);
			setTimeout(() => setMovingId(null), 400);
		},
		[onMoveForecast],
	);

	// Shift selected forecast by one allowed month.
	const shiftMonth = useCallback(
		(direction: -1 | 1) => {
			if (!selectedId) return;
			const f = forecasts.find((x) => x.forecastId === selectedId);
			if (!f || f.draggable !== 1) return;
			const current = f.month ?? month;
			const curIdx = allowedMonths.findIndex((m) => m.month >= current);
			if (curIdx === -1) return;
			const targetIdx = curIdx + direction;
			if (targetIdx < 0 || targetIdx >= allowedMonths.length) return;
			doMove(selectedId, allowedMonths[targetIdx].month);
		},
		[selectedId, forecasts, month, allowedMonths, doMove],
	);

	// Keyboard handler for forecast rows.
	const handleForecastKeyDown = useCallback(
		(e: React.KeyboardEvent, forecastId: string) => {
			const f = forecasts.find((x) => x.forecastId === forecastId);
			if (!f) return;
			const mod = e.ctrlKey || e.metaKey;
			if (mod && e.key === "ArrowLeft") {
				e.preventDefault();
				shiftMonth(-1);
			} else if (mod && e.key === "ArrowRight") {
				e.preventDefault();
				shiftMonth(1);
			} else if (mod && (e.key === "m" || e.key === "M")) {
				e.preventDefault();
				if (f.draggable === 1) setPickerOpen(true);
			} else if (e.key === "Enter" || e.key === " ") {
				e.preventDefault();
				setSelectedId((prev) => (prev === forecastId ? null : forecastId));
			}
		},
		[forecasts, shiftMonth],
	);

	const submitForecast = useCallback(() => {
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
	}, [description, amount, outflow, month, store, onAdded]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter") submitForecast();
		},
		[submitForecast],
	);

	if (forecasts.length === 0 && !addOpen) {
		return (
			<div style={{ padding: "10px 0" }}>
				<button onClick={openAdd} className="mono" style={addBtnStyle}>
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
				onClick={toggleOpen}
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
								const isSelected = selectedId === f.forecastId;
								const isMoving = movingId === f.forecastId;
								const lockReason =
									f.kind === "installment"
										? "parcela — bloqueada"
										: f.kind === "subscription"
											? "assinatura — bloqueada"
											: null;
								return (
									<div
										key={f.forecastId}
										tabIndex={0}
										role="option"
										aria-selected={isSelected}
										aria-label={`previsão ${f.description}${locked ? " — " + lockReason : ""}`}
										onClick={() => {
											setSelectedId((prev) =>
												prev === f.forecastId ? null : f.forecastId,
											);
										}}
										onKeyDown={(e) => handleForecastKeyDown(e, f.forecastId)}
										onPointerDown={(e) => {
											if (locked || e.button !== 0) return;
											startDrag(
												{
													kind: "forecast",
													forecastId: f.forecastId,
													label: f.description,
													amount: formatMoney(f.amount),
												},
												e,
											);
										}}
										title={
											locked
												? (lockReason ?? "bloqueada")
												: !isSelected
													? "clique para selecionar; arraste para outro mês"
													: "Ctrl+←/→ move mês; Ctrl+M abre seletor"
										}
										style={{
											display: "flex",
											justifyContent: "space-between",
											alignItems: "center",
											gap: 10,
											padding: "6px 10px",
											borderRadius: "var(--radius-sm)",
											border: isSelected
												? "1px solid var(--purple)"
												: "1px dashed var(--border)",
											background:
												f.kind === "manual" ? "transparent" : "var(--surface)",
											cursor: locked ? "default" : "grab",
											opacity: isDragging || isMoving ? 0.35 : 1,
											touchAction: "none",
											userSelect: "none",
											transition: "opacity 150ms, border-color 120ms",
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
									onClick={setOut}
								>
									saída
								</ToggleBtn>
								<ToggleBtn
									active={!outflow}
									color="var(--green)"
									onClick={setIn}
								>
									entrada
								</ToggleBtn>
								<input
									inputMode="decimal"
									placeholder="0,00"
									value={amount}
									onChange={(e) => setAmount(e.target.value)}
									onKeyDown={handleKeyDown}
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
										opacity: !description.trim() || !amount.trim() ? 0.4 : 1,
									}}
								>
									adicionar →
								</button>
								<button onClick={closeAdd} className="mono" style={pillStyle}>
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
						<button onClick={openAdd} className="mono" style={addBtnStyle}>
							+ nova previsão
						</button>
					</motion.div>
				)}
			</AnimatePresence>

			{/* Month picker popover for keyboard move (Ctrl+M) */}
			<AnimatePresence>
				{pickerOpen && selectedId != null && (
					<motion.div
						initial={{ opacity: 0, scale: 0.96 }}
						animate={{ opacity: 1, scale: 1 }}
						exit={{ opacity: 0, scale: 0.96 }}
						transition={{ duration: 0.12 }}
						style={{
							marginTop: 8,
							padding: 10,
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-sm)",
							background: "var(--surface)",
						}}
					>
						<div
							className="mono"
							style={{
								fontSize: 11,
								color: "var(--muted)",
								marginBottom: 6,
							}}
						>
							mover previsão para:
						</div>
						<div
							style={{
								display: "flex",
								flexWrap: "wrap",
								gap: 4,
								marginBottom: 6,
							}}
						>
							{allowedMonths.map((m) => {
								const isCurrent = m.month === month;
								return (
									<button
										key={m.month}
										onClick={() => doMove(selectedId, m.month)}
										className="mono"
										style={{
											...pillStyle,
											color: isCurrent ? "var(--cyan)" : "var(--white)",
											borderColor: isCurrent ? "var(--cyan)" : "var(--border)",
										}}
									>
										{m.label}
									</button>
								);
							})}
						</div>
						<button
							onClick={() => setPickerOpen(false)}
							className="mono"
							style={pillStyle}
						>
							cancelar
						</button>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
};

// ── Filter bar ─────────────────────────────────────────────────────────────

const FilterDivider = () => (
	<span
		aria-hidden
		style={{
			width: 1,
			alignSelf: "stretch",
			minHeight: 20,
			background: "var(--border)",
			margin: "0 2px",
		}}
	/>
);

const FilterBar = ({
	ui,
	textInput,
	setUi,
	onTextInput,
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
		uncategorizedOnly: boolean;
	};
	textInput: string;
	setUi: (patch: Partial<typeof ui>) => void;
	onTextInput: (v: string) => void;
	owners: string[];
	accounts: ReadonlyArray<{ id: string; label: string; owner: string }>;
	hasFilters: boolean;
}) => {
	const handleTextChange = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) => onTextInput(e.target.value),
		[onTextInput],
	);

	const handleCategoryChange = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) =>
			setUi({ categoryFilter: e.target.value || null }),
		[setUi],
	);

	const handleAccountChange = useCallback(
		(e: React.ChangeEvent<HTMLSelectElement>) =>
			setUi({ accountFilter: e.target.value || null }),
		[setUi],
	);

	const handleOwnerChange = useCallback(
		(e: React.ChangeEvent<HTMLSelectElement>) =>
			setUi({ ownerFilter: e.target.value || null }),
		[setUi],
	);

	const toggleInstallments = useCallback(
		() => setUi({ installmentsOnly: !ui.installmentsOnly }),
		[setUi, ui.installmentsOnly],
	);

	const toggleSubscriptions = useCallback(
		() => setUi({ subscriptionsOnly: !ui.subscriptionsOnly }),
		[setUi, ui.subscriptionsOnly],
	);

	const toggleUnreviewed = useCallback(
		() => setUi({ unreviewedOnly: !ui.unreviewedOnly }),
		[setUi, ui.unreviewedOnly],
	);

	const toggleUncategorized = useCallback(
		() => setUi({ uncategorizedOnly: !ui.uncategorizedOnly }),
		[setUi, ui.uncategorizedOnly],
	);

	const clearFilters = useCallback(
		() =>
			setUi({
				textFilter: null,
				categoryFilter: null,
				accountFilter: null,
				ownerFilter: null,
				installmentsOnly: false,
				subscriptionsOnly: false,
				unreviewedOnly: false,
				uncategorizedOnly: false,
			}),
		[setUi],
	);

	return (
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
					value={textInput}
					onChange={handleTextChange}
					className="mono"
					style={{ ...inputStyle, paddingLeft: 26, width: "100%" }}
					aria-label="busca textual"
				/>
			</div>

			<FilterDivider />

			{/* Structural filters */}
			<input
				list="phai-cats"
				placeholder="categoria…"
				value={ui.categoryFilter ?? ""}
				onChange={handleCategoryChange}
				className="mono"
				style={{ ...inputStyle, color: "var(--cyan)", width: 150 }}
				aria-label="filtrar por categoria"
			/>

			{/* Account filter */}
			{accounts.length > 0 && (
				<select
					value={ui.accountFilter ?? ""}
					onChange={handleAccountChange}
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
					onChange={handleOwnerChange}
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

			<FilterDivider />

			{/* Quick action chips */}
			<ToggleBtn
				active={ui.installmentsOnly}
				color="var(--amber)"
				onClick={toggleInstallments}
			>
				parcelas
			</ToggleBtn>
			<ToggleBtn
				active={ui.subscriptionsOnly}
				color="var(--cyan)"
				onClick={toggleSubscriptions}
			>
				assinaturas
			</ToggleBtn>
			<ToggleBtn
				active={ui.uncategorizedOnly}
				color="var(--rose)"
				onClick={toggleUncategorized}
			>
				sem categoria
			</ToggleBtn>
			<ToggleBtn
				active={ui.unreviewedOnly}
				color="var(--purple)"
				onClick={toggleUnreviewed}
			>
				não revisadas
			</ToggleBtn>

			{/* Clear filters */}
			{hasFilters && (
				<button
					onClick={clearFilters}
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
};

const FilterSummary = ({
	count,
	saidas,
	entradas,
	selectedCount,
}: {
	count: number;
	saidas: number;
	entradas: number;
	selectedCount?: number;
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
		<span>
			{count} transação{count !== 1 ? "ões" : ""}
		</span>
		{selectedCount != null && selectedCount > 0 && (
			<span style={{ color: "var(--purple)" }}>
				{selectedCount} selecionada{selectedCount !== 1 ? "s" : ""}
			</span>
		)}
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
				líquido {entradas - saidas >= 0 ? "+" : ""}
				{formatMoneyNumber(entradas - saidas)}
			</span>
		)}
	</div>
);

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
