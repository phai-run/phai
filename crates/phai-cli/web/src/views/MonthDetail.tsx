import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { TxRow } from "../components/TxRow";
import { CategoryPicker } from "../components/CategoryPicker";
import {
	groupHierarchical,
	toHierarchicalArray,
	type HierarchicalParentGroup,
	type HierarchicalSubGroup,
} from "../lib/derivations";
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

	// Group hierarchically: parent → sub
	const groups = useMemo(() => {
		return toHierarchicalArray(groupHierarchical(filtered, overlayById));
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [filtered, overlayById]);

	// Denominator for each expense category's share of the month's spending.
	const expenseTotal = useMemo(
		() => groups.expenses.reduce((s, p) => s + Math.abs(p.total), 0),
		[groups],
	);

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

	// ── Keyboard selection / batch selection ────────────────────────────
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
	const [focusedIdx, setFocusedIdx] = useState<number>(-1);
	const lastClickedIdx = useRef<number>(-1);

	// Quick category picker state
	const [quickPicker, setQuickPicker] = useState<{
		anchorRect: DOMRect;
	} | null>(null);

	// Recently used categories (track across quick picks)
	const [recentCats, setRecentCats] = useState<string[]>([]);

	// Drag-to-recategorize setup
	const { startDrag, registerTarget } = useDnd();

	// Build a flat index list of visible (filtered) transactions for keyboard nav
	const flatTxs = useMemo(() => {
		const result: TxView[] = [];
		for (const parent of groups.expenses) {
			for (const sub of parent.subs) {
				for (const tx of sub.txs) {
					result.push(tx);
				}
			}
		}
		for (const tx of groups.income) {
			result.push(tx);
		}
		return result;
	}, [groups]);

	// Clear selection when filtered list changes significantly
	const flatIds = useMemo(() => flatTxs.map((t) => t.id), [flatTxs]);
	useEffect(() => {
		setSelectedIds(new Set());
		setFocusedIdx(-1);
		lastClickedIdx.current = -1;
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [flatIds.length]); // reset only on count change, not identity

	const focusedTx =
		focusedIdx >= 0 && flatTxs[focusedIdx] ? flatTxs[focusedIdx] : null;

	// ── Click handler with modifier support ─────────────────────────────
	const handleTxClick = useCallback(
		(tx: TxView, e: React.MouseEvent) => {
			const idx = flatTxs.findIndex((t) => t.id === tx.id);
			if (idx === -1) {
				onEdit(tx);
				return;
			}

			if (e.shiftKey && lastClickedIdx.current >= 0) {
				// Shift+click: select range
				const start = Math.min(lastClickedIdx.current, idx);
				const end = Math.max(lastClickedIdx.current, idx);
				const rangeIds = flatTxs.slice(start, end + 1).map((t) => t.id);
				setSelectedIds(new Set(rangeIds));
				setFocusedIdx(idx);
			} else if (e.ctrlKey || e.metaKey) {
				// Ctrl/Cmd+click: toggle selection
				setSelectedIds((prev) => {
					const next = new Set(prev);
					if (next.has(tx.id)) next.delete(tx.id);
					else next.add(tx.id);
					return next;
				});
				setFocusedIdx(idx);
				lastClickedIdx.current = idx;
			} else {
				// Plain click: select single, open for editing
				setSelectedIds(new Set([tx.id]));
				setFocusedIdx(idx);
				lastClickedIdx.current = idx;
				onEdit(tx);
			}
		},
		[flatTxs, onEdit],
	);

	// ── Keyboard handler ────────────────────────────────────────────────
	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (modalTx) return; // don't intercept when modal is open
			if (quickPicker) return; // don't intercept when picker is open

			const target = e.target as HTMLElement;
			// Don't intercept when user is typing in input/textarea/select
			if (
				target.tagName === "INPUT" ||
				target.tagName === "TEXTAREA" ||
				target.tagName === "SELECT"
			)
				return;

			switch (true) {
				case e.key === "ArrowDown":
					e.preventDefault();
					setFocusedIdx((i) => {
						const next = Math.min(i + 1, flatTxs.length - 1);
						if (e.shiftKey) {
							// Extend selection
							const anchor =
								lastClickedIdx.current >= 0 ? lastClickedIdx.current : Math.max(i, 0);
							const start = Math.min(anchor, next);
							const end = Math.max(anchor, next);
							const rangeIds = flatTxs.slice(start, end + 1).map((t) => t.id);
							setSelectedIds(new Set(rangeIds));
							lastClickedIdx.current = anchor;
						} else {
							setSelectedIds(new Set(flatTxs[next] ? [flatTxs[next].id] : []));
							lastClickedIdx.current = next;
						}
						return next;
					});
					break;
				case e.key === "ArrowUp":
					e.preventDefault();
					setFocusedIdx((i) => {
						const next = Math.max(i - 1, 0);
						if (e.shiftKey) {
							const anchor =
								lastClickedIdx.current >= 0 ? lastClickedIdx.current : Math.max(i, 0);
							const start = Math.min(anchor, next);
							const end = Math.max(anchor, next);
							const rangeIds = flatTxs.slice(start, end + 1).map((t) => t.id);
							setSelectedIds(new Set(rangeIds));
							lastClickedIdx.current = anchor;
						} else {
							setSelectedIds(new Set(flatTxs[next] ? [flatTxs[next].id] : []));
							lastClickedIdx.current = next;
						}
						return next;
					});
					break;
				case e.key === "Enter": {
					e.preventDefault();
					if (focusedIdx >= 0 && flatTxs[focusedIdx]) {
						onEdit(flatTxs[focusedIdx]);
					}
					break;
				}
				case (e.ctrlKey || e.metaKey) && e.key === "k": {
					e.preventDefault();
					if (focusedIdx >= 0) {
						// Find the focused row element to anchor the picker
						const txId = flatTxs[focusedIdx]?.id;
						if (txId) {
							const el = document.querySelector(`[data-tx-id="${txId}"]`);
							const rect =
								el?.getBoundingClientRect() ?? new DOMRect(100, 200, 100, 40);
							setQuickPicker({ anchorRect: rect });
						}
					}
					break;
				}
				case e.key === "Escape":
					e.preventDefault();
					if (selectedIds.size > 0) {
						setSelectedIds(new Set());
						setFocusedIdx(-1);
						lastClickedIdx.current = -1;
					} else {
						setModalTx(null);
					}
					break;
			}
		},
		[modalTx, quickPicker, flatTxs, focusedIdx, selectedIds, onEdit],
	);

	const handleOpenModal = useCallback((tx: TxView) => setModalTx(tx), []);
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

	// ── Quick category picker actions ───────────────────────────────────
	const handleQuickCategory = useCallback(
		(categoryId: string) => {
			const idsToUpdate =
				selectedIds.size > 0
					? Array.from(selectedIds)
					: focusedTx
						? [focusedTx.id]
						: [];
			for (const txId of idsToUpdate) {
				submit(txId, {
					description: null,
					merchantName: null,
					purpose: null,
					categoryId,
				});
			}
			// Track recent
			setRecentCats((prev) => {
				const next = [categoryId, ...prev.filter((c) => c !== categoryId)];
				return next.slice(0, 10);
			});
			setQuickPicker(null);
			setSelectedIds(new Set());
		},
		[selectedIds, focusedTx, submit],
	);

	const handleClosePicker = useCallback(() => setQuickPicker(null), []);

	// ── Drag-to-recategorize ────────────────────────────────────────────
	const handleTxDragStart = useCallback(
		(tx: TxView, e: React.PointerEvent) => {
			startDrag(
				{
					kind: "transaction",
					txId: tx.id,
					categoryId: effectiveCat(tx),
					label: tx.description ?? tx.merchantName ?? tx.rawDescription,
					amount: formatMoney(tx.amount),
				},
				e,
			);
		},
		[startDrag, effectiveCat],
	);

	// Register category headers as drop targets
	const registerDropTarget = useCallback(
		(categoryId: string | null, el: HTMLElement | null) => {
			if (!el) return undefined;
			const targetId = `category:${categoryId ?? "__uncategorized__"}`;
			const target = {
				id: targetId,
				getRect: () => el.getBoundingClientRect(),
				onDrop: (payload: import("../lib/dnd").DragPayload) => {
					if (payload.kind !== "transaction" || !payload.txId) return;
					submit(payload.txId, {
						description: null,
						merchantName: null,
						purpose: null,
						categoryId,
					});
				},
			};
			return registerTarget(target);
		},
		[registerTarget, submit],
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
		<div style={{ paddingBottom: 80 }} onKeyDown={handleKeyDown}>
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
							selectedCount={selectedIds.size}
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
						onEdit={handleOpenModal}
						selectedIds={selectedIds}
						focusedTxId={focusedTx?.id ?? null}
						onTxClick={handleTxClick}
						onTxDragStart={handleTxDragStart}
						registerDropTarget={registerDropTarget}
					/>
				)}

				{/* Expenses — hierarchical: parent → sub */}
				{groups.expenses.map((parent) => (
					<HierarchicalCategoryGroup
						key={parent.parent}
						parent={parent}
						monthTotal={expenseTotal}
						overlayById={overlayById}
						onEdit={handleOpenModal}
						selectedIds={selectedIds}
						focusedTxId={focusedTx?.id ?? null}
						onTxClick={handleTxClick}
						onTxDragStart={handleTxDragStart}
						registerDropTarget={registerDropTarget}
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
									(t.merchantName && t.merchantName === modalTx.merchantName)),
						)}
						overlayById={overlayById}
						onSubmit={(patch) => handleModalSubmit(modalTx.id, patch)}
						onClose={handleCloseModal}
					/>
				)}
			</AnimatePresence>

			{/* ── Quick category picker (Ctrl/Cmd+K) ── */}
			<AnimatePresence>
				{quickPicker && (
					<CategoryPicker
						categories={categoryIds}
						recentCategories={recentCats}
						anchorRect={quickPicker.anchorRect}
						selectedCount={selectedIds.size || 1}
						onSelect={handleQuickCategory}
						onClose={handleClosePicker}
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

// ── Hierarchical category group ────────────────────────────────────────────

const HierarchicalCategoryGroup = ({
	parent,
	monthTotal,
	overlayById,
	onEdit,
	selectedIds,
	focusedTxId,
	onTxClick,
	onTxDragStart,
	registerDropTarget,
}: {
	parent: HierarchicalParentGroup;
	monthTotal: number;
	overlayById: Map<
		string,
		{
			categoryId: string | null;
			description: string | null;
			merchantName: string | null;
			purpose: string | null;
		}
	>;
	onEdit: (tx: TxView) => void;
	selectedIds: Set<string>;
	focusedTxId: string | null;
	onTxClick: (tx: TxView, e: React.MouseEvent) => void;
	onTxDragStart: (tx: TxView, e: React.PointerEvent) => void;
	registerDropTarget: (
		categoryId: string | null,
		el: HTMLElement | null,
	) => (() => void) | undefined;
}) => {
	const [expanded, setExpanded] = useState(false);
	const installmentTxs = parent.subs.flatMap((s) =>
		s.txs.filter((t) => t.isInstallment === 1),
	);
	const isUncategorized = parent.parent === "—";

	// Register parent category as a drop target
	const headerRef = useRef<HTMLButtonElement>(null);
	useEffect(() => {
		if (headerRef.current) {
			const categoryId = isUncategorized ? null : parent.parent;
			return registerDropTarget(categoryId, headerRef.current);
		}
		return undefined;
	}, [isUncategorized, parent.parent, registerDropTarget]);

	return (
		<div
			style={{
				marginBottom: 8,
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-md)",
				overflow: "hidden",
			}}
		>
			{/* Parent header */}
			<button
				ref={headerRef}
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
						color: isUncategorized ? "var(--muted)" : "var(--white)",
						overflow: "hidden",
						textOverflow: "ellipsis",
						whiteSpace: "nowrap",
					}}
				>
					{isUncategorized ? "— sem categoria" : parent.parent}
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
						color: "var(--rose)",
						whiteSpace: "nowrap",
					}}
				>
					{formatMoney(String(parent.total))}
				</span>
				{monthTotal > 0 && (
					<span
						className="mono"
						style={{
							fontSize: 10,
							color: "var(--muted)",
							minWidth: 30,
							textAlign: "right",
						}}
					>
						{Math.round((Math.abs(parent.total) / monthTotal) * 100)}%
					</span>
				)}
				<span className="mono" style={{ fontSize: 10, color: "var(--muted2)" }}>
					{parent.count}
				</span>
			</button>

			{/* Body */}
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
							{parent.hasSubs ? (
								/* Subcategory tiles in a responsive grid; a tile expands
								   in place (full row) to reveal its transactions (N6). */
								<div
									style={{
										display: "grid",
										gridTemplateColumns:
											"repeat(auto-fill, minmax(220px, 1fr))",
										gap: 8,
										padding: 10,
									}}
								>
									{parent.subs.map((sub) => (
										<SubTile
											key={sub.sub ?? "_flat_"}
											sub={sub}
											parentLabel={parent.parent}
											overlayById={overlayById}
											onEdit={onEdit}
											selectedIds={selectedIds}
											focusedTxId={focusedTxId}
											onTxClick={onTxClick}
											onTxDragStart={onTxDragStart}
											registerDropTarget={registerDropTarget}
										/>
									))}
								</div>
							) : (
								/* Flat parent — render txs directly (no sub header) */
								parent.subs[0]?.txs.map((tx) => {
										const o = overlayById.get(tx.id);
										return (
											<TxRow
												key={tx.id}
												tx={tx}
												overlay={o}
												onEdit={onEdit}
												isSelected={selectedIds.has(tx.id)}
												isFocused={focusedTxId === tx.id}
												onClick={onTxClick}
												showDragHandle
												onDragStart={onTxDragStart}
											/>
										);
									})
								)}
						</div>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
};

