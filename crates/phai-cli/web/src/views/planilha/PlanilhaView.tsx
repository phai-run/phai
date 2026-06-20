import { queryDb } from "@livestore/livestore";
import { useQuery, useStore, useClientDocument } from "@livestore/react";
import {
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { AnimatePresence } from "framer-motion";
import { events, tables } from "../../livestore/schema";
import { CategoryPicker } from "../../components/CategoryPicker";
import {
	TransactionModal,
	type ReviewPatch,
} from "../../components/TransactionModal";
import { TierBadge } from "../../components/TierBadge";
import { categoryEmoji } from "../../lib/categoryEmoji";
import { formatMoneyNumber, isNegative, toCents } from "../../lib/format";
import {
	buildAccountMap,
	buildOverlayMap,
	commitmentTier,
	COMMITMENT_TIER_LABELS,
	COMMITMENT_TIERS,
	effectiveCategory,
	effectiveTx,
	filterTransactions,
	fixedCategoriesFromForecasts,
	hasActiveFilters,
	sheetLabel,
	sortForSheet,
	transactionsForMonth,
	type CommitmentTier,
	type SheetSort,
	type SheetSortKey,
	type TxView,
} from "../../lib/derivations";

const txAll$ = queryDb(tables.transactions.orderBy("postedAt", "desc"));
const overlay$ = queryDb(tables.reviewOverlay);
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));
const accounts$ = queryDb(tables.accounts.orderBy("label", "asc"));
const forecasts$ = queryDb(tables.forecasts);

const COLUMNS: Array<{ key: SheetSortKey; label: string; width?: string }> = [
	{ key: "date", label: "date", width: "64px" },
	{ key: "description", label: "description" },
	{ key: "account", label: "account", width: "120px" },
	{ key: "category", label: "category", width: "220px" },
	{ key: "amount", label: "amount", width: "136px" },
];

const CSV_COLUMNS = [
	"transaction_id",
	"posted_at",
	"description",
	"merchant_name",
	"purpose",
	"account",
	"category_id",
	"amount",
	"installment",
] as const;

