import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { events, tables } from "../livestore/schema";
import {
	buildAccountMap,
	buildOverlayMap,
	effectiveTx,
	sheetLabel,
	type TxView,
} from "../lib/derivations";
import { amountColor, formatMoney, toCents } from "../lib/format";
import { categoryEmoji } from "../lib/categoryEmoji";
import {
	TransactionModal,
	type ReviewPatch,
} from "./TransactionModal";

const txAll$ = queryDb(tables.transactions.orderBy("postedAt", "desc"));
const overlay$ = queryDb(tables.reviewOverlay);
const accounts$ = queryDb(tables.accounts.orderBy("label", "asc"));
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));

const MAX_RESULTS = 20;

interface ScoredTx {
	tx: TxView;
	score: number;
}

/**
 * Score a transaction against a search query. Higher = better match.
 *   3 = exact field match
 *   2 = field starts with query
 *   1 = field contains query
 * Multiple field matches add up so multi-signal results rank higher.
 */
const scoreTx = (tx: TxView, query: string): number => {
	const q = query.toLowerCase();
	let score = 0;

	const fields = [
		tx.description,
		tx.rawDescription,
		tx.merchantName,
		tx.purpose,
		tx.categoryId,
		tx.accountLabel,
		tx.installmentMarker,
		tx.postedAt,
	];

	for (const field of fields) {
		if (!field) continue;
		const f = field.toLowerCase();
		if (f === q) {
			score += 3;
		} else if (f.startsWith(q)) {
			score += 2;
		} else if (f.includes(q)) {
			score += 1;
		}
	}

	// Amount matching: search "150" matches amounts around 150 (or -150).
	const amountNum = Math.abs(toCents(tx.amount)) / 100;
	const queryNum = Number(q.replace(/[.,]/g, (c) => (c === "," ? "." : "")));
	if (Number.isFinite(queryNum) && queryNum > 0) {
		const amountStr = amountNum.toFixed(2);
		if (amountStr === queryNum.toFixed(2)) {
			score += 3;
		} else if (amountStr.startsWith(String(queryNum))) {
			score += 2;
		} else if (amountStr.includes(String(queryNum))) {
			score += 1;
		}
	}

	return score;
};

