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
import { monthTheme } from "../../lib/monthTheme";
import {
	applyScenarioToMonthRows,
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
	matchesAccountFilter,
	matchesSheetLocalFilters,
	readSheetLocalFilters,
	readSheetSort,
	routeSheetAdd,
	routeSheetAmountEdit,
	routeSheetDelete,
	SHEET_ORIGIN_ICONS,
	sheetLabel,
	sortUnifiedRows,
	transactionsForMonth,
	writeSheetLocalFilters,
	writeSheetSort,
	type CommitmentTier,
	type PlannedSheetRow,
	type ScenarioChangeLike,
	type SheetDeleteScope,
	type SheetForecastLike,
	type SheetLocalFilters,
	type SheetOrigin,
	type SheetRowRef,
	type SheetSort,
	type SheetSortKey,
	type TxView,
} from "../../lib/derivations";
import type { ForecastView } from "../types";
import { InsertHandle, InsertRowEditor, type InsertDraft } from "./InsertRow";
import {
	categoryChipStyle,
	rowActionBtnStyle,
	tdStyle,
	thStyle,
} from "./sheetShared";
import { SheetScenarioBar } from "./SheetScenarioBar";
import { SheetFilterBar } from "./SheetFilterBar";

// Re-exported so the CSV unit test keeps importing them from this module.
export {
	csvAmountCell,
	sheetAmountLabel,
	sheetRowsCsv,
	sheetSignedTotal,
} from "./sheetShared";
import {
	downloadCsv,
	sheetAmountLabel,
	sheetRowsCsv,
} from "./sheetShared";

const txAll$ = queryDb(tables.transactions.orderBy("postedAt", "desc"));
const overlay$ = queryDb(tables.reviewOverlay);
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));
const accounts$ = queryDb(tables.accounts.orderBy("label", "asc"));
const forecasts$ = queryDb(tables.forecasts);
const scenarioChanges$ = queryDb(tables.scenarioChanges);

/** Sortable columns of the unified sheet (origin lives in the icon header). */
const COLUMNS: Array<{ key: SheetSortKey; label: string; width?: string; align?: "right" }> =
	[
		{ key: "description", label: "descrição" },
		{ key: "category", label: "categoria", width: "200px" },
		{ key: "date", label: "dia", width: "56px" },
		{ key: "amount", label: "valor", width: "150px", align: "right" },
	];

const ORIGIN_TITLE: Record<SheetOrigin, string> = {
	real: "realizado",
	installment: "parcela",
	recurring: "recorrente",
	fixed: "conta fixa",
	manual: "previsto manual",
	scenario: "item do cenário",
};