const csvCell = (value: string | null | undefined): string => {
	const text = value ?? "";
	if (/[",\n\r]/.test(text)) {
		return `"${text.replaceAll('"', '""')}"`;
	}
	return text;
};

export const csvAmountCell = (amount: string): string => {
	const cents = toCents(amount);
	const sign = cents < 0 ? "-" : "";
	const absolute = Math.abs(cents);
	const whole = Math.trunc(absolute / 100);
	const fraction = String(absolute % 100).padStart(2, "0");
	return `${sign}${whole},${fraction}`;
};

export const sheetRowsCsv = (
	rows: ReadonlyArray<TxView>,
	accountMap: Map<string, { label: string }>,
): string => {
	const lines = [CSV_COLUMNS.join(",")];
	for (const tx of rows) {
		const account = accountMap.get(tx.accountId)?.label ?? tx.accountId;
		lines.push(
			[
				tx.id,
				tx.postedAt,
				sheetLabel(tx),
				tx.merchantName,
				tx.purpose,
				account,
				tx.categoryId,
				csvAmountCell(tx.amount),
				tx.installmentMarker,
			]
				.map(csvCell)
				.join(","),
		);
	}
	return `${lines.join("\n")}\n`;
};

export const sheetAmountLabel = (amount: string): string => {
	const cents = toCents(amount);
	const value = Math.abs(cents) / 100;
	const formatted = formatMoneyNumber(value);
	return cents < 0 ? `(${formatted})` : formatted;
};

export const sheetSignedTotal = (rows: ReadonlyArray<TxView>): number =>
	rows.reduce((total, tx) => total + toCents(tx.amount), 0) / 100;

const downloadCsv = (filename: string, csv: string) => {
	const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
	const url = URL.createObjectURL(blob);
	const link = document.createElement("a");
	link.href = url;
	link.download = filename;
	document.body.append(link);
	link.click();
	link.remove();
	URL.revokeObjectURL(url);
};

/**
 * Planilha — the spreadsheet view of a month. Every transaction is one flat
 * row: sortable columns, inline category editing (click the chip or press
 * Enter), shift/cmd multi-select with a bulk-apply bar, and live totals for
 * the current filter. Edits go through the same `reviewSubmitted` event the
 * grouped view uses (optimistic overlay + background flush to the bridge).
 */
export const PlanilhaView = ({ month }: { month: string }) => {
	const { store } = useStore();
	const [ui, setUi] = useClientDocument(tables.ui);
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const overlay = useQuery(overlay$);
	const categories = useQuery(categories$);
	const accounts = useQuery(accounts$);
	const forecasts = useQuery(forecasts$);

	const overlayMap = useMemo(() => buildOverlayMap(overlay), [overlay]);
	const accountMap = useMemo(() => buildAccountMap(accounts), [accounts]);
	const categoryIds = useMemo(() => categories.map((c) => c.id), [categories]);
	const fixedCategories = useMemo(
		() => fixedCategoriesFromForecasts(forecasts),
		[forecasts],
	);

	const [sort, setSort] = useState<SheetSort>({ key: "date", dir: -1 });
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
	const [focusedIdx, setFocusedIdx] = useState(-1);
	const lastClickedIdx = useRef(-1);
	const [picker, setPicker] = useState<{
		anchorRect: DOMRect;
		targetIds: string[];
	} | null>(null);
	const [recentCats, setRecentCats] = useState<string[]>([]);
	const [modalTx, setModalTx] = useState<TxView | null>(null);
	const tableRef = useRef<HTMLDivElement>(null);

	const filters = useMemo(
		() => ({
			accountFilter: ui.accountFilter,
			ownerFilter: ui.ownerFilter,
			categoryFilter: ui.categoryFilter,
			textFilter: ui.textFilter,
			installmentsOnly: ui.installmentsOnly,
			subscriptionsOnly: ui.subscriptionsOnly,
			unreviewedOnly: ui.unreviewedOnly,
			tierFilter: (ui.tierFilter as CommitmentTier | null) ?? null,
		}),
		[
			ui.accountFilter,
			ui.ownerFilter,
			ui.categoryFilter,
			ui.textFilter,
			ui.installmentsOnly,
			ui.subscriptionsOnly,
			ui.unreviewedOnly,
			ui.tierFilter,
		],
	);

	const rows = useMemo(() => {
		// Bake the optimistic overlay in first so edited description/merchant/
		// category reflect everywhere (sheet, sums, sort), not just the modal.
		const monthTxs = transactionsForMonth(txRows, month).map((tx) =>
			effectiveTx(tx, overlayMap),
		);
		const filtered = filterTransactions(
			monthTxs,
			filters,
			overlayMap,
			accountMap,
			fixedCategories,
		).filter(
			(tx) =>
				!ui.uncategorizedOnly || (effectiveCategory(tx, overlayMap) ?? "") === "",
		);
		return sortForSheet(filtered, sort, overlayMap, accountMap);
	}, [
		txRows,
		month,
		filters,
		ui.uncategorizedOnly,
		overlayMap,
		accountMap,
		fixedCategories,
		sort,
	]);

	const hasSheetFilters = useMemo(
		() => hasActiveFilters(filters) || ui.uncategorizedOnly,
		[filters, ui.uncategorizedOnly],
	);

	// Reset selection when the month or the visible set changes size.
	const rowCount = rows.length;
	useEffect(() => {
		setSelectedIds(new Set());
		setFocusedIdx(-1);
		lastClickedIdx.current = -1;
	}, [month, rowCount]);

	const totals = useMemo(() => {
		let inCents = 0;
		let outCents = 0;
		for (const tx of rows) {
			const c = toCents(tx.amount);
			if (c < 0) outCents += -c;
			else inCents += c;
		}
		return {
			entradas: inCents / 100,
			saidas: outCents / 100,
			net: sheetSignedTotal(rows),
		};
	}, [rows]);

	const selectionTotal = useMemo(() => {
		let cents = 0;
		for (const tx of rows) if (selectedIds.has(tx.id)) cents += toCents(tx.amount);
		return cents / 100;
	}, [rows, selectedIds]);

	const handleExportCsv = useCallback(() => {
		downloadCsv(`phai-planilha-${month}.csv`, sheetRowsCsv(rows, accountMap));
	}, [month, rows, accountMap]);

	const applyCategory = useCallback(
		(categoryId: string, targetIds: string[]) => {
			for (const transactionId of targetIds) {
				store.commit(
					events.reviewSubmitted({
						writeId: crypto.randomUUID(),
						transactionId,
						patch: {
							description: null,
							merchantName: null,
							purpose: null,
							categoryId,
						},
						submittedAt: Date.now(),
					}),
				);
			}
			setRecentCats((prev) =>
				[categoryId, ...prev.filter((c) => c !== categoryId)].slice(0, 8),
			);
			setPicker(null);
			setSelectedIds(new Set());
		},
		[store],
	);

	// Bulk-set the commitment-tier override on the selection (ADR-0032);
	// `tier === ""` clears the override back to the derived tier.
	const applyTier = useCallback(
		(tier: CommitmentTier | "") => {
			for (const transactionId of selectedIds) {
				store.commit(
					events.reviewSubmitted({
						writeId: crypto.randomUUID(),
						transactionId,
						patch: {
							description: null,
							merchantName: null,
							purpose: null,
							categoryId: null,
							commitmentTier: tier,
						},
						submittedAt: Date.now(),
					}),
				);
			}
			setSelectedIds(new Set());
		},
		[store, selectedIds],
	);

	const openPickerFor = useCallback(
		(tx: TxView, anchor: HTMLElement) => {
			const ids =
				selectedIds.size > 1 && selectedIds.has(tx.id)
					? Array.from(selectedIds)
					: [tx.id];
			setPicker({ anchorRect: anchor.getBoundingClientRect(), targetIds: ids });
		},
		[selectedIds],
	);

	const handleRowClick = useCallback(
		(tx: TxView, idx: number, e: React.MouseEvent) => {
			if (e.shiftKey && lastClickedIdx.current >= 0) {
				const start = Math.min(lastClickedIdx.current, idx);
				const end = Math.max(lastClickedIdx.current, idx);
				setSelectedIds(new Set(rows.slice(start, end + 1).map((t) => t.id)));
			} else if (e.ctrlKey || e.metaKey) {
				setSelectedIds((prev) => {
					const next = new Set(prev);
					if (next.has(tx.id)) next.delete(tx.id);
					else next.add(tx.id);
					return next;
				});
				lastClickedIdx.current = idx;
			} else {
				// Plain click opens the same edit modal the categorias view uses;
				// selection stays on the checkbox / modifier clicks.
				lastClickedIdx.current = idx;
				setModalTx(tx);
			}
			setFocusedIdx(idx);
		},
		[rows],
	);

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

	// Same "similar" notion as the categorias view: shares the effective
	// category or the merchant. Restricted to the seeded window (all months).
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

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (picker || modalTx) return; // the popover/modal owns the keyboard
			switch (e.key) {
				case "ArrowDown":
				case "ArrowUp": {
					e.preventDefault();
					const delta = e.key === "ArrowDown" ? 1 : -1;
					setFocusedIdx((i) => {
						const next = Math.min(Math.max(i + delta, 0), rows.length - 1);
						tableRef.current
							?.querySelector(`[data-row-idx="${next}"]`)
							?.scrollIntoView({ block: "nearest" });
						return next;
					});
					break;
				}
				case " ": {
					e.preventDefault();
					const tx = rows[focusedIdx];
					if (!tx) break;
					setSelectedIds((prev) => {
						const next = new Set(prev);
						if (next.has(tx.id)) next.delete(tx.id);
						else next.add(tx.id);
						return next;
					});
					break;
				}
				case "Enter": {
					// Mirrors the plain click: open the full edit modal.
					const tx = rows[focusedIdx];
					if (!tx) break;
					e.preventDefault();
					setModalTx(tx);
					break;
				}
				case "k": {
					const tx = rows[focusedIdx];
					if (!tx) break;
					e.preventDefault();
					const el = tableRef.current?.querySelector(
						`[data-row-idx="${focusedIdx}"] [data-cat-chip]`,
					);
					if (el) openPickerFor(tx, el as HTMLElement);
					break;
				}
				case "Escape":
					setSelectedIds(new Set());
					break;
			}
		},
		[picker, modalTx, rows, focusedIdx, openPickerFor],
	);

	const toggleSort = (key: SheetSortKey) =>
		setSort((s) =>
			s.key === key ? { key, dir: s.dir === 1 ? -1 : 1 } : { key, dir: key === "date" || key === "amount" ? -1 : 1 },
		);

	const allVisibleSelected = rowCount > 0 && selectedIds.size === rowCount;

	return (
		<section aria-label={`Sheet for ${month}`}>
			<SheetFilterBar
				ui={ui}
				setUi={setUi}
				accounts={accounts}
				count={rowCount}
				hasActiveFilters={hasSheetFilters}
				filteredTotal={totals.net}
				onExportCsv={handleExportCsv}
			/>

			<div
				ref={tableRef}
				role="grid"
				aria-rowcount={rowCount}
				tabIndex={0}
				onKeyDown={handleKeyDown}
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-md)",
					overflow: "auto",
					maxHeight: "70vh",
					outline: "none",
					background: "var(--card)",
				}}
			>
				<table
					style={{
						width: "100%",
						// "separate": with collapsed borders, body cells z-fight the
						// sticky header and paint through it while scrolling.
						borderCollapse: "separate",
						borderSpacing: 0,
						fontSize: 14,
					}}
				>
					<thead>
						<tr className="mono">
							<th style={{ ...thStyle, width: 34 }}>
								<input
									type="checkbox"
									aria-label="select all"
									checked={allVisibleSelected}
									onChange={() =>
										setSelectedIds(
											allVisibleSelected
												? new Set()
												: new Set(rows.map((t) => t.id)),
										)
									}
								/>
							</th>
							{COLUMNS.map((col) => (
								<th
									key={col.key}
									style={{ ...thStyle, width: col.width }}
									aria-sort={
										sort.key === col.key
											? sort.dir === 1
												? "ascending"
												: "descending"
											: undefined
									}
								>
									<button
										onClick={() => toggleSort(col.key)}
										className="mono"
										style={{
											background: "transparent",
											border: "none",
											cursor: "pointer",
											color:
												sort.key === col.key ? "var(--purple)" : "var(--muted)",
											fontSize: 12,
											textTransform: "uppercase",
											letterSpacing: "0.06em",
											padding: 0,
											textAlign: col.key === "amount" ? "right" : "left",
											width: "100%",
										}}
									>
										{col.label}
										{sort.key === col.key ? (sort.dir === 1 ? " ↑" : " ↓") : ""}
									</button>
								</th>
							))}
							<th style={{ ...thStyle, width: 70 }}>installment</th>
						</tr>
					</thead>
					<tbody>
						{rows.map((tx, idx) => (
							<SheetRow
								key={tx.id}
								tx={tx}
								idx={idx}
								selected={selectedIds.has(tx.id)}
								focused={focusedIdx === idx}
								category={effectiveCategory(tx, overlayMap)}
								accountLabel={
									accountMap.get(tx.accountId)?.label ?? tx.accountId
								}
								tier={commitmentTier(tx, fixedCategories, overlayMap)}
								onClick={handleRowClick}
								onCategoryClick={openPickerFor}
							/>
						))}
					</tbody>
				</table>
				{rowCount === 0 && (
					<div
						className="mono"
						style={{
							padding: 24,
							textAlign: "center",
							color: "var(--muted)",
							fontSize: 13,
						}}
					>
						No transactions for the current filters.
					</div>
				)}
			</div>

			{/* Totals footer — the "sheet status bar". */}
			<div
				className="mono"
				style={{
					display: "flex",
					gap: 20,
					padding: "10px 4px",
					fontSize: 12,
					color: "var(--muted)",
					flexWrap: "wrap",
				}}
			>
				<span>{rowCount} transactions</span>
				<span style={{ color: "var(--green)" }}>
					income {formatMoneyNumber(totals.entradas)}
				</span>
				<span style={{ color: "var(--rose)" }}>
					expenses {formatMoneyNumber(totals.saidas)}
				</span>
				<span style={{ color: totals.net >= 0 ? "var(--green)" : "var(--rose)" }}>
					net {formatMoneyNumber(totals.net)}
				</span>
				<span style={{ marginLeft: "auto", opacity: 0.7 }}>
					↑↓ navigate · space select · Enter/click edit · k categorize ·
					shift+click range
				</span>
			</div>

			{/* Bulk-apply bar */}
			{selectedIds.size > 0 && (
				<div
					role="toolbar"
					aria-label="bulk actions"
					style={{
						position: "sticky",
						bottom: 12,
						display: "flex",
						alignItems: "center",
						gap: 12,
						background: "var(--ink, #15131f)",
						color: "#fff",
						borderRadius: "var(--radius-full)",
						padding: "10px 18px",
						boxShadow: "0 8px 24px rgba(21,19,31,0.25)",
						width: "fit-content",
						margin: "0 auto",
						zIndex: 10,
					}}
				>
					<span className="mono" style={{ fontSize: 12 }}>
						{selectedIds.size} selected ·{" "}
						{formatMoneyNumber(selectionTotal)}
					</span>
					<button
						onClick={(e) =>
							setPicker({
								anchorRect: (e.target as HTMLElement).getBoundingClientRect(),
								targetIds: Array.from(selectedIds),
							})
						}
						className="mono"
						style={{
							background: "var(--purple)",
							color: "#fff",
							border: "none",
							borderRadius: "var(--radius-full)",
							padding: "6px 14px",
							cursor: "pointer",
							fontSize: 12,
						}}
					>
						categorize
					</button>
					<span
						aria-hidden
						style={{
							width: 1,
							alignSelf: "stretch",
							background: "rgba(255,255,255,0.25)",
						}}
					/>
					<span className="mono" style={{ fontSize: 11, opacity: 0.7 }}>
						tier:
					</span>
					{COMMITMENT_TIERS.map((tier) => (
						<button
							key={tier}
							onClick={() => applyTier(tier)}
							className="mono"
							style={{
								background: "rgba(255,255,255,0.12)",
								color: "#fff",
								border: "1px solid rgba(255,255,255,0.35)",
								borderRadius: "var(--radius-full)",
								padding: "6px 12px",
								cursor: "pointer",
								fontSize: 12,
							}}
						>
							{COMMITMENT_TIER_LABELS[tier]}
						</button>
					))}
					<button
						onClick={() => applyTier("")}
						className="mono"
						title="clear tier override"
						style={{
							background: "transparent",
							color: "#fff",
							border: "1px solid rgba(255,255,255,0.35)",
							borderRadius: "var(--radius-full)",
							padding: "6px 10px",
							cursor: "pointer",
							fontSize: 12,
						}}
					>
						× tier
					</button>
					<button
						onClick={() => setSelectedIds(new Set())}
						className="mono"
						style={{
							background: "transparent",
							color: "#fff",
							border: "1px solid rgba(255,255,255,0.35)",
							borderRadius: "var(--radius-full)",
							padding: "6px 14px",
							cursor: "pointer",
							fontSize: 12,
						}}
					>
						clear
					</button>
				</div>
			)}

			{picker && (
				<CategoryPicker
					categories={categoryIds}
					recentCategories={recentCats}
					anchorRect={picker.anchorRect}
					selectedCount={picker.targetIds.length}
					onSelect={(cat) => applyCategory(cat, picker.targetIds)}
					onClose={() => setPicker(null)}
				/>
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
		</section>
	);
};

