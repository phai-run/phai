import { memo, useCallback, useRef } from "react";
import { amountColor, formatMoney } from "../lib/format";

// ── Types ──────────────────────────────────────────────────────────────────

export interface TxView {
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

interface OverlayEntry {
	categoryId: string | null;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
}

// ── Estimated row height for content-visibility ────────────────────────────

/** Estimated pixel height of a single transaction row. Used by
 *  content-visibility: auto to preserve scroll height for off-screen rows. */
const ROW_HEIGHT_ESTIMATE = 52;

// ── TxRow (memoized) ───────────────────────────────────────────────────────

interface TxRowProps {
	tx: TxView;
	overlay?: OverlayEntry;
	onEdit: (tx: TxView) => void;
	/** Selection/focus state */
	isSelected?: boolean;
	isFocused?: boolean;
	/** Click event with modifier info for batch selection */
	onClick?: (tx: TxView, e: React.MouseEvent) => void;
	/** Show a drag handle for drag-to-recategorize */
	showDragHandle?: boolean;
	onDragStart?: (tx: TxView, e: React.PointerEvent) => void;
}

/**
 * A single transaction row rendered inside a CategoryGroup.
 *
 * Memoized with a shallow comparator that re-renders only when the
 * transaction id, overlay reference, selection state, or handler
 * identities change.  Uses content-visibility: auto so the browser can
 * skip rendering off-screen rows, preserving scroll height via
 * contain-intrinsic-size.
 */
export const TxRow = memo(
	({
		tx,
		overlay,
		onEdit,
		isSelected = false,
		isFocused = false,
		onClick,
		showDragHandle = false,
		onDragStart,
	}: TxRowProps) => {
		const btnRef = useRef<HTMLButtonElement>(null);

		const display =
			overlay?.description ??
			tx.description ??
			overlay?.merchantName ??
			tx.merchantName ??
			tx.rawDescription;
		const cat = overlay?.categoryId ?? tx.categoryId;

		const handleClick = useCallback(
			(e: React.MouseEvent) => {
				if (onClick) {
					onClick(tx, e);
				} else {
					onEdit(tx);
				}
			},
			[onClick, onEdit, tx],
		);

		const handleMouseEnter = useCallback(() => {
			if (btnRef.current) btnRef.current.style.background = "rgba(0,0,0,0.02)";
		}, []);

		const handleMouseLeave = useCallback(() => {
			if (btnRef.current) btnRef.current.style.background = "transparent";
		}, []);

		const handlePointerDown = useCallback(
			(e: React.PointerEvent) => {
				if (e.button !== 0) return;
				if (onDragStart) {
					onDragStart(tx, e);
				}
			},
			[onDragStart, tx],
		);

		return (
			<button
				ref={btnRef}
				onClick={handleClick}
				onMouseEnter={handleMouseEnter}
				onMouseLeave={handleMouseLeave}
				data-tx-id={tx.id}
				style={{
					width: "100%",
					display: "flex",
					alignItems: "center",
					gap: 12,
					padding: "9px 14px",
					background: isSelected ? "rgba(109,74,255,0.06)" : "transparent",
					border: "none",
					borderTop: "1px solid var(--border)",
					borderLeft: isSelected
						? "2px solid var(--purple)"
						: "2px solid transparent",
					cursor: onDragStart ? "grab" : "pointer",
					textAlign: "left" as const,
					transition: "background 80ms, border-color 80ms",
					contentVisibility: "auto",
					containIntrinsicSize: `auto ${ROW_HEIGHT_ESTIMATE}px`,
					outline: isFocused ? "1px solid var(--purple)" : "none",
					outlineOffset: -1,
					position: "relative",
					touchAction: onDragStart ? "none" : undefined,
					userSelect: onDragStart ? "none" : undefined,
				}}
			>
				{/* Drag handle */}
				{showDragHandle && (
					<span
						onPointerDown={handlePointerDown}
						style={{
							color: "var(--muted2)",
							fontSize: 12,
							cursor: "grab",
							lineHeight: 1,
							padding: "2px 0",
							touchAction: "none",
							userSelect: "none" as const,
						}}
						title="arraste para recategorizar"
					>
						⠿
					</span>
				)}

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
						{tx.reviewed === 1 && <TagBadge label="✓" color="var(--green)" />}
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
	},
	(prev, next) =>
		prev.tx.id === next.tx.id &&
		prev.overlay === next.overlay &&
		prev.onEdit === next.onEdit &&
		prev.isSelected === next.isSelected &&
		prev.isFocused === next.isFocused &&
		prev.onClick === next.onClick &&
		prev.onDragStart === next.onDragStart &&
		prev.showDragHandle === next.showDragHandle,
);

// ── Re-export TagBadge (used by MonthDetail, SimilarPanel) ─────────────────

export const TagBadge = ({
	label,
	color,
}: {
	label: string;
	color: string;
}) => (
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