/** "YYYY-MM" of the current calendar month. */
const currentMonthKey = (): string => {
	const d = new Date();
	return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

const MONTH_LABEL_FMT = new Intl.DateTimeFormat("pt-BR", {
	month: "long",
	year: "numeric",
});

/** "2026-07" → "Julho 2026" (capitalised) for the sheet's month heading. */
const monthLabel = (month: string): string => {
	const [y, m] = month.split("-").map(Number);
	if (!y || !m) return month;
	const s = MONTH_LABEL_FMT.format(new Date(y, m - 1, 1));
	return s.charAt(0).toUpperCase() + s.slice(1);
};

/** Number of days in a "YYYY-MM" month — used to validate the day column. */
const daysInMonth = (month: string): number => {
	const [y, m] = month.split("-").map(Number);
	if (!y || !m) return 31;
	return new Date(y, m, 0).getDate();
};

/** Normalize a typed magnitude (BR or dot decimal) to a plain dot-decimal string. */
const normalizeMagnitude = (raw: string): string => {
	let s = raw.trim().replace(/\s/g, "").replace(/^[+-]/, "");
	if (s.includes(",")) s = s.replace(/\./g, "").replace(",", ".");
	return s;
};

/** The magnitude (no sign) of a signed decimal amount, as an editable string. */
const magnitudeOf = (amount: string): string => {
	const cents = Math.abs(toCents(amount));
	return (cents / 100).toFixed(2);
};

const monthOf = (date: string | null): string | null =>
	date && date.length >= 7 ? date.slice(0, 7) : null;

/** Build a full scenario-change table row from the parts a gesture supplies. */
const scenarioChangeRow = (
	scenarioId: string,
	kind: string,
	parts: Partial<{
		changeId: string;
		targetForecastId: string | null;
		targetTemplateId: string | null;
		month: string | null;
		effectiveFrom: string | null;
		amount: string | null;
		monthsCount: number | null;
		description: string | null;
		categoryId: string | null;
		accountId: string | null;
	}>,
) => ({
	changeId: parts.changeId ?? `chg-${crypto.randomUUID()}`,
	scenarioId,
	kind,
	targetForecastId: parts.targetForecastId ?? null,
	targetTemplateId: parts.targetTemplateId ?? null,
	month: parts.month ?? null,
	effectiveFrom: parts.effectiveFrom ?? null,
	amount: parts.amount ?? null,
	monthsCount: parts.monthsCount ?? null,
	description: parts.description ?? null,
	categoryId: parts.categoryId ?? null,
	accountId: parts.accountId ?? null,
	status: "ativo",
	orphaned: 0,
});

interface BaseUnifiedRow {
	id: string;
	date: string;
	description: string;
	account: string;
	category: string | null;
	amount: string;
	origin: SheetOrigin;
}

type UnifiedRow =
	| (BaseUnifiedRow & { kind: "tx"; origin: "real"; tx: TxView; tier: CommitmentTier })
	| (BaseUnifiedRow & { kind: "planned"; planned: PlannedSheetRow });

const rowRef = (planned: PlannedSheetRow): SheetRowRef => ({
	origin: planned.origin,
	forecastId: planned.forecastId,
	templateId: planned.templateId,
	changeId: planned.changeId,
});

/**
 * Order the sheet rows. With no frozen order this is the plain column sort;
 * while a manual order is frozen (after an inline insert) rows keep the snapshot
 * order, and any row not in the snapshot — the freshly created one — is dropped
 * in at `insertAt`, so a new line stays exactly where it was placed until the
 * user re-sorts by clicking a column.
 */
const orderUnifiedRows = (
	all: UnifiedRow[],
	frozen: { ids: string[]; insertAt: number } | null,
	sort: SheetSort,
): UnifiedRow[] => {
	if (!frozen) return sortUnifiedRows(all, sort);
	const pos = new Map(frozen.ids.map((id, i) => [id, i]));
	const known: UnifiedRow[] = [];
	const fresh: UnifiedRow[] = [];
	for (const r of all) (pos.has(r.id) ? known : fresh).push(r);
	known.sort((a, b) => (pos.get(a.id) ?? 0) - (pos.get(b.id) ?? 0));
	const at = Math.min(Math.max(frozen.insertAt, 0), known.length);
	return [...known.slice(0, at), ...fresh, ...known.slice(at)];
};

/**
 * Planilha — the unified sheet of a month (ADR-0038). Real transactions and
 * planned items (forecasts + the active scenario's deltas) share one table:
 * each row carries an origin glyph, an inline-editable amount, and per-row
 * delete/insert affordances on hover. Baseline edits write real forecasts;
 * with a scenario active the same gestures become plan deltas (routing in
 * `routeSheet*`). Sort and origin/flow filters persist in localStorage so a
 * view tweak never bumps STORE_VERSION.
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
	const forecasts = useQuery(forecasts$) as ReadonlyArray<ForecastView>;
	const scenarioChangeRows = useQuery(scenarioChanges$) as ReadonlyArray<
		ScenarioChangeLike & { scenarioId: string; status: string }
	>;

	const overlayMap = useMemo(() => buildOverlayMap(overlay), [overlay]);
	const accountMap = useMemo(() => buildAccountMap(accounts), [accounts]);
	const categoryIds = useMemo(() => categories.map((c) => c.id), [categories]);
	const fixedCategories = useMemo(
		() => fixedCategoriesFromForecasts(forecasts),
		[forecasts],
	);

	const monthEditable = month >= currentMonthKey();
	// Discreet per-month identity (ADR-free view sugar): a stable accent + season
	// glyph so each month's sheet is distinguishable at a glance (never loud).
	const theme = useMemo(() => monthTheme(month), [month]);

	// ── View preferences (localStorage, not LiveStore) ──────────────────────
	const [sort, setSort] = useState<SheetSort>(
		() => readSheetSort(window.localStorage) ?? { key: "date", dir: -1 },
	);
	const [localFilters, setLocalFilters] = useState<SheetLocalFilters>(
		() => readSheetLocalFilters(window.localStorage),
	);
	useEffect(() => writeSheetSort(window.localStorage, sort), [sort]);
	useEffect(
		() => writeSheetLocalFilters(window.localStorage, localFilters),
		[localFilters],
	);

	// ── Interaction state ───────────────────────────────────────────────────
	const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
	const [focusedIdx, setFocusedIdx] = useState(-1);
	const lastClickedIdx = useRef(-1);
	const [picker, setPicker] = useState<{
		anchorRect: DOMRect;
		targetIds: string[];
	} | null>(null);
	const [recentCats, setRecentCats] = useState<string[]>([]);
	const [modalTx, setModalTx] = useState<TxView | null>(null);
	const [editingId, setEditingId] = useState<string | null>(null);
	const [editingValue, setEditingValue] = useState("");
	const [insertAfter, setInsertAfter] = useState<string | "end" | null>(null);
	// Frozen manual order: after an inline insert the sheet keeps the row where
	// it was dropped (instead of re-sorting by day) until the user clicks a
	// column header. `ids` is the displayed order snapshot; `insertAt` is where
	// newly-created rows (unknown ids) land within it.
	const [frozen, setFrozen] = useState<{ ids: string[]; insertAt: number } | null>(
		null,
	);
	const [deleteRowId, setDeleteRowId] = useState<string | null>(null);
	const tableRef = useRef<HTMLDivElement>(null);

	// ── Undo (skip / delete) ─────────────────────────────────────────────────
	// Removing or skipping a planned row is a mistake away from lost work, so we
	// stash a reversing action and surface a transient "desfazer" toast. `run` is
	// null when the gesture has no clean client-side inverse (a template-ended
	// recurrence, a discarded materialized forecast) — then the toast only
	// confirms, without promising an undo we can't honour.
	const [undo, setUndo] = useState<{ label: string; run: (() => void) | null } | null>(
		null,
	);
	const undoTimer = useRef<number | null>(null);
	const flashUndo = useCallback(
		(label: string, run: (() => void) | null) => {
			if (undoTimer.current) window.clearTimeout(undoTimer.current);
			setUndo({ label, run });
			undoTimer.current = window.setTimeout(() => setUndo(null), 7000);
		},
		[],
	);
	useEffect(
		() => () => {
			if (undoTimer.current) window.clearTimeout(undoTimer.current);
		},
		[],
	);

	const uiFilters = useMemo(
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

	// Active scenario's changes, as the derivation and the re-emit paths need.
	const activeChanges = useMemo<ScenarioChangeLike[]>(
		() =>
			activeScenarioId
				? scenarioChangeRows.filter(
						(c) => c.scenarioId === activeScenarioId && c.status !== "aplicado",
					)
				: [],
		[scenarioChangeRows, activeScenarioId],
	);

	const forecastsLike = useMemo<SheetForecastLike[]>(
		() =>
			forecasts.map((f) => ({
				forecastId: f.forecastId,
				dueDate: f.dueDate,
				description: f.description,
				amount: f.amount,
				categoryId: f.categoryId,
				accountId: f.accountId,
				status: f.status,
				kind: f.kind,
				templateId: f.templateId,
			})),
		[forecasts],
	);

	// Real transactions of the month, with the optimistic overlay baked in and
	// the ui-doc filters applied (account/text/tier/uncategorized/…).
	const txUnified = useMemo<UnifiedRow[]>(() => {
		const monthTxs = transactionsForMonth(txRows, month).map((tx) =>
			effectiveTx(tx, overlayMap),
		);
		const filtered = filterTransactions(
			monthTxs,
			uiFilters,
			overlayMap,
			accountMap,
			fixedCategories,
		);
		return filtered.map((tx) => ({
			kind: "tx" as const,
			origin: "real" as const,
			id: tx.id,
			date: tx.postedAt,
			description: sheetLabel(tx),
			account: accountMap.get(tx.accountId)?.label ?? tx.accountId,
			category: effectiveCategory(tx, overlayMap),
			amount: tx.amount,
			tx,
			tier: commitmentTier(tx, fixedCategories, overlayMap),
		}));
	}, [txRows, month, uiFilters, overlayMap, accountMap, fixedCategories]);

	// Planned rows: baseline forecasts + the active scenario applied on top.
	const plannedUnified = useMemo<UnifiedRow[]>(() => {
		const text = uiFilters.textFilter?.toLowerCase() ?? null;
		return applyScenarioToMonthRows(forecastsLike, activeChanges, month)
			.filter((p) => {
				if (!matchesAccountFilter(uiFilters.accountFilter, p.accountId ?? "")) {
					return false;
				}
				if (
					uiFilters.categoryFilter &&
					p.categoryId !== uiFilters.categoryFilter
				) {
					return false;
				}
				if (text) {
					const haystack = [p.description, p.categoryId ?? ""]
						.join(" ")
						.toLowerCase();
					if (!haystack.includes(text)) return false;
				}
				return true;
			})
			.map((p) => ({
				kind: "planned" as const,
				id: `p:${p.id}`,
				date: p.dueDate,
				description: p.description,
				account: p.accountId
					? (accountMap.get(p.accountId)?.label ?? p.accountId)
					: "sem conta",
				category: p.categoryId,
				amount: p.amount,
				origin: p.origin,
				planned: p,
			}));
	}, [forecastsLike, activeChanges, month, uiFilters, accountMap]);

	const rows = useMemo<UnifiedRow[]>(() => {
		const all = [...txUnified, ...plannedUnified].filter((r) =>
			matchesSheetLocalFilters(r, localFilters),
		);
		return orderUnifiedRows(all, frozen, sort);
	}, [txUnified, plannedUnified, localFilters, sort, frozen]);

	const hasSheetFilters = useMemo(
		() => hasActiveFilters(uiFilters) || localFilters.origin !== "all" || localFilters.flow !== "all",
		[uiFilters, localFilters],
	);

	const rowCount = rows.length;
	useEffect(() => {
		setSelectedIds(new Set());
		setFocusedIdx(-1);
		lastClickedIdx.current = -1;
		setEditingId(null);
		setInsertAfter(null);
		setDeleteRowId(null);
		setUndo(null);
		setFrozen(null);
	}, [month, activeScenarioId]);

	// ── Totals footer (the sheet's status bar) ──────────────────────────────
	const totals = useMemo(() => {
		let realizedCents = 0;
		let plannedCents = 0;
		for (const row of rows) {
			if (row.kind === "tx") realizedCents += toCents(row.amount);
			else if (!row.planned.skipped) plannedCents += toCents(row.amount);
		}
		return {
			realizado: realizedCents / 100,
			previsto: plannedCents / 100,
			net: (realizedCents + plannedCents) / 100,
		};
	}, [rows]);

	const selectionTotal = useMemo(() => {
		let cents = 0;
		for (const row of rows) {
			if (row.kind === "tx" && selectedIds.has(row.id)) {
				cents += toCents(row.amount);
			}
		}
		return cents / 100;
	}, [rows, selectedIds]);

	const rawTxForCsv = useMemo(
		() => txUnified.map((r) => (r as Extract<UnifiedRow, { kind: "tx" }>).tx),
		[txUnified],
	);
	const handleExportCsv = useCallback(() => {
		downloadCsv(`phai-planilha-${month}.csv`, sheetRowsCsv(rawTxForCsv, accountMap));
	}, [month, rawTxForCsv, accountMap]);

	const notifyScenario = useCallback(() => {
		onScenarioMutated?.();
	}, [onScenarioMutated]);

	// ── Inline amount edit ──────────────────────────────────────────────────
	const beginEdit = useCallback(
		(row: Extract<UnifiedRow, { kind: "planned" }>) => {
			if (!monthEditable) return;
			setEditingId(row.id);
			// Seed with the sign so the despesa/receita meaning is visible and
			// editable inline ("-" = despesa, positivo = receita).
			const mag = magnitudeOf(row.amount);
			setEditingValue(isNegative(row.amount) ? `-${mag}` : mag);
		},
		[monthEditable],
	);

	const commitEdit = useCallback(
		(row: Extract<UnifiedRow, { kind: "planned" }>) => {
			const magnitude = normalizeMagnitude(editingValue);
			setEditingId(null);
			if (!magnitude || Number(magnitude) === 0) return;
			// The typed sign wins: a leading "-" makes it a despesa, a leading "+"
			// (or a bare positive) an entrada; with no sign we keep the row's sign.
			const trimmed = editingValue.trim();
			const negative = trimmed.startsWith("-")
				? true
				: trimmed.startsWith("+")
					? false
					: isNegative(row.amount);
			const signed = negative ? `-${magnitude}` : magnitude;
			const action = routeSheetAmountEdit(rowRef(row.planned), activeScenarioId);
			const writeId = crypto.randomUUID();
			const now = Date.now();
			if (action.kind === "baselinePatch") {
				store.commit(
					events.forecastEnvelopeUpserted({
						writeId,
						forecastId: action.forecastId,
						description: "",
						amount: signed,
						dueDate: row.planned.dueDate,
						categoryId: null,
						upsertedAt: now,
					}),
				);
			} else if (action.kind === "scenarioAdjust" && activeScenarioId) {
				store.commit(
					events.scenarioChangeAdded({
						writeId,
						row: scenarioChangeRow(activeScenarioId, "adjust_amount", {
							changeId:
								row.planned.adjustChangeId ??
								`adj-${activeScenarioId}-${action.forecastId}`,
							targetForecastId: action.forecastId,
							amount: signed,
						}),
						addedAt: now,
					}),
				);
				notifyScenario();
			} else if (action.kind === "scenarioReplaceOneShot" && activeScenarioId) {
				const existing = activeChanges.find((c) => c.changeId === action.changeId);
				store.commit(
					events.scenarioChangeAdded({
						writeId,
						row: scenarioChangeRow(activeScenarioId, "add_one_shot", {
							changeId: action.changeId,
							month: existing?.month ?? month,
							amount: signed,
							description: existing?.description ?? row.description,
							categoryId: existing?.categoryId ?? null,
							accountId: existing?.accountId ?? null,
						}),
						addedAt: now,
					}),
				);
				notifyScenario();
			}
		},
		[editingValue, activeScenarioId, activeChanges, month, store, notifyScenario],
	);

	// Remove a scenario change by id (the inverse of every scenario-delta gesture).
	const removeScenarioChange = useCallback(
		(changeId: string) => {
			if (!activeScenarioId) return;
			store.commit(
				events.scenarioChangeRemoved({
					writeId: crypto.randomUUID(),
					changeId,
					scenarioId: activeScenarioId,
					removedAt: Date.now(),
				}),
			);
			notifyScenario();
		},
		[activeScenarioId, store, notifyScenario],
	);

	// ── Delete a planned row (with the recurring "só/em diante" popover) ─────
	const deletePlanned = useCallback(
		(planned: PlannedSheetRow, scope: SheetDeleteScope) => {
			const action = routeSheetDelete(rowRef(planned), scope, month, activeScenarioId);
			const writeId = crypto.randomUUID();
			const now = Date.now();
			setDeleteRowId(null);
			switch (action.kind) {
				case "baselineDelete":
					store.commit(
						events.forecastDeleted({ writeId, forecastId: action.forecastId, deletedAt: now }),
					);
					// A manual one-shot can be re-created verbatim (a fresh id, same fields).
					flashUndo("linha removida", () =>
						store.commit(
							events.forecastCreated({
								writeId: crypto.randomUUID(),
								description: planned.description,
								amount: planned.amount,
								dueDate: planned.dueDate,
								categoryId: planned.categoryId ?? null,
								accountId: planned.accountId ?? null,
								uiRole: "planned_transaction",
								createdAt: Date.now(),
							}),
						),
					);
					break;
				case "baselineDiscard":
					store.commit(
						events.forecastDiscarded({ writeId, forecastId: action.forecastId, discardedAt: now }),
					);
					// No un-discard endpoint on the bridge — confirm only.
					flashUndo("linha removida deste mês", null);
					break;
				case "baselineEndTemplate": {
					const forecastIds = forecasts
						.filter(
							(f) =>
								f.templateId === action.templateId &&
								f.status !== "descartado" &&
								(monthOf(f.dueDate) ?? "") >= action.effectiveFrom,
						)
						.map((f) => f.forecastId);
					store.commit(
						events.forecastTemplateEnded({
							writeId,
							templateId: action.templateId,
							effectiveFrom: action.effectiveFrom,
							forecastIds,
							endedAt: now,
						}),
					);
					flashUndo("recorrência encerrada", null);
					break;
				}
				case "scenarioSkip": {
					if (!activeScenarioId) break;
					const changeId =
						planned.skipChangeId ?? `skip-${activeScenarioId}-${action.forecastId}`;
					store.commit(
						events.scenarioChangeAdded({
							writeId,
							row: scenarioChangeRow(activeScenarioId, "skip_forecast", {
								changeId,
								targetForecastId: action.forecastId,
							}),
							addedAt: now,
						}),
					);
					notifyScenario();
					flashUndo("linha pulada", () => removeScenarioChange(changeId));
					break;
				}
				case "scenarioEndTemplate": {
					if (!activeScenarioId) break;
					const changeId = `end-${activeScenarioId}-${action.templateId}`;
					store.commit(
						events.scenarioChangeAdded({
							writeId,
							row: scenarioChangeRow(activeScenarioId, "end_template", {
								changeId,
								targetTemplateId: action.templateId,
								effectiveFrom: action.effectiveFrom,
							}),
							addedAt: now,
						}),
					);
					notifyScenario();
					flashUndo("recorrência encerrada no cenário", () =>
						removeScenarioChange(changeId),
					);
					break;
				}
				case "scenarioRemoveChange": {
					if (!activeScenarioId) break;
					store.commit(
						events.scenarioChangeRemoved({
							writeId,
							changeId: action.changeId,
							scenarioId: activeScenarioId,
							removedAt: now,
						}),
					);
					notifyScenario();
					// Re-add the removed scenario-added row (one-shot) with the same id.
					flashUndo("item do cenário removido", () =>
						store.commit(
							events.scenarioChangeAdded({
								writeId: crypto.randomUUID(),
								row: scenarioChangeRow(activeScenarioId, "add_one_shot", {
									changeId: action.changeId,
									month: monthOf(planned.dueDate) ?? month,
									amount: planned.amount,
									description: planned.description,
									categoryId: planned.categoryId ?? null,
									accountId: planned.accountId ?? null,
								}),
								addedAt: Date.now(),
							}),
						),
					);
					break;
				}
				case "none":
					break;
			}
		},
		[month, activeScenarioId, forecasts, store, notifyScenario, flashUndo, removeScenarioChange],
	);

	// ── Positional insert (design E) ────────────────────────────────────────
	const commitInsert = useCallback(
		(draft: InsertDraft) => {
			const magnitude = normalizeMagnitude(draft.magnitude);
			// Freeze the current order and remember where the new row goes, so it
			// stays put instead of jumping to its sorted-by-day position.
			const anchor = insertAfter;
			setInsertAfter(null);
			if (!magnitude || Number(magnitude) === 0) return;
			const insertAt =
				anchor === "end"
					? rows.length
					: anchor
						? rows.findIndex((r) => r.id === anchor) + 1
						: rows.length;
			setFrozen({ ids: rows.map((r) => r.id), insertAt });
			const signed = draft.isExpense ? `-${magnitude}` : magnitude;
			const day = Math.min(Math.max(draft.day, 1), daysInMonth(month));
			const dueDate = `${month}-${String(day).padStart(2, "0")}`;
			const categoryId = draft.categoryId?.trim() || null;
			const writeId = crypto.randomUUID();
			const now = Date.now();
			if (routeSheetAdd(activeScenarioId) === "forecastCreate") {
				store.commit(
					events.forecastCreated({
						writeId,
						description: draft.description,
						amount: signed,
						dueDate,
						categoryId,
						accountId: null,
						uiRole: "planned_transaction",
						createdAt: now,
					}),
				);
			} else if (activeScenarioId) {
				store.commit(
					events.scenarioChangeAdded({
						writeId,
						row: scenarioChangeRow(activeScenarioId, "add_one_shot", {
							month,
							amount: signed,
							description: draft.description,
							categoryId,
						}),
						addedAt: now,
					}),
				);
				notifyScenario();
			}
		},
		[month, activeScenarioId, store, notifyScenario, insertAfter, rows],
	);

	// ── Category picker + bulk actions (real transactions) ──────────────────
	const applyCategory = useCallback(
		(categoryId: string, targetIds: string[]) => {
			for (const transactionId of targetIds) {
				store.commit(
					events.reviewSubmitted({
						writeId: crypto.randomUUID(),
						transactionId,
						patch: { description: null, merchantName: null, purpose: null, categoryId },
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

	const txIndex = useMemo(
		() => rows.filter((r) => r.kind === "tx").map((r) => r.id),
		[rows],
	);

	const handleTxClick = useCallback(
		(tx: TxView, e: React.MouseEvent) => {
			const idx = txIndex.indexOf(tx.id);
			if (e.shiftKey && lastClickedIdx.current >= 0) {
				const start = Math.min(lastClickedIdx.current, idx);
				const end = Math.max(lastClickedIdx.current, idx);
				setSelectedIds(new Set(txIndex.slice(start, end + 1)));
			} else if (e.ctrlKey || e.metaKey) {
				setSelectedIds((prev) => {
					const next = new Set(prev);
					if (next.has(tx.id)) next.delete(tx.id);
					else next.add(tx.id);
					return next;
				});
				lastClickedIdx.current = idx;
			} else {
				lastClickedIdx.current = idx;
				setModalTx(tx);
			}
		},
		[txIndex],
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
			if (picker || modalTx || editingId) return;
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
					// Space toggles the focused real-transaction row in/out of the
					// selection (the keyboard equivalent of ⌘-click) so the bulk bar's
					// running sum + batch actions work without the mouse.
					const row = rows[focusedIdx];
					if (!row || row.kind !== "tx") break;
					e.preventDefault();
					setSelectedIds((prev) => {
						const next = new Set(prev);
						if (next.has(row.id)) next.delete(row.id);
						else next.add(row.id);
						return next;
					});
					lastClickedIdx.current = txIndex.indexOf(row.id);
					break;
				}
				case "Enter": {
					const row = rows[focusedIdx];
					if (!row) break;
					e.preventDefault();
					if (row.kind === "tx") setModalTx(row.tx);
					else beginEdit(row);
					break;
				}
				case "Escape":
					setSelectedIds(new Set());
					break;
			}
		},
		[picker, modalTx, editingId, rows, focusedIdx, beginEdit, txIndex],
	);

	const toggleSort = (key: SheetSortKey) => {
		// Any explicit sort abandons the frozen manual order.
		setFrozen(null);
		setSort((s) =>
			s.key === key
				? { key, dir: s.dir === 1 ? -1 : 1 }
				: { key, dir: key === "date" || key === "amount" ? -1 : 1 },
		);
	};

	const columnCount = COLUMNS.length + 2; // origin icon + actions

	const sortArrow = (key: SheetSortKey) =>
		sort.key === key ? (sort.dir === 1 ? " ↑" : " ↓") : "";

	return (
		<section aria-label={`Sheet for ${month}`}>
			{/* ── Month heading — keeps the working month obvious once the hero
			       has scrolled away (the sheet can be long). ── */}
			<div
				style={{
					display: "flex",
					alignItems: "baseline",
					gap: 10,
					padding: "4px 0 2px",
				}}
			>
				<span
					aria-hidden
					title={theme.season}
					style={{
						fontSize: "1.2rem",
						lineHeight: 1,
						alignSelf: "center",
						filter: "saturate(0.85)",
					}}
				>
					{theme.glyph}
				</span>
				<h2
					style={{
						fontFamily: "var(--font-display)",
						fontSize: "1.35rem",
						fontWeight: 700,
						letterSpacing: "-0.02em",
						margin: 0,
						// A hairline of the month's accent under the title — the one
						// place the theme colour is allowed to read as "this month".
						borderBottom: `2px solid ${theme.accent}`,
						paddingBottom: 1,
					}}
				>
					{monthLabel(month)}
				</h2>
				<span
					className="mono"
					style={{ fontSize: 12, color: "var(--muted)" }}
				>
					planilha
				</span>
				{!monthEditable && (
					<span
						className="mono"
						title="meses passados não são editáveis"
						style={{ fontSize: 11, color: "var(--muted2)" }}
					>
						· somente leitura
					</span>
				)}
			</div>

			{/* ── Scenario pills (ADR-0037) — creation gated to current+future ── */}
			{onActivateScenario && onScenarioMutated && (
				<SheetScenarioBar
					activeScenarioId={activeScenarioId}
					scenarioDelta={scenarioDelta}
					canCreate={monthEditable}
					onActivate={onActivateScenario}
					onMutated={onScenarioMutated}
				/>
			)}

			<SheetFilterBar
				ui={ui}
				setUi={setUi}
				accounts={accounts}
				localFilters={localFilters}
				setLocalFilters={setLocalFilters}
				count={rowCount}
				hasActiveFilters={hasSheetFilters}
				filteredTotal={totals.net}
				onExportCsv={handleExportCsv}
				accent={theme.accent}
			/>

			<div
				ref={tableRef}
				role="grid"
				aria-rowcount={rowCount}
				tabIndex={0}
				onKeyDown={handleKeyDown}
				style={{
					border: "1px solid var(--border)",
					borderTop: `2px solid ${theme.accent}`,
					borderRadius: "var(--radius-md)",
					// The sheet renders in full — no inner scroll. The page grows to the
					// table's height and the whole document scrolls as one; the sticky
					// thead then pins column headers to the viewport while scrolling.
					outline: "none",
					background: "var(--card)",
				}}
			>
				<datalist id="sheet-forecast-categories">
					{categories.map((category) => (
						<option key={category.id} value={category.id} />
					))}
				</datalist>
				<table
					style={{
						width: "100%",
						borderCollapse: "separate",
						borderSpacing: 0,
						fontSize: 14,
					}}
				>
					<thead>
						<tr className="mono">
							<th style={{ ...thStyle, width: 34 }}>
								<button
									onClick={() => toggleSort("origin")}
									className="mono"
									title="ordenar por origem"
									style={sortBtnStyle(sort.key === "origin", "left")}
								>
									·{sortArrow("origin")}
								</button>
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
										style={sortBtnStyle(sort.key === col.key, col.align ?? "left")}
									>
										{col.label}
										{sortArrow(col.key)}
									</button>
								</th>
							))}
							<th style={{ ...thStyle, width: 72 }} />
						</tr>
					</thead>
					<tbody className="sheet-tbody">
						{rows.map((row, idx) => (
							<React.Fragment key={`${row.kind}:${row.id}`}>
								<SheetRow
									row={row}
									idx={idx}
									focused={focusedIdx === idx}
									selected={row.kind === "tx" && selectedIds.has(row.id)}
									editable={monthEditable}
									editing={editingId === row.id}
									editingValue={editingValue}
									onEditingValueChange={setEditingValue}
									onBeginEdit={beginEdit}
									onCommitEdit={commitEdit}
									onCancelEdit={() => setEditingId(null)}
									onTxClick={handleTxClick}
									onCategoryClick={openPickerFor}
									onAskDelete={setDeleteRowId}
									onInsertAfter={setInsertAfter}
									deleteOpen={deleteRowId === row.id}
									onDelete={deletePlanned}
									onCloseDelete={() => setDeleteRowId(null)}
									month={month}
								/>
								{insertAfter === row.id && (
									<InsertRowEditor
										defaultDay={Number(month.slice(5)) === new Date().getMonth() + 1 ? new Date().getDate() : 1}
										maxDay={daysInMonth(month)}
										contextLabel={activeScenarioId ? "cenário ativo" : "baseline"}
										colSpan={columnCount}
										accent={theme.accent}
										onSubmit={commitInsert}
										onCancel={() => setInsertAfter(null)}
									/>
								)}
							</React.Fragment>
						))}
						{monthEditable && insertAfter !== "end" && (
							<tr>
								<td style={tdStyle} />
								<td
									colSpan={columnCount - 1}
									style={{ ...tdStyle, color: "var(--muted)", cursor: "pointer" }}
									onClick={() => setInsertAfter("end")}
								>
									<span className="mono" style={{ fontSize: 12.5 }}>
										+ adicionar linha…
									</span>
								</td>
							</tr>
						)}
						{insertAfter === "end" && (
							<InsertRowEditor
								defaultDay={Number(month.slice(5)) === new Date().getMonth() + 1 ? new Date().getDate() : 1}
								maxDay={daysInMonth(month)}
								contextLabel={activeScenarioId ? "cenário ativo" : "baseline"}
								colSpan={columnCount}
								accent={theme.accent}
								onSubmit={commitInsert}
								onCancel={() => setInsertAfter(null)}
							/>
						)}
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
						Nada para os filtros atuais.
					</div>
				)}
			</div>

			{/* Totals footer — the sheet status bar (design: previsto · Δ · net). */}
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
				<span>{rowCount} linhas</span>
				{frozen && (
					<button
						className="mono"
						onClick={() => setFrozen(null)}
						title="voltar à ordenação por coluna"
						style={{
							background: "transparent",
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-full)",
							padding: "1px 10px",
							cursor: "pointer",
							color: "var(--purple)",
							fontSize: 11,
						}}
					>
						ordem manual · reordenar
					</button>
				)}
				{totals.realizado !== 0 && (
					<span>
						realizado{" "}
						<span style={{ color: totals.realizado >= 0 ? "var(--green)" : "var(--rose)" }}>
							{formatMoneyNumber(totals.realizado)}
						</span>
					</span>
				)}
				<span>
					previsto{" "}
					<span style={{ color: "var(--purple)" }}>
						{formatMoneyNumber(totals.previsto)}
					</span>
				</span>
				{activeScenarioId && scenarioDelta != null && (
					<span style={{ color: "var(--cyan)" }}>
						Δ cenário {formatMoneyNumber(scenarioDelta)}
					</span>
				)}
				<span style={{ color: totals.net >= 0 ? "var(--green)" : "var(--rose)" }}>
					saldo {formatMoneyNumber(totals.net)}
				</span>
				<span style={{ marginLeft: "auto", opacity: 0.7 }}>
					↑↓ navega · espaço seleciona · Enter abre/edita · hover: + insere · 🗑 remove
				</span>
			</div>

			{/* Bulk-apply bar (real transactions only) */}
			{selectedIds.size > 0 && (
				<div
					role="toolbar"
					aria-label="ações em lote"
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
						{selectedIds.size} selecionad{selectedIds.size === 1 ? "o" : "os"} · {formatMoneyNumber(selectionTotal)}
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
						categorizar
					</button>
					<span
						aria-hidden
						style={{ width: 1, alignSelf: "stretch", background: "rgba(255,255,255,0.25)" }}
					/>
					<span className="mono" style={{ fontSize: 11, opacity: 0.7 }}>
						comprometimento:
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
						title="limpar comprometimento"
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
						× limpar
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
						limpar seleção
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

			{undo && (
				<div
					role="status"
					aria-live="polite"
					style={{
						position: "fixed",
						left: "50%",
						bottom: 24,
						transform: "translateX(-50%)",
						zIndex: 60,
						display: "flex",
						alignItems: "center",
						gap: 14,
						background: "var(--ink, #15131f)",
						color: "#fff",
						borderRadius: "var(--radius-full)",
						padding: "10px 18px",
						boxShadow: "0 8px 24px rgba(21,19,31,0.28)",
						fontSize: 13,
					}}
				>
					<span className="mono">{undo.label}</span>
					{undo.run ? (
						<button
							type="button"
							className="mono pressable"
							onClick={() => {
								undo.run?.();
								setUndo(null);
							}}
							style={{
								background: "var(--purple)",
								color: "#fff",
								border: "none",
								borderRadius: "var(--radius-full)",
								padding: "5px 14px",
								cursor: "pointer",
								fontSize: 12,
								fontWeight: 600,
							}}
						>
							↺ desfazer
						</button>
					) : (
						<span className="mono" style={{ fontSize: 11, opacity: 0.6 }}>
							não é possível desfazer
						</span>
					)}
					<button
						type="button"
						aria-label="fechar aviso"
						onClick={() => setUndo(null)}
						style={{
							background: "transparent",
							color: "rgba(255,255,255,0.7)",
							border: "none",
							cursor: "pointer",
							fontSize: 14,
							padding: 0,
						}}
					>
						×
					</button>
				</div>
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

// ── Row ─────────────────────────────────────────────────────────────────────

const SheetRow = React.memo(
	({
		row,
		idx,
		focused,
		selected,
		editable,
		editing,
		editingValue,
		onEditingValueChange,
		onBeginEdit,
		onCommitEdit,
		onCancelEdit,
		onTxClick,
		onCategoryClick,
		onAskDelete,
		onInsertAfter,
		deleteOpen,
		onDelete,
		onCloseDelete,
		month,
	}: {
		row: UnifiedRow;
		idx: number;
		focused: boolean;
		selected: boolean;
		editable: boolean;
		editing: boolean;
		editingValue: string;
		onEditingValueChange: (value: string) => void;
		onBeginEdit: (row: Extract<UnifiedRow, { kind: "planned" }>) => void;
		onCommitEdit: (row: Extract<UnifiedRow, { kind: "planned" }>) => void;
		onCancelEdit: () => void;
		onTxClick: (tx: TxView, e: React.MouseEvent) => void;
		onCategoryClick: (tx: TxView, anchor: HTMLElement) => void;
		onAskDelete: (rowId: string) => void;
		onInsertAfter: (rowId: string) => void;
		deleteOpen: boolean;
		onDelete: (planned: PlannedSheetRow, scope: SheetDeleteScope) => void;
		onCloseDelete: () => void;
		month: string;
	}) => {
		const negative = isNegative(row.amount);
		const skipped = row.kind === "planned" && row.planned.skipped;
		// Teal marks anything the active scenario touched: its own added rows,
		// plus baseline forecasts it adjusted or skipped.
		const scenario =
			row.kind === "planned" &&
			(row.origin === "scenario" ||
				!!row.planned.adjustChangeId ||
				row.planned.skipped);
		const teal = "var(--cyan)";

		const rowStyle: React.CSSProperties = {
			background: scenario
				? "rgba(8,145,178,0.06)"
				: row.kind === "planned"
					? "rgba(109,74,255,0.03)"
					: selected
						? "var(--purple-50, rgba(124,93,250,0.08))"
						: undefined,
			boxShadow: focused ? "inset 2px 0 0 var(--purple)" : undefined,
			opacity: skipped ? 0.6 : 1,
			userSelect: "none",
			position: "relative",
		};

		const originGlyph = SHEET_ORIGIN_ICONS[row.origin];
		const originTitle = ORIGIN_TITLE[row.origin];

		const amountColor = scenario
			? teal
			: row.kind === "planned"
				? "var(--purple)"
				: negative
					? "var(--rose)"
					: "var(--green)";

		return (
			<tr
				data-row-idx={idx}
				className={row.kind === "tx" && !selected ? "sheet-row-hover" : undefined}
				aria-selected={selected}
				style={rowStyle}
				onClick={row.kind === "tx" ? (e) => onTxClick(row.tx, e) : undefined}
			>
				{/* Origin glyph + hover insert affordance */}
				<td style={{ ...tdStyle, textAlign: "center", position: "relative", color: scenario ? teal : "var(--muted)" }}>
					<span title={originTitle} aria-label={originTitle} style={{ fontSize: 13 }}>
						{originGlyph}
					</span>
					{editable && (
						<InsertHandle
							label="inserir linha aqui"
							onClick={() => onInsertAfter(row.id)}
						/>
					)}
				</td>

				{/* Description */}
				<td
					style={{ ...tdStyle, maxWidth: 380, color: scenario ? "#0b7285" : undefined }}
					title={row.kind === "tx" ? row.tx.rawDescription : undefined}
				>
					<div style={{ display: "flex", alignItems: "center", gap: 6, overflow: "hidden" }}>
						{row.kind === "tx" && negative && <TierBadge tier={row.tier} compact />}
						<span
							style={{
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
								textDecoration: skipped ? "line-through" : undefined,
							}}
						>
							{row.description}
						</span>
						{row.kind === "planned" && row.planned.installmentLabel && (
							<span className="mono" style={{ fontSize: 11, color: scenario ? teal : "var(--muted)" }}>
								{row.planned.installmentLabel}
							</span>
						)}
						{skipped && (
							<span className="mono" style={{ fontSize: 11, color: teal }}>
								pulado
							</span>
						)}
					</div>
					{row.kind === "tx" && row.tx.purpose && (
						<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
							{row.tx.purpose}
						</div>
					)}
				</td>

				{/* Category */}
				<td style={tdStyle} onClick={(e) => e.stopPropagation()}>
					{row.kind === "tx" ? (
						<button
							data-cat-chip
							onClick={(e) => onCategoryClick(row.tx, e.currentTarget)}
							className="mono"
							title="mudar categoria (Enter)"
							style={{ ...categoryChipStyle(!!row.category), cursor: "pointer" }}
						>
							{row.category
								? `${categoryEmoji(row.category, !negative)} ${row.category}`
								: "❓ sem categoria"}
						</button>
					) : (
						<span className="mono" style={categoryChipStyle(!!row.category)}>
							{row.category
								? `${categoryEmoji(row.category, !negative)} ${row.category}`
								: "❓ sem categoria"}
						</span>
					)}
				</td>

				{/* Date — day only; the sheet is always a single month. */}
				<td
					className="mono"
					style={{ ...tdStyle, color: "var(--muted)", textAlign: "center" }}
				>
					{Number(row.date.slice(8, 10))}
				</td>

				{/* Amount (inline-editable for planned rows) */}
				<td
					className="mono"
					style={{
						...tdStyle,
						textAlign: "right",
						color: amountColor,
						fontVariantNumeric: "tabular-nums",
						whiteSpace: "nowrap",
					}}
					onClick={(e) => {
						if (row.kind === "planned" && editable && !editing) {
							e.stopPropagation();
							onBeginEdit(row);
						}
					}}
				>
					{editing && row.kind === "planned" ? (
						<input
							autoFocus
							value={editingValue}
							inputMode="decimal"
							onChange={(e) => onEditingValueChange(e.target.value)}
							onBlur={() => onCommitEdit(row)}
							onKeyDown={(e) => {
								if (e.key === "Enter") onCommitEdit(row);
								if (e.key === "Escape") onCancelEdit();
							}}
							onClick={(e) => e.stopPropagation()}
							className="mono"
							style={{
								width: 110,
								textAlign: "right",
								fontSize: 13,
								padding: "4px 6px",
								border: `1px solid ${teal}`,
								borderRadius: "var(--radius-sm)",
								background: "var(--card)",
							}}
						/>
					) : (
						<span
							style={{
								cursor: row.kind === "planned" && editable ? "text" : "default",
								textDecoration:
									row.kind === "planned" && row.planned.originalAmount
										? undefined
										: skipped
											? "line-through"
											: undefined,
							}}
						>
							{row.kind === "planned" && row.planned.originalAmount && (
								<s style={{ color: "var(--muted)", marginRight: 6 }}>
									{sheetAmountLabel(row.planned.originalAmount)}
								</s>
							)}
							{sheetAmountLabel(row.amount)}
						</span>
					)}
				</td>

				{/* Hover actions */}
				<td
					style={{ ...tdStyle, textAlign: "right", position: "relative" }}
					onClick={(e) => e.stopPropagation()}
				>
					{row.kind === "planned" && editable && (
						<span className="sheet-row-actions">
							<button
								className="mono"
								title={
									row.planned.templateId
										? "remover — só neste mês ou em diante"
										: "remover"
								}
								onClick={() =>
									row.planned.templateId
										? onAskDelete(row.id)
										: onDelete(row.planned, "month")
								}
								style={rowActionBtnStyle}
							>
								🗑
							</button>
						</span>
					)}
					{deleteOpen && row.kind === "planned" && (
						<DeleteScopePopover
							month={month}
							onScope={(scope) => onDelete(row.planned, scope)}
							onClose={onCloseDelete}
						/>
					)}
				</td>
			</tr>
		);
	},
);

// ── Delete-scope popover (recurring rows) ────────────────────────────────────

const DeleteScopePopover = ({
	month,
	onScope,
	onClose,
}: {
	month: string;
	onScope: (scope: SheetDeleteScope) => void;
	onClose: () => void;
}) => {
	const monthName = new Date(`${month}-15`).toLocaleString("pt-BR", { month: "long" });
	return (
		<>
			<div
				onClick={onClose}
				style={{ position: "fixed", inset: 0, zIndex: 40 }}
				aria-hidden
			/>
			<div
				role="menu"
				style={{
					position: "absolute",
					right: 8,
					top: "100%",
					zIndex: 41,
					width: 230,
					background: "var(--card)",
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-md)",
					boxShadow: "0 8px 24px rgba(21,19,31,0.18)",
					padding: 8,
				}}
			>
				<button
					className="mono"
					onClick={() => onScope("month")}
					style={popoverBtnStyle}
				>
					📅 só em {monthName}
				</button>
				<button
					className="mono"
					onClick={() => onScope("onward")}
					style={popoverBtnStyle}
				>
					✂ de {monthName} em diante
				</button>
				<p style={{ fontSize: 11, color: "var(--muted)", margin: "6px 2px 0" }}>
					no cenário vira mudança; no baseline grava direto
				</p>
			</div>
		</>
	);
};

const popoverBtnStyle: React.CSSProperties = {
	display: "block",
	width: "100%",
	textAlign: "left",
	background: "transparent",
	border: "none",
	borderRadius: "var(--radius-sm)",
	padding: "7px 8px",
	cursor: "pointer",
	fontSize: 12.5,
	color: "var(--text)",
};

const sortBtnStyle = (active: boolean, align: "left" | "right"): React.CSSProperties => ({
	background: "transparent",
	border: "none",
	cursor: "pointer",
	color: active ? "var(--purple)" : "var(--muted)",
	fontSize: 12,
	textTransform: "uppercase",
	letterSpacing: "0.06em",
	padding: 0,
	textAlign: align,
	width: "100%",
});