const thStyle: React.CSSProperties = {
	padding: "8px 10px",
	textAlign: "left",
	fontWeight: 500,
	// Sticky lives on each th, not the tr: with collapsed table borders some
	// engines skip painting a sticky row's background, so body rows scrolled
	// through it. The th needs its own opaque background.
	position: "sticky",
	top: 0,
	zIndex: 2,
	background: "var(--card)",
	boxShadow: "0 1px 0 var(--border)",
};

const SheetRow = ({
	tx,
	idx,
	selected,
	focused,
	category,
	accountLabel,
	tier,
	onClick,
	onCategoryClick,
}: {
	tx: TxView;
	idx: number;
	selected: boolean;
	focused: boolean;
	category: string | null;
	accountLabel: string;
	tier: CommitmentTier;
	onClick: (tx: TxView, idx: number, e: React.MouseEvent) => void;
	onCategoryClick: (tx: TxView, anchor: HTMLElement) => void;
}) => {
	const negative = isNegative(tx.amount);
	return (
		<tr
			data-row-idx={idx}
			aria-selected={selected}
			onClick={(e) => onClick(tx, idx, e)}
			style={{
				cursor: "default",
				background: selected
					? "var(--purple-50, rgba(124,93,250,0.08))"
					: undefined,
				boxShadow: focused ? "inset 2px 0 0 var(--purple)" : undefined,
				userSelect: "none",
			}}
		>
			<td style={tdStyle} onClick={(e) => e.stopPropagation()}>
				<input
					type="checkbox"
					aria-label={`select ${sheetLabel(tx)}`}
					checked={selected}
					onChange={(e) =>
						onClick(tx, idx, {
							...e,
							ctrlKey: true,
							metaKey: true,
							shiftKey: false,
						} as unknown as React.MouseEvent)
					}
				/>
			</td>
			<td className="mono" style={{ ...tdStyle, color: "var(--muted)" }}>
				{tx.postedAt.slice(8, 10)}/{tx.postedAt.slice(5, 7)}
			</td>
			<td style={{ ...tdStyle, maxWidth: 380 }} title={tx.rawDescription}>
				<div
					style={{
						display: "flex",
						alignItems: "center",
						gap: 6,
						overflow: "hidden",
					}}
				>
					{/* Tiers describe controllability of spending — income has none. */}
					{negative && <TierBadge tier={tier} compact />}
					<span
						style={{
							overflow: "hidden",
							textOverflow: "ellipsis",
							whiteSpace: "nowrap",
						}}
					>
						{sheetLabel(tx)}
					</span>
				</div>
				{tx.purpose && (
					<div
						className="mono"
						style={{ fontSize: 11, color: "var(--muted)" }}
					>
						{tx.purpose}
					</div>
				)}
			</td>
			<td className="mono" style={{ ...tdStyle, fontSize: 12, color: "var(--muted)" }}>
				{accountLabel}
			</td>
			<td style={tdStyle} onClick={(e) => e.stopPropagation()}>
				<button
					data-cat-chip
					onClick={(e) => onCategoryClick(tx, e.currentTarget)}
					className="mono"
					title="change category (Enter)"
					style={{
						background: category ? "var(--chip, #f1eefc)" : "transparent",
						color: category ? "var(--purple)" : "var(--amber)",
						border: category
							? "1px solid transparent"
							: "1px dashed var(--amber)",
						borderRadius: "var(--radius-full)",
						padding: "3px 10px",
						cursor: "pointer",
						fontSize: 12,
						maxWidth: 220,
						overflow: "hidden",
						textOverflow: "ellipsis",
						whiteSpace: "nowrap",
					}}
				>
					{category
						? `${categoryEmoji(category, !negative)} ${category}`
						: "❓ uncategorized"}
				</button>
			</td>
			<td
				className="mono"
				style={{
					...tdStyle,
					textAlign: "right",
					color: negative ? "var(--rose)" : "var(--green)",
					fontVariantNumeric: "tabular-nums",
					whiteSpace: "nowrap",
				}}
			>
				{sheetAmountLabel(tx.amount)}
			</td>
			<td className="mono" style={{ ...tdStyle, fontSize: 12, color: "var(--muted)" }}>
				{tx.installmentMarker ?? ""}
			</td>
		</tr>
	);
};

