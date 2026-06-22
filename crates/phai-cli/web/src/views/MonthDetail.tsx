import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { events, tables } from "../livestore/schema";
import {
	commitmentTier,
	effectiveTx,
	fixedCategoriesFromForecasts,
	type CommitmentTier,
} from "../lib/derivations";
import { formatMoneyNumber, isNegative, sumAmounts } from "../lib/format";
import { useDebounce } from "../hooks/useDebounce";
import { CategoryTreemap } from "./categorias/CategoryTreemap";
import { TransactionModal } from "../components/TransactionModal";
import type { ChartMonthView, ForecastView } from "./types";
import { ForecastSection } from "./month/ForecastSection";
import { ManualPlannedTransactions } from "./month/ManualPlannedTransactions";
import { FilterBar, FilterSummary } from "./month/MonthFilters";

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
	// Fixed-category set drives the commitment tier (ADR-0030).
	const fixedCategories = useMemo(
		() => fixedCategoriesFromForecasts(forecasts),
		[forecasts],
	);

	// Effective category (overlay first, then seed)
	const effectiveCat = useCallback(
		(tx: TxView) => overlayById.get(tx.id)?.categoryId ?? tx.categoryId,
		[overlayById],
	);

	// Transactions for this month, with the optimistic overlay baked in so edits
	// reflect in the treemap/sums immediately (not just the modal).
	const monthTxs = useMemo(
		() =>
			txRows
				.filter((t) => t.month === month)
				.map((t) => ({
					...effectiveTx(t, overlayById),
					accountLabel: accountById.get(t.accountId)?.label || t.accountId,
				})),
		[txRows, month, accountById, overlayById],
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
			if (
				ui.tierFilter &&
				commitmentTier(tx, fixedCategories) !==
					(ui.tierFilter as CommitmentTier)
			)
				return false;
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
	}, [
		monthTxs,
		overlayById,
		accountById,
		ui,
		debouncedText,
		effectiveCat,
		fixedCategories,
	]);

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
		!!ui.tierFilter ||
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
			<ManualPlannedTransactions month={month} />
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
					fixedCategories={fixedCategories}
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
	const monthName = new Date(month + "-15").toLocaleString("en-US", {
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
						forecast
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
					{installmentCount} installment{installmentCount !== 1 ? "s" : ""} ·{" "}
					{formatMoneyNumber(-installmentSum)} ·{" "}
					{forecastCount > 0 ? `${forecastCount} forecasts` : ""}
				</div>
			)}
		</div>
	);
};
