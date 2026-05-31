import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { motion } from "framer-motion";

// ── Types ──────────────────────────────────────────────────────────────────

export interface CategoryPickerProps {
	categories: ReadonlyArray<string>;
	/** Recently used categories (most recent first). */
	recentCategories: string[];
	anchorRect: DOMRect;
	selectedCount: number;
	onSelect: (categoryId: string) => void;
	onClose: () => void;
}

/**
 * Quick category command palette (Ctrl/Cmd+K).
 *
 * Opens as a popover near the selected transaction row.  Shows recently used
 * categories first, then all categories alphabetically.  Type to filter.
 * Enter applies the category immediately (no modal needed).
 */
export const CategoryPicker = ({
	categories,
	recentCategories,
	anchorRect,
	selectedCount,
	onSelect,
	onClose,
}: CategoryPickerProps) => {
	const [query, setQuery] = useState("");
	const [highlightIdx, setHighlightIdx] = useState(0);
	const inputRef = useRef<HTMLInputElement>(null);

	// Focus the input on mount
	useEffect(() => {
		inputRef.current?.focus();
	}, []);

	// Build filtered list: recent first, then rest, filtered by query
	const filtered = useMemo(() => {
		const q = query.trim().toLowerCase();
		const recentSet = new Set(recentCategories);

		const match = (c: string) => !q || c.toLowerCase().includes(q);

		const filteredRecent = recentCategories.filter(match);
		const filteredRest = categories.filter(
			(c) => !recentSet.has(c) && match(c),
		);

		return [...filteredRecent, ...filteredRest];
	}, [categories, recentCategories, query]);

	// Clamp highlight index
	const safeIdx = Math.min(highlightIdx, filtered.length - 1);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			switch (e.key) {
				case "ArrowDown":
					e.preventDefault();
					setHighlightIdx((i) => Math.min(i + 1, filtered.length - 1));
					break;
				case "ArrowUp":
					e.preventDefault();
					setHighlightIdx((i) => Math.max(i - 1, 0));
					break;
				case "Enter":
					e.preventDefault();
					if (filtered[safeIdx]) onSelect(filtered[safeIdx]);
					break;
				case "Escape":
					e.preventDefault();
					onClose();
					break;
			}
		},
		[filtered, safeIdx, onSelect, onClose],
	);

	// Compute popover position relative to the anchor row
	const style = useMemo((): React.CSSProperties => {
		const padding = 8;
		const maxHeight = 340;
		const width = 280;

		// Place below the anchor if there's room, otherwise above
		const belowSpace = window.innerHeight - anchorRect.bottom - padding;
		const aboveSpace = anchorRect.top - padding;
		const placeBelow = belowSpace >= maxHeight || belowSpace >= aboveSpace;

		return {
			position: "fixed",
			zIndex: 80,
			width,
			maxHeight,
			overflowY: "auto",
			left: Math.max(
				padding,
				Math.min(anchorRect.left, window.innerWidth - width - padding),
			),
			top: Math.max(
				padding,
				placeBelow ? anchorRect.bottom + 4 : anchorRect.top - maxHeight - 4,
			),
			background: "var(--bg)",
			border: "1px solid var(--border)",
			borderRadius: "var(--radius-lg)",
			boxShadow: "0 12px 40px rgba(21,19,31,0.2)",
		};
	}, [anchorRect]);

	return (
		<>
			{/* Backdrop to catch outside clicks */}
			<motion.div
				initial={{ opacity: 0 }}
				animate={{ opacity: 1 }}
				exit={{ opacity: 0 }}
				transition={{ duration: 0.1 }}
				onClick={onClose}
				style={{
					position: "fixed",
					inset: 0,
					zIndex: 79,
				}}
			/>
			<motion.div
				initial={{ opacity: 0, scale: 0.96, y: -4 }}
				animate={{ opacity: 1, scale: 1, y: 0 }}
				exit={{ opacity: 0, scale: 0.96, y: -4 }}
				transition={{ duration: 0.12, ease: "easeOut" }}
				style={style}
				onClick={(e) => e.stopPropagation()}
			>
				{/* Search input */}
				<div
					style={{
						padding: "8px 10px",
						borderBottom: "1px solid var(--border)",
					}}
				>
					<input
						ref={inputRef}
						value={query}
						onChange={(e) => {
							setQuery(e.target.value);
							setHighlightIdx(0);
						}}
						onKeyDown={handleKeyDown}
						placeholder={
							selectedCount > 1
								? `categoria para ${selectedCount} transações…`
								: "categoria…"
						}
						className="mono"
						style={{
							width: "100%",
							background: "transparent",
							border: "none",
							color: "var(--white)",
							fontSize: 13,
							outline: "none",
							padding: "4px 0",
						}}
					/>
				</div>

				{/* Category list */}
				<div style={{ padding: "4px 0" }}>
					{filtered.length === 0 && (
						<div
							className="mono"
							style={{
								padding: "12px 14px",
								color: "var(--muted)",
								fontSize: 12,
								textAlign: "center",
							}}
						>
							nenhuma categoria encontrada
						</div>
					)}
					{filtered.map((cat, idx) => {
						const isRecent = recentCategories.includes(cat);
						return (
							<button
								key={cat}
								onClick={() => onSelect(cat)}
								onMouseEnter={() => setHighlightIdx(idx)}
								style={{
									width: "100%",
									display: "flex",
									alignItems: "center",
									gap: 8,
									padding: "6px 14px",
									background:
										idx === safeIdx ? "rgba(109,74,255,0.08)" : "transparent",
									border: "none",
									cursor: "pointer",
									textAlign: "left",
									fontSize: 13,
									color: "var(--white)",
								}}
							>
								<span
									style={{ color: "var(--cyan)", fontSize: 11, minWidth: 16 }}
								>
									{isRecent ? "⏱" : ""}
								</span>
								<span
									style={{
										flex: 1,
										overflow: "hidden",
										textOverflow: "ellipsis",
										whiteSpace: "nowrap",
									}}
								>
									{cat}
								</span>
							</button>
						);
					})}
				</div>
				<div
					className="mono"
					style={{
						padding: "6px 14px",
						borderTop: "1px solid var(--border)",
						fontSize: 10,
						color: "var(--muted2)",
						textAlign: "right",
					}}
				>
					↑↓ navegar · ↵ aplicar · esc fechar
				</div>
			</motion.div>
		</>
	);
};