export const SearchPalette = ({
	open,
	onClose,
}: {
	open: boolean;
	onClose: () => void;
}) => {
	const { store } = useStore();
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const rOverlay = useQuery(overlay$);
	const accountRows = useQuery(accounts$);
	const categoryIds = useQuery(categories$);

	const overlayMap = useMemo(
		() => buildOverlayMap(rOverlay as never),
		[rOverlay],
	);
	const accountMap = useMemo(() => buildAccountMap(accountRows), [accountRows]);

	// Enrich transactions with account labels + overlays
	const enrichedTxs = useMemo(
		() =>
			txRows.map((tx) => {
				const eff = effectiveTx(tx, overlayMap);
				return {
					...eff,
					accountLabel: accountMap.get(eff.accountId)?.label ?? eff.accountId,
				};
			}),
		[txRows, overlayMap, accountMap],
	);

	const [query, setQuery] = useState("");
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [modalTx, setModalTx] = useState<TxView | null>(null);
	const inputRef = useRef<HTMLInputElement>(null);
	const listRef = useRef<HTMLDivElement>(null);

	// Reset on open/close
	useEffect(() => {
		if (open) {
			setQuery("");
			setSelectedIndex(0);
			setModalTx(null);
			// Delay focus to after animation
			requestAnimationFrame(() => inputRef.current?.focus());
		}
	}, [open]);

	// Search results
	const { results, totalCount } = useMemo(() => {
		const q = query.trim();
		if (!q) return { results: [], totalCount: 0 };

		const scored: ScoredTx[] = [];
		for (const tx of enrichedTxs) {
			const s = scoreTx(tx, q);
			if (s > 0) scored.push({ tx, score: s });
		}
		scored.sort((a, b) => b.score - a.score || (a.tx.postedAt < b.tx.postedAt ? 1 : -1));
		return {
			results: scored.slice(0, MAX_RESULTS),
			totalCount: scored.length,
		};
	}, [query, enrichedTxs]);

	// Clamp selection
	useEffect(() => {
		setSelectedIndex((prev) => Math.min(prev, Math.max(0, results.length - 1)));
	}, [results.length]);

	// Scroll selected into view
	useEffect(() => {
		const list = listRef.current;
		if (!list) return;
		const item = list.children[selectedIndex] as HTMLElement | undefined;
		item?.scrollIntoView({ block: "nearest" });
	}, [selectedIndex]);

	const openResult = useCallback(
		(tx: TxView) => {
			setModalTx(tx);
		},
		[],
	);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "ArrowDown") {
				e.preventDefault();
				setSelectedIndex((i) => Math.min(i + 1, results.length - 1));
			} else if (e.key === "ArrowUp") {
				e.preventDefault();
				setSelectedIndex((i) => Math.max(i - 1, 0));
			} else if (e.key === "Enter" && results.length > 0) {
				e.preventDefault();
				openResult(results[selectedIndex].tx);
			} else if (e.key === "Escape") {
				e.preventDefault();
				onClose();
			}
		},
		[results, selectedIndex, openResult, onClose],
	);

	const submitModal = useCallback(
		(txId: string, patch: ReviewPatch) => {
			const writeId = crypto.randomUUID();
			store.commit(
				events.reviewSubmitted({
					writeId,
					transactionId: txId,
					patch,
					submittedAt: Date.now(),
				}),
			);
			setModalTx(null);
		},
		[store],
	);

	// Find similar transactions for the modal
	const similarTxs = useMemo(() => {
		if (!modalTx) return [];
		const label = sheetLabel(modalTx).toLowerCase();
		return enrichedTxs.filter(
			(t) => t.id !== modalTx.id && sheetLabel(t).toLowerCase() === label,
		);
	}, [modalTx, enrichedTxs]);

	if (!open) return null;

	return (
		<>
			<AnimatePresence>
				{open && !modalTx && (
					<motion.div
						key="search-backdrop"
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						exit={{ opacity: 0 }}
						transition={{ duration: 0.15 }}
						onClick={onClose}
						style={{
							position: "fixed",
							inset: 0,
							zIndex: 200,
							background: "rgba(21, 19, 31, 0.45)",
							backdropFilter: "blur(6px)",
							WebkitBackdropFilter: "blur(6px)",
						}}
					/>
				)}
			</AnimatePresence>

			<AnimatePresence>
				{open && !modalTx && (
					<motion.div
						key="search-panel"
						initial={{ opacity: 0, y: -20, scale: 0.98 }}
						animate={{ opacity: 1, y: 0, scale: 1 }}
						exit={{ opacity: 0, y: -20, scale: 0.98 }}
						transition={{ duration: 0.15 }}
						style={{
							position: "fixed",
							top: "12vh",
							left: "50%",
							transform: "translateX(-50%)",
							zIndex: 201,
							width: "min(600px, 92vw)",
							maxHeight: "70vh",
							background: "var(--bg)",
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-lg)",
							boxShadow: "0 16px 48px rgba(21, 19, 31, 0.25)",
							display: "flex",
							flexDirection: "column",
							overflow: "hidden",
						}}
						onClick={(e) => e.stopPropagation()}
					>
						{/* Search input */}
						<div
							style={{
								display: "flex",
								alignItems: "center",
								gap: 10,
								padding: "14px 16px",
								borderBottom: "1px solid var(--border)",
							}}
						>
							<span style={{ fontSize: 16, color: "var(--muted)", flexShrink: 0 }}>
								/
							</span>
							<input
								ref={inputRef}
								type="text"
								placeholder="Search transactions..."
								value={query}
								onChange={(e) => {
									setQuery(e.target.value);
									setSelectedIndex(0);
								}}
								onKeyDown={handleKeyDown}
								className="mono"
								style={{
									flex: 1,
									background: "transparent",
									border: "none",
									outline: "none",
									fontSize: 15,
									fontFamily: "var(--font-mono)",
									color: "var(--white)",
								}}
							/>
							<kbd
								className="mono"
								style={{
									fontSize: 10,
									color: "var(--muted)",
									background: "var(--surface)",
									border: "1px solid var(--border)",
									borderRadius: 4,
									padding: "2px 6px",
									flexShrink: 0,
								}}
							>
								esc
							</kbd>
						</div>

						{/* Results */}
						<div
							ref={listRef}
							style={{
								overflowY: "auto",
								flex: 1,
								padding: query.trim() ? "4px 0" : 0,
							}}
						>
							{query.trim() && results.length === 0 && (
								<div
									className="mono"
									style={{
										padding: "24px 16px",
										textAlign: "center",
										color: "var(--muted)",
										fontSize: 13,
									}}
								>
									No transactions found
								</div>
							)}
							{results.map(({ tx }, i) => (
								<button
									key={tx.id}
									type="button"
									onClick={() => openResult(tx)}
									onMouseEnter={() => setSelectedIndex(i)}
									style={{
										width: "100%",
										display: "grid",
										gridTemplateColumns: "70px 1fr auto",
										gap: 8,
										alignItems: "center",
										padding: "8px 16px",
										border: "none",
										cursor: "pointer",
										textAlign: "left",
										fontSize: 12,
										fontFamily: "var(--font-mono)",
										background:
											i === selectedIndex
												? "var(--surface)"
												: "transparent",
										transition: "background 80ms",
									}}
								>
									{/* Date */}
									<span style={{ color: "var(--muted)", fontSize: 11 }}>
										{tx.postedAt.slice(5, 10).replace("-", "/")}
									</span>

									{/* Description + account + category */}
									<div
										style={{
											display: "flex",
											flexDirection: "column",
											gap: 2,
											overflow: "hidden",
										}}
									>
										<span
											style={{
												color: "var(--white)",
												whiteSpace: "nowrap",
												overflow: "hidden",
												textOverflow: "ellipsis",
											}}
										>
											{sheetLabel(tx)}
										</span>
										<div
											style={{
												display: "flex",
												gap: 8,
												alignItems: "center",
												fontSize: 11,
												color: "var(--muted)",
											}}
										>
											<span
												style={{
													whiteSpace: "nowrap",
													overflow: "hidden",
													textOverflow: "ellipsis",
													maxWidth: 120,
												}}
											>
												{tx.accountLabel}
											</span>
											{tx.categoryId && (
												<span
													style={{
														whiteSpace: "nowrap",
														overflow: "hidden",
														textOverflow: "ellipsis",
														maxWidth: 140,
													}}
												>
													{categoryEmoji(tx.categoryId)}{" "}
													{tx.categoryId}
												</span>
											)}
											{tx.installmentMarker && (
												<span style={{ flexShrink: 0 }}>
													{tx.installmentMarker}
												</span>
											)}
										</div>
									</div>

									{/* Amount */}
									<span
										style={{
											color: amountColor(tx.amount),
											whiteSpace: "nowrap",
											fontWeight: 600,
											fontSize: 12,
										}}
									>
										{formatMoney(tx.amount)}
									</span>
								</button>
							))}
						</div>

						{/* Footer */}
						{query.trim() && totalCount > 0 && (
							<div
								className="mono"
								style={{
									padding: "8px 16px",
									borderTop: "1px solid var(--border)",
									fontSize: 11,
									color: "var(--muted)",
									display: "flex",
									justifyContent: "space-between",
									alignItems: "center",
								}}
							>
								<span>
									{totalCount} result{totalCount !== 1 ? "s" : ""}
									{totalCount > MAX_RESULTS && ` (showing ${MAX_RESULTS})`}
								</span>
								<span>
									<kbd style={kbdStyle}>&#8593;&#8595;</kbd> navigate{" "}
									<kbd style={kbdStyle}>&#8629;</kbd> open
								</span>
							</div>
						)}

						{/* Empty state hint */}
						{!query.trim() && (
							<div
								className="mono"
								style={{
									padding: "20px 16px",
									textAlign: "center",
									color: "var(--muted)",
									fontSize: 12,
								}}
							>
								Type to search by description, merchant, amount, category, account, or date
							</div>
						)}
					</motion.div>
				)}
			</AnimatePresence>

			{/* Transaction edit modal */}
			<AnimatePresence>
				{modalTx && (
					<TransactionModal
						tx={modalTx}
						overlay={overlayMap.get(modalTx.id)}
						similarTxs={similarTxs}
						overlayById={overlayMap}
						categories={categoryIds.map((c) => c.id)}
						onSubmit={(patch) => submitModal(modalTx.id, patch)}
						onClose={() => setModalTx(null)}
					/>
				)}
			</AnimatePresence>
		</>
	);
};

const kbdStyle: React.CSSProperties = {
	fontSize: 10,
	background: "var(--surface)",
	border: "1px solid var(--border)",
	borderRadius: 3,
	padding: "1px 5px",
	marginInline: 2,
};
