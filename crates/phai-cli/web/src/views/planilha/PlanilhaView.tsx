import { queryDb } from "@livestore/livestore";
import { useQuery, useStore, useClientDocument } from "@livestore/react";
import React, {
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
import type { ForecastView } from "../types";
import { SheetScenarioBar } from "./SheetScenarioBar";

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

const STATUS_LABEL: Record<string, string> = {
	ativo: "Previsto",
	active: "Previsto",
	realizado: "Efetivado",
	descartado: "Excluido",
};

const metaString = (meta: Record<string, unknown>, key: string): string | null => {
	const value = meta[key];
	return typeof value === "string" && value.trim() ? value : null;
};

const monthOf = (date: string | null): string | null =>
	date && date.length >= 7 ? date.slice(0, 7) : null;

/** All active forecasts for the sheet — includes envelopes, installments,
 *  templates, and manual planned transactions. */
const isSheetForecast = (forecast: ForecastView): boolean =>
	!["descartado", "inativo"].includes(forecast.status);

const FORECAST_KIND_LABEL: Record<string, string> = {
	manual: "manual",
	installment: "installment",
	template: "template",
};

const forecastKindLabel = (forecast: ForecastView): string => {
	if (
		forecast.kind === "manual" &&
		forecast.metadataJson.ui_role !== "planned_transaction"
	) {
		return "envelope";
	}
	return FORECAST_KIND_LABEL[forecast.kind] ?? forecast.kind;
};

const predictedAmount = (forecast: ForecastView): string =>
	metaString(forecast.metadataJson, "predicted_amount") ?? forecast.amount;

const realizedAmount = (forecast: ForecastView): string | null =>
	metaString(forecast.metadataJson, "realized_amount") ??
	(forecast.status === "realizado" ? forecast.amount : null);

const scoreCandidate = (forecast: ForecastView, tx: TxView): number => {
	const amountGap = Math.abs(
		Math.abs(toCents(tx.amount)) - Math.abs(toCents(predictedAmount(forecast))),
	);
	const due = Date.parse(forecast.dueDate ?? `${forecast.month ?? tx.month}-01`);
	const posted = Date.parse(tx.postedAt);
	const dateGap =
		Number.isFinite(due) && Number.isFinite(posted) ? Math.abs(due - posted) : 0;
	return amountGap + dateGap / 86400000;
};

type SheetDataRow =
	| {
			kind: "transaction";
			id: string;
			date: string;
			description: string;
			account: string;
			category: string | null;
			amount: string;
			tx: TxView;
			tier: CommitmentTier;
	  }
	| {
			kind: "forecast";
			id: string;
			date: string;
			description: string;
			account: string;
			category: string | null;
			amount: string;
			forecast: ForecastView;
			candidates: ReadonlyArray<TxView>;
	  };

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
export const PlanilhaView = ({
	month,
	activeScenarioId = null,
	scenarioDelta = null,
	onActivateScenario,
	onScenarioMutated,
}: {
	month: string;
	/** Active planning scenario (ADR-0037); null = baseline. */
	activeScenarioId?: string | null;
	/** Selected-month projected-saldo delta vs. baseline (null = not seeded). */
	scenarioDelta?: number | null;
	onActivateScenario?: (scenarioId: string | null) => void;
	/** Fired after any scenario write so the caller re-seeds the projection. */
	onScenarioMutated?: () => void;
}) => {
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
	const [composerOpen, setComposerOpen] = useState<"expense" | "income" | null>(
		null,
	);
	const [forecastDescription, setForecastDescription] = useState("");
	const [forecastAmount, setForecastAmount] = useState("");
	const [forecastAccountId, setForecastAccountId] = useState("");
	const [forecastCategoryId, setForecastCategoryId] = useState("");
	const [settlingId, setSettlingId] = useState<string | null>(null);
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
			uncategorizedOnly: ui.uncategorizedOnly,
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
			ui.uncategorizedOnly,
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
		);
		return sortForSheet(filtered, sort, overlayMap, accountMap);
	}, [txRows, month, filters, overlayMap, accountMap, fixedCategories, sort]);

	const sheetRows = useMemo(() => {
		const txSheetRows: SheetDataRow[] = rows.map((tx) => ({
			kind: "transaction",
			id: tx.id,
			date: tx.postedAt,
			description: sheetLabel(tx),
			account: accountMap.get(tx.accountId)?.label ?? tx.accountId,
			category: effectiveCategory(tx, overlayMap),
			amount: tx.amount,
			tx,
			tier: commitmentTier(tx, fixedCategories, overlayMap),
		}));
		const forecastRows = (forecasts as ReadonlyArray<Omit<ForecastView, "month">>)
			.map((forecast) => ({
				...forecast,
				month: monthOf(forecast.dueDate),
				metadataJson:
					forecast.metadataJson && typeof forecast.metadataJson === "object"
						? (forecast.metadataJson as Record<string, unknown>)
						: {},
			}))
			.filter((forecast) => forecast.month === month && isSheetForecast(forecast))
			.filter((forecast) => {
				if (
					filters.accountFilter &&
					forecast.accountId !== filters.accountFilter
				) {
					return false;
				}
				if (
					filters.categoryFilter &&
					forecast.categoryId !== filters.categoryFilter
				) {
					return false;
				}
				if (ui.textFilter) {
					const haystack = [
						forecast.description,
						forecast.categoryId ?? "",
						accountMap.get(forecast.accountId ?? "") ?? "",
					]
						.join(" ")
						.toLowerCase();
					if (!haystack.includes(ui.textFilter.toLowerCase())) return false;
				}
				return true;
			})
			.map((forecast): SheetDataRow => ({
				kind: "forecast",
				id: forecast.forecastId,
					date: forecast.dueDate ?? `${month}-01`,
					description: forecast.description,
					account: forecast.accountId
						? (accountMap.get(forecast.accountId)?.label ?? forecast.accountId)
						: "sem conta",
				category: forecast.categoryId,
				amount: forecast.amount,
				forecast,
				candidates: rows
					.filter(
						(tx) =>
							isNegative(tx.amount) === isNegative(forecast.amount) &&
							(!forecast.accountId || tx.accountId === forecast.accountId),
					)
					.sort(
						(left, right) =>
							scoreCandidate(forecast, left) - scoreCandidate(forecast, right),
					)
					.slice(0, 6),
			}));
		const allRows = [...txSheetRows, ...forecastRows];
		return allRows.sort((left, right) => {
			const dir = sort.dir;
			switch (sort.key) {
				case "amount":
					return (toCents(left.amount) - toCents(right.amount)) * dir;
				case "account":
					return left.account.localeCompare(right.account) * dir;
				case "category":
					return (left.category ?? "").localeCompare(right.category ?? "") * dir;
				case "description":
					return left.description.localeCompare(right.description) * dir;
				case "date":
				default:
					return left.date.localeCompare(right.date) * dir;
			}
		});
	}, [
		rows,
		forecasts,
		month,
		filters.accountFilter,
		filters.categoryFilter,
		ui.textFilter,
		accountMap,
		overlayMap,
		fixedCategories,
		sort,
	]);

	const hasSheetFilters = useMemo(
		() => hasActiveFilters(filters),
		[filters],
	);

	// Reset selection when the month or the visible set changes size.
	const rowCount = sheetRows.length;
	useEffect(() => {
		setSelectedIds(new Set());
		setFocusedIdx(-1);
		lastClickedIdx.current = -1;
	}, [month, rowCount]);

	const totals = useMemo(() => {
		let inCents = 0;
		let outCents = 0;
		for (const row of sheetRows) {
			const c = toCents(row.amount);
			if (c < 0) outCents += -c;
			else inCents += c;
		}
		return {
			entradas: inCents / 100,
			saidas: outCents / 100,
			net: sheetRows.reduce((total, row) => total + toCents(row.amount), 0) / 100,
		};
	}, [sheetRows]);

	const selectionTotal = useMemo(() => {
		let cents = 0;
		for (const row of sheetRows) {
			if (row.kind === "transaction" && selectedIds.has(row.id)) {
				cents += toCents(row.amount);
			}
		}
		return cents / 100;
	}, [sheetRows, selectedIds]);

	const handleExportCsv = useCallback(() => {
		downloadCsv(`phai-planilha-${month}.csv`, sheetRowsCsv(rows, accountMap));
	}, [month, rows, accountMap]);

	const submitForecast = useCallback(() => {
		const desc = forecastDescription.trim();
		const mag = forecastAmount.replace(/^-/, "").trim();
		if (!desc || !mag || !composerOpen) return;
		store.commit(
			events.forecastCreated({
				writeId: crypto.randomUUID(),
				description: desc,
				amount: composerOpen === "expense" ? `-${mag}` : mag,
				dueDate: `${month}-01`,
				categoryId: forecastCategoryId || null,
				accountId: forecastAccountId || null,
				uiRole: "planned_transaction",
				createdAt: Date.now(),
			}),
		);
		setForecastDescription("");
		setForecastAmount("");
		setForecastAccountId("");
		setForecastCategoryId("");
		setComposerOpen(null);
	}, [
		store,
		forecastDescription,
		forecastAmount,
		composerOpen,
		month,
		forecastCategoryId,
		forecastAccountId,
	]);

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
						const next = Math.min(
							Math.max(i + delta, 0),
							sheetRows.length - 1,
						);
						tableRef.current
							?.querySelector(`[data-row-idx="${next}"]`)
							?.scrollIntoView({ block: "nearest" });
						return next;
					});
					break;
				}
				case " ": {
					e.preventDefault();
					const row = sheetRows[focusedIdx];
					if (!row || row.kind !== "transaction") break;
					setSelectedIds((prev) => {
						const next = new Set(prev);
						if (next.has(row.id)) next.delete(row.id);
						else next.add(row.id);
						return next;
					});
					break;
				}
				case "Enter": {
					// Mirrors the plain click: open the full edit modal.
					const row = sheetRows[focusedIdx];
					if (!row || row.kind !== "transaction") break;
					e.preventDefault();
					setModalTx(row.tx);
					break;
				}
				case "k": {
					const row = sheetRows[focusedIdx];
					if (!row || row.kind !== "transaction") break;
					e.preventDefault();
					const el = tableRef.current?.querySelector(
						`[data-row-idx="${focusedIdx}"] [data-cat-chip]`,
					);
					if (el) openPickerFor(row.tx, el as HTMLElement);
					break;
				}
				case "Escape":
					setSelectedIds(new Set());
					break;
			}
		},
		[picker, modalTx, sheetRows, focusedIdx, openPickerFor],
	);

	const toggleSort = (key: SheetSortKey) =>
		setSort((s) =>
			s.key === key ? { key, dir: s.dir === 1 ? -1 : 1 } : { key, dir: key === "date" || key === "amount" ? -1 : 1 },
		);

	const selectableRowIds = useMemo(
		() =>
			sheetRows
				.filter((row) => row.kind === "transaction")
				.map((row) => row.id),
		[sheetRows],
	);
	const allVisibleSelected =
		selectableRowIds.length > 0 &&
		selectableRowIds.every((id) => selectedIds.has(id));

	return (
		<section aria-label={`Sheet for ${month}`}>
			{/* ── Scenario pills (ADR-0037) — the sheet header owns them ── */}
			{onActivateScenario && onScenarioMutated && (
				<SheetScenarioBar
					activeScenarioId={activeScenarioId}
					scenarioDelta={scenarioDelta}
					onActivate={onActivateScenario}
					onMutated={onScenarioMutated}
				/>
			)}

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
				<div
					style={{
						display: "flex",
						flexWrap: "wrap",
						gap: 8,
						padding: 12,
						borderBottom: "1px solid var(--border)",
						background: "rgba(148,163,184,0.04)",
					}}
				>
					<button
						onClick={() =>
							setComposerOpen((value) =>
								value === "expense" ? null : "expense",
							)
						}
						className="mono pressable"
						style={inlineActionBtn("var(--rose)")}
					>
						+ despesa no mes
					</button>
					<button
						onClick={() =>
							setComposerOpen((value) =>
								value === "income" ? null : "income",
							)
						}
						className="mono pressable"
						style={inlineActionBtn("var(--green)")}
					>
						+ receita no mes
					</button>
					<span className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
						previsoes manuais aparecem na mesma grade e podem ser efetivadas ou excluidas
					</span>
				</div>
				{composerOpen ? (
					<div
						style={{
							display: "grid",
							gridTemplateColumns: "minmax(220px,2fr) repeat(4,minmax(140px,1fr)) auto",
							gap: 8,
							padding: 12,
							borderBottom: "1px solid var(--border)",
							background:
								composerOpen === "expense"
									? "rgba(244,63,94,0.05)"
									: "rgba(22,163,74,0.05)",
						}}
					>
						<input
							value={forecastDescription}
							onChange={(e) => setForecastDescription(e.target.value)}
							placeholder="descricao"
							style={sheetInputStyle}
						/>
						<input
							value={forecastAmount}
							onChange={(e) => setForecastAmount(e.target.value)}
							placeholder={composerOpen === "expense" ? "120.00" : "2500.00"}
							style={sheetInputStyle}
						/>
						<select
							value={forecastAccountId}
							onChange={(e) => setForecastAccountId(e.target.value)}
							style={sheetInputStyle}
						>
							<option value="">conta opcional</option>
							{accounts.map((account) => (
								<option key={account.id} value={account.id}>
									{account.label}
								</option>
							))}
						</select>
						<input
							value={forecastCategoryId}
							onChange={(e) => setForecastCategoryId(e.target.value)}
							list="sheet-forecast-categories"
							placeholder="categoria opcional"
							style={sheetInputStyle}
						/>
						<div className="mono" style={{ ...sheetInputStyle, opacity: 0.7 }}>
							{month}
						</div>
						<button onClick={submitForecast} className="mono" style={saveBtnStyle}>
							salvar previsao
						</button>
					</div>
				) : null}
				<datalist id="sheet-forecast-categories">
					{categories.map((category) => (
						<option key={category.id} value={category.id} />
					))}
				</datalist>
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
												: new Set(selectableRowIds),
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
					<tbody className="sheet-tbody">
						{sheetRows.map((row, idx) => (
							<SheetRow
								key={`${row.kind}:${row.id}`}
								row={row}
								idx={idx}
								selected={
									row.kind === "transaction" && selectedIds.has(row.id)
								}
								focused={focusedIdx === idx}
								onClick={handleRowClick}
								onCategoryClick={openPickerFor}
								onSettle={(forecast, transactionId) =>
									store.commit(
										events.forecastSettled({
											writeId: crypto.randomUUID(),
											forecastId: forecast.forecastId,
											transactionId,
											predictedAmount: predictedAmount(forecast),
											actualAmount:
												rows.find((tx) => tx.id === transactionId)?.amount ??
												forecast.amount,
											actualDate:
												rows.find((tx) => tx.id === transactionId)?.postedAt ??
												forecast.dueDate ??
												`${month}-01`,
											actualDescription:
												rows.find((tx) => tx.id === transactionId)?.rawDescription ??
												forecast.description,
											settledAt: new Date().toISOString(),
											settledAtMs: Date.now(),
										}),
									)
								}
								onDelete={(forecast) =>
									store.commit(
										events.forecastDeleted({
											writeId: crypto.randomUUID(),
											forecastId: forecast.forecastId,
											deletedAt: Date.now(),
										}),
									)
								}
								settlingId={settlingId}
								onToggleSettling={setSettlingId}
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
						className="mono pressable"
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

const SheetRow = React.memo(({
	row,
	idx,
	selected,
	focused,
	onClick,
	onCategoryClick,
	onSettle,
	onDelete,
	settlingId,
	onToggleSettling,
}: {
	row: SheetDataRow;
	idx: number;
	selected: boolean;
	focused: boolean;
	onClick: (tx: TxView, idx: number, e: React.MouseEvent) => void;
	onCategoryClick: (tx: TxView, anchor: HTMLElement) => void;
	onSettle: (forecast: ForecastView, transactionId: string) => void;
	onDelete: (forecast: ForecastView) => void;
	settlingId: string | null;
	onToggleSettling: (id: string | null) => void;
}) => {
	if (row.kind === "forecast") {
		const forecast = row.forecast;
		const negative = isNegative(forecast.amount);
		const linkedAmount = realizedAmount(forecast);
		const kindLbl = forecastKindLabel(forecast);
		return (
			<tr
				data-row-idx={idx}
				style={{
					background:
						forecast.status === "realizado"
							? "rgba(22,163,74,0.05)"
							: "rgba(109,74,255,0.04)",
					boxShadow: focused ? "inset 2px 0 0 var(--purple)" : undefined,
				}}
			>
				<td style={tdStyle} />
				<td className="mono" style={{ ...tdStyle, color: "var(--purple)" }}>
					{row.date.slice(8, 10)}/{row.date.slice(5, 7)}
				</td>
				<td style={{ ...tdStyle, maxWidth: 380, color: "var(--purple)" }}>
					<div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
						<span>{forecast.description}</span>
						<span style={forecastPillStyle(forecast.status === "realizado" ? "done" : "pending")}>
							{STATUS_LABEL[forecast.status] ?? forecast.status}
						</span>
						<span style={forecastPillStyle("pending")}>{kindLbl}</span>
					</div>
					<div className="mono" style={{ fontSize: 11, color: "var(--muted)", marginTop: 4 }}>
						{forecast.status === "realizado"
							? `forecast ${kindLbl} efetivado`
							: `forecast ${kindLbl} pendente`}
					</div>
						{linkedAmount && linkedAmount !== predictedAmount(forecast) ? (
							<div className="mono" style={{ fontSize: 11, color: "var(--muted)", marginTop: 4 }}>
								prev. {formatMoneyNumber(Math.abs(toCents(predictedAmount(forecast))) / 100)} {"->"} real{" "}
								{formatMoneyNumber(Math.abs(toCents(linkedAmount)) / 100)}
							</div>
						) : null}
					<div style={{ display: "flex", gap: 8, flexWrap: "wrap", marginTop: 8 }}>
						{forecast.status !== "realizado" ? (
							<button
								onClick={() =>
									onToggleSettling(
										settlingId === forecast.forecastId ? null : forecast.forecastId,
									)
								}
								className="mono"
								style={forecastRowBtnStyle}
							>
								marcar como pago
							</button>
						) : null}
						<button
							onClick={() => onDelete(forecast)}
							className="mono"
							style={forecastRowBtnStyle}
						>
							excluir
						</button>
					</div>
					{settlingId === forecast.forecastId && row.candidates.length > 0 ? (
						<div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginTop: 8 }}>
							{row.candidates.map((candidate) => (
								<button
									key={candidate.id}
									onClick={() => onSettle(forecast, candidate.id)}
									className="mono"
									style={forecastCandidateBtnStyle}
								>
									{candidate.postedAt.slice(8, 10)}/{candidate.postedAt.slice(5, 7)} · {sheetLabel(candidate)} · {sheetAmountLabel(candidate.amount)}
								</button>
							))}
						</div>
					) : null}
				</td>
				<td className="mono" style={{ ...tdStyle, fontSize: 12, color: "var(--muted)" }}>
					{row.account}
				</td>
				<td style={tdStyle}>
					<span
						className="mono"
						style={{
							background: row.category ? "var(--chip, #f1eefc)" : "transparent",
							color: row.category ? "var(--purple)" : "var(--amber)",
							border: row.category
								? "1px solid transparent"
								: "1px dashed var(--amber)",
							borderRadius: "var(--radius-full)",
							padding: "3px 10px",
							fontSize: 12,
						}}
					>
						{row.category
							? `${categoryEmoji(row.category, !negative)} ${row.category}`
							: "❓ uncategorized"}
					</span>
				</td>
				<td
					className="mono"
					style={{
						...tdStyle,
						textAlign: "right",
						color: forecast.status === "realizado"
							? (negative ? "var(--rose)" : "var(--green)")
							: "var(--purple)",
						fontVariantNumeric: "tabular-nums",
						whiteSpace: "nowrap",
					}}
				>
					{sheetAmountLabel(forecast.amount)}
				</td>
				<td className="mono" style={{ ...tdStyle, fontSize: 11, color: "var(--purple)", opacity: 0.7 }}>
					{forecast.status === "realizado" ? "efetivado" : kindLbl}
				</td>
			</tr>
		);
	}
	const tx = row.tx;
	const negative = isNegative(tx.amount);
	return (
		<tr
			data-row-idx={idx}
			aria-selected={selected}
			className={!selected ? "sheet-row-hover" : undefined}
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
					{negative && <TierBadge tier={row.tier} compact />}
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
				{row.account}
			</td>
			<td style={tdStyle} onClick={(e) => e.stopPropagation()}>
				<button
					data-cat-chip
					onClick={(e) => onCategoryClick(tx, e.currentTarget)}
					className="mono"
					title="change category (Enter)"
					style={{
						background: row.category ? "var(--chip, #f1eefc)" : "transparent",
						color: row.category ? "var(--purple)" : "var(--amber)",
						border: row.category
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
					{row.category
						? `${categoryEmoji(row.category, !negative)} ${row.category}`
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
});

const tdStyle: React.CSSProperties = {
	padding: "6px 10px",
	verticalAlign: "top",
	// Row separator lives on the td: tr borders don't render with
	// border-collapse: separate (which the sticky header requires).
	borderBottom: "1px solid var(--border)",
};

const inlineActionBtn = (color: string): React.CSSProperties => ({
	// color is a CSS var (e.g. var(--rose)) — concatenating a hex alpha
	// (`${color}12`) yields invalid CSS, so use color-mix to tint it.
	background: `color-mix(in srgb, ${color} 10%, transparent)`,
	color,
	border: `1px solid color-mix(in srgb, ${color} 38%, transparent)`,
	borderRadius: "var(--radius-full)",
	padding: "6px 12px",
	cursor: "pointer",
	fontSize: 12,
	fontWeight: 600,
	whiteSpace: "nowrap",
});

const sheetInputStyle: React.CSSProperties = {
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "8px 10px",
	fontSize: 12,
	background: "var(--card)",
	minHeight: 36,
};

const saveBtnStyle: React.CSSProperties = {
	background: "var(--purple)",
	color: "#fff",
	border: "none",
	borderRadius: "var(--radius-sm)",
	padding: "8px 12px",
	cursor: "pointer",
	fontSize: 12,
};

const forecastPillStyle = (
	tone: "pending" | "done",
): React.CSSProperties => ({
	borderRadius: "var(--radius-full)",
	padding: "2px 8px",
	fontSize: 10,
	fontFamily: "var(--font-mono, monospace)",
	textTransform: "uppercase",
	letterSpacing: "0.06em",
	background:
		tone === "done" ? "rgba(22,163,74,0.12)" : "rgba(124,93,250,0.1)",
	color: tone === "done" ? "var(--green)" : "var(--purple)",
});

const forecastRowBtnStyle: React.CSSProperties = {
	background: "transparent",
	color: "var(--muted)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "4px 10px",
	cursor: "pointer",
	fontSize: 11,
};

const forecastCandidateBtnStyle: React.CSSProperties = {
	background: "rgba(148,163,184,0.08)",
	color: "var(--text)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "4px 10px",
	cursor: "pointer",
	fontSize: 11,
	maxWidth: "100%",
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
				className="mono pressable"
				style={chip(ui.uncategorizedOnly)}
				onClick={() => setUi({ uncategorizedOnly: !ui.uncategorizedOnly })}
			>
				uncategorized
			</button>
			<button
				className="mono pressable"
				style={chip(ui.unreviewedOnly)}
				onClick={() => setUi({ unreviewedOnly: !ui.unreviewedOnly })}
			>
				unreviewed
			</button>
			<button
				className="mono pressable"
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
					className="mono pressable"
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