/** Subcategory tile: a compact card (label · total · count) that expands in
 *  place — spanning the full grid row — to reveal its transactions (N6). */
const SubTile = ({
	sub,
	parentLabel,
	overlayById,
	onEdit,
	selectedIds,
	focusedTxId,
	onTxClick,
	onTxDragStart,
	registerDropTarget,
}: {
	sub: HierarchicalSubGroup;
	parentLabel: string;
	overlayById: Map<
		string,
		{
			categoryId: string | null;
			description: string | null;
			merchantName: string | null;
			purpose: string | null;
		}
	>;
	onEdit: (tx: TxView) => void;
	selectedIds: Set<string>;
	focusedTxId: string | null;
	onTxClick: (tx: TxView, e: React.MouseEvent) => void;
	onTxDragStart: (tx: TxView, e: React.PointerEvent) => void;
	registerDropTarget: (
		categoryId: string | null,
		el: HTMLElement | null,
	) => (() => void) | undefined;
}) => {
	const [expanded, setExpanded] = useState(false);
	const isFlatSub = sub.sub === "—";
	const subLabel = isFlatSub ? "sem subcategoria" : sub.sub;
	const targetCategoryId =
		parentLabel === "—"
			? null
			: isFlatSub
				? parentLabel
				: `${parentLabel}:${sub.sub}`;
	const hasInstallment = sub.txs.some((t) => t.isInstallment === 1);

	// Register the tile as a drop target (compound category: parent:sub).
	const tileRef = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (tileRef.current) {
			return registerDropTarget(targetCategoryId, tileRef.current);
		}
		return undefined;
	}, [registerDropTarget, targetCategoryId]);

	return (
		<div
			ref={tileRef}
			className="lift"
			style={{
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-sm)",
				background: "var(--surface)",
				overflow: "hidden",
				// An expanded tile takes the whole row so its transactions list
				// gets full width regardless of where it sits in the grid.
				gridColumn: expanded ? "1 / -1" : "auto",
				alignSelf: "start",
			}}
		>
			{/* Tile header (click to expand/collapse) */}
			<button
				type="button"
				onClick={() => setExpanded((v) => !v)}
				style={{
					width: "100%",
					display: "flex",
					alignItems: "center",
					gap: 8,
					padding: "9px 11px",
					background: "transparent",
					border: "none",
					cursor: "pointer",
					textAlign: "left",
				}}
			>
				<span style={{ color: "var(--muted2)", fontSize: 10, minWidth: 9 }}>
					{expanded ? "▾" : "▸"}
				</span>
				<span
					style={{
						flex: 1,
						fontSize: 12,
						color: "var(--white)",
						overflow: "hidden",
						textOverflow: "ellipsis",
						whiteSpace: "nowrap",
					}}
				>
					{subLabel}
				</span>
				{hasInstallment && (
					<span
						className="mono"
						style={{ fontSize: 9, color: "var(--amber)" }}
						title="contém parcelas"
					>
						parc
					</span>
				)}
				<span
					className="mono"
					style={{
						fontSize: 11,
						fontWeight: 600,
						color: "var(--rose)",
						whiteSpace: "nowrap",
					}}
				>
					{formatMoney(String(sub.total))}
				</span>
				<span
					className="mono"
					style={{ fontSize: 10, color: "var(--muted2)", minWidth: 14, textAlign: "right" }}
				>
					{sub.count}
				</span>
			</button>

			{/* Transactions (revealed on expand) */}
			<AnimatePresence initial={false}>
				{expanded && (
					<motion.div
						initial={{ height: 0, opacity: 0 }}
						animate={{ height: "auto", opacity: 1 }}
						exit={{ height: 0, opacity: 0 }}
						transition={{ duration: 0.16, ease: "easeInOut" }}
						style={{ overflow: "hidden" }}
					>
						{sub.txs.map((tx) => {
							const o = overlayById.get(tx.id);
							return (
								<TxRow
									key={tx.id}
									tx={tx}
									overlay={o}
									onEdit={onEdit}
									isSelected={selectedIds.has(tx.id)}
									isFocused={focusedTxId === tx.id}
									onClick={onTxClick}
									showDragHandle
									onDragStart={onTxDragStart}
								/>
							);
						})}
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
};

// ── Category group ─────────────────────────────────────────────────────────

const CategoryGroup = ({
	label,
	isIncome,
	txs,
	overlayById,
	onEdit,
	selectedIds,
	focusedTxId,
	onTxClick,
	onTxDragStart,
	registerDropTarget,
}: {
	label: string;
	isIncome: boolean;
	txs: TxView[];
	overlayById: Map<
		string,
		{
			categoryId: string | null;
			description: string | null;
			merchantName: string | null;
			purpose: string | null;
		}
	>;
	onEdit: (tx: TxView) => void;
	selectedIds: Set<string>;
	focusedTxId: string | null;
	onTxClick: (tx: TxView, e: React.MouseEvent) => void;
	onTxDragStart: (tx: TxView, e: React.PointerEvent) => void;
	registerDropTarget: (
		categoryId: string | null,
		el: HTMLElement | null,
	) => (() => void) | undefined;
}) => {
	const [expanded, setExpanded] = useState(false);
	const total = sumAmounts(txs.map((t) => t.amount));
	const installmentTxs = useMemo(
		() => txs.filter((t) => t.isInstallment === 1),
		[txs],
	);

	const toggleExpanded = useCallback(() => setExpanded((v) => !v), []);

	// Register category as a drop target
	const headerRef = useRef<HTMLButtonElement>(null);
	useEffect(() => {
		if (!isIncome && headerRef.current) {
			return registerDropTarget(label, headerRef.current);
		}
		return undefined;
	}, [isIncome, label, registerDropTarget]);

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
				ref={headerRef}
				onClick={toggleExpanded}
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
				<span className="mono" style={{ fontSize: 10, color: "var(--muted2)" }}>
					{txs.length}
				</span>
			</button>

			{/* Transactions body — CSS grid transition instead of AnimatePresence */}
			<div
				style={{
					display: "grid",
					gridTemplateRows: expanded ? "1fr" : "0fr",
					transition: "grid-template-rows 0.2s ease, opacity 0.15s ease",
					opacity: expanded ? 1 : 0,
				}}
			>
				<div style={{ overflow: "hidden" }}>
					<div style={{ display: "flex", flexDirection: "column" }}>
						{txs.map((tx) => {
							const o = overlayById.get(tx.id);
							return (
								<TxRow
									key={tx.id}
									tx={tx}
									overlay={o}
									onEdit={onEdit}
									isSelected={selectedIds.has(tx.id)}
									isFocused={focusedTxId === tx.id}
									onClick={onTxClick}
									showDragHandle
									onDragStart={onTxDragStart}
								/>
							);
						})}
					</div>
				</div>
			</div>
		</div>
	);
};

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
		{
			categoryId: string | null;
			description: string | null;
			merchantName: string | null;
			purpose: string | null;
		}
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

	const applyBulk = useCallback(
		(patch: ReviewPatch) => {
			for (const id of selectedSimilar) {
				store.commit(
					events.reviewSubmitted({
						writeId: crypto.randomUUID(),
						transactionId: id,
						patch,
						submittedAt: Date.now(),
					}),
				);
			}
			setSelectedSimilar(new Set());
		},
		[selectedSimilar, store],
	);

	const handleToggle = useCallback((id: string) => {
		setSelectedSimilar((prev) => {
			const next = new Set(prev);
			if (next.has(id)) next.delete(id);
			else next.add(id);
			return next;
		});
	}, []);

	const handleSelectAll = useCallback(() => {
		setSelectedSimilar(new Set(similarTxs.map((t) => t.id)));
	}, [similarTxs]);

	const handleClearAll = useCallback(() => {
		setSelectedSimilar(new Set());
	}, []);

	const handleSave = useCallback(() => {
		const patch = {
			description: description.trim() || null,
			merchantName: merchantName.trim() || null,
			purpose: purpose.trim() || null,
			categoryId: category.trim() || null,
		};
		onSubmit(patch);
	}, [onSubmit, description, merchantName, purpose, category]);

	const currentPatch = useMemo(
		() => ({
			description: description.trim() || null,
			merchantName: merchantName.trim() || null,
			purpose: purpose.trim() || null,
			categoryId: category.trim() || null,
		}),
		[description, merchantName, purpose, category],
	);

	return (
		<>
			{/* Backdrop */}
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
									onSave={handleSave}
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
									onToggle={handleToggle}
									onSelectAll={handleSelectAll}
									onClearAll={handleClearAll}
									onApplyBulk={applyBulk}
									patch={currentPatch}
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
	patch,
}: {
	similarTxs: ReadonlyArray<TxView>;
	overlayById: Map<
		string,
		{
			categoryId: string | null;
			description: string | null;
			merchantName: string | null;
			purpose: string | null;
		}
	>;
	selected: Set<string>;
	onToggle: (id: string) => void;
	onSelectAll: () => void;
	onClearAll: () => void;
	onApplyBulk: (patch: ReviewPatch) => void;
	patch: ReviewPatch;
}) => {
	const handleApply = useCallback(() => {
		onApplyBulk(patch);
	}, [onApplyBulk, patch]);

	const hasPatch =
		patch.description != null ||
		patch.merchantName != null ||
		patch.purpose != null ||
		patch.categoryId != null;

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
						<button
							onClick={handleApply}
							disabled={!hasPatch}
							className="mono"
							style={{
								...pillStyle,
								background: "var(--purple)",
								color: "#fff",
								opacity: !hasPatch ? 0.4 : 1,
							}}
						>
							aplicar campos em {selected.size} →
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
									background: isSelected ? "var(--purple)" : "transparent",
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