const tdStyle: React.CSSProperties = {
	padding: "6px 10px",
	verticalAlign: "top",
	// Row separator lives on the td: tr borders don't render with
	// border-collapse: separate (which the sticky header requires).
	borderBottom: "1px solid var(--border)",
};

interface SheetFilterState {
	textFilter: string | null;
	accountFilter: string | null;
	ownerFilter: string | null;
	categoryFilter: string | null;
	uncategorizedOnly: boolean;
	unreviewedOnly: boolean;
	installmentsOnly: boolean;
	subscriptionsOnly: boolean;
	tierFilter: string | null;
}

/** Compact filter strip shared with the grouped view via the ui document. */
const SheetFilterBar = ({
	ui,
	setUi,
	accounts,
	count,
	hasActiveFilters,
	filteredTotal,
	onExportCsv,
}: {
	ui: SheetFilterState;
	setUi: (patch: Partial<SheetFilterState>) => void;
	accounts: ReadonlyArray<{ id: string; label: string }>;
	count: number;
	hasActiveFilters: boolean;
	filteredTotal: number;
	onExportCsv: () => void;
}) => {
	const chip = (active: boolean): React.CSSProperties => ({
		background: active ? "var(--purple)" : "transparent",
		color: active ? "#fff" : "var(--muted)",
		border: `1px solid ${active ? "var(--purple)" : "var(--border)"}`,
		borderRadius: "var(--radius-full)",
		padding: "4px 12px",
		cursor: "pointer",
		fontSize: 12,
		whiteSpace: "nowrap",
		flexShrink: 0,
	});
	// Controllability tiers (ADR-0030): single-select, distinct colours.
	const tierColor: Record<CommitmentTier, string> = {
		locked: "#9a9aae",
		cancellable: "var(--amber)",
		variable: "var(--green)",
	};
	const tierChip = (tier: CommitmentTier): React.CSSProperties => {
		const active = ui.tierFilter === tier;
		const c = tierColor[tier];
		return {
			background: active ? c : "transparent",
			color: active ? "#1a1a1a" : "var(--muted)",
			border: `1px solid ${active ? c : "var(--border)"}`,
			borderRadius: "var(--radius-full)",
			padding: "4px 12px",
			cursor: "pointer",
			fontSize: 12,
		};
	};
	return (
		<div
			style={{
				display: "flex",
				gap: 8,
				alignItems: "center",
				flexWrap: "wrap",
				padding: "12px 0",
			}}
		>
			<input
				type="search"
				placeholder={`search ${count} transactions…`}
				value={ui.textFilter ?? ""}
				onChange={(e) => setUi({ textFilter: e.target.value || null })}
				className="mono"
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-full)",
					padding: "6px 14px",
					fontSize: 12,
					minWidth: 220,
					background: "var(--card)",
				}}
			/>
			<select
				aria-label="filter by account"
				value={ui.accountFilter ?? ""}
				onChange={(e) => setUi({ accountFilter: e.target.value || null })}
				className="mono"
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-full)",
					padding: "6px 10px",
					fontSize: 12,
					background: "var(--card)",
				}}
			>
				<option value="">all accounts</option>
				{accounts.map((a) => (
					<option key={a.id} value={a.id}>
						{a.label}
					</option>
				))}
			</select>
			<button
				className="mono"
				style={chip(ui.uncategorizedOnly)}
				onClick={() => setUi({ uncategorizedOnly: !ui.uncategorizedOnly })}
			>
				uncategorized
			</button>
			<button
				className="mono"
				style={chip(ui.unreviewedOnly)}
				onClick={() => setUi({ unreviewedOnly: !ui.unreviewedOnly })}
			>
				unreviewed
			</button>
			<button
				className="mono"
				style={chip(ui.installmentsOnly)}
				onClick={() => setUi({ installmentsOnly: !ui.installmentsOnly })}
			>
				installments
			</button>
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
			{COMMITMENT_TIERS.map((tier) => (
				<button
					key={tier}
					className="mono"
					style={tierChip(tier)}
					aria-pressed={ui.tierFilter === tier}
					onClick={() =>
						setUi({ tierFilter: ui.tierFilter === tier ? null : tier })
					}
				>
					{COMMITMENT_TIER_LABELS[tier]}
				</button>
			))}
			{hasActiveFilters && (
				<div
					className="mono"
					aria-live="polite"
					style={{
						display: "flex",
						alignItems: "center",
						gap: 8,
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-full)",
						padding: "5px 12px",
						background: "var(--card)",
						fontSize: 12,
						color: "var(--muted)",
						marginLeft: "auto",
					}}
				>
					<span>filtered sum</span>
					<strong
						style={{
							color: filteredTotal >= 0 ? "var(--green)" : "var(--rose)",
							fontWeight: 700,
							fontVariantNumeric: "tabular-nums",
						}}
					>
						{formatMoneyNumber(filteredTotal)}
					</strong>
				</div>
			)}
			<button
				className="mono"
				style={{ ...chip(false), marginLeft: hasActiveFilters ? 0 : "auto" }}
				onClick={onExportCsv}
				disabled={count === 0}
			>
				export CSV
			</button>
		</div>
	);
};
