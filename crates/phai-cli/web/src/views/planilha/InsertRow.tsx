import { useState } from "react";

/**
 * Positional insert (design E): the round "+" that appears on the boundary of
 * a hovered row, and the inline editor row it opens. The visual position is
 * cosmetic — the date decides where the row sorts.
 */

/** The "+" affordance rendered inside a row's first cell (hover-revealed). */
export const InsertHandle = ({
	onClick,
	label,
}: {
	onClick: () => void;
	label: string;
}) => (
	<button
		className="sheet-insert-btn mono"
		aria-label={label}
		title={label}
		onClick={(e) => {
			e.stopPropagation();
			onClick();
		}}
		style={{
			position: "absolute",
			bottom: -10,
			left: 4,
			zIndex: 3,
			width: 18,
			height: 18,
			borderRadius: "50%",
			border: "none",
			background: "var(--purple)",
			color: "#fff",
			fontSize: 13,
			lineHeight: "18px",
			padding: 0,
			cursor: "pointer",
		}}
	>
		+
	</button>
);

export interface InsertDraft {
	description: string;
	/** Positive magnitude, decimal string as typed. */
	magnitude: string;
	isExpense: boolean;
	/** Day of month (1..31); clamped by the caller. */
	day: number;
}

/** The inline editor row: description, value with expense/income toggle, day. */
export const InsertRowEditor = ({
	columnCount,
	defaultDay,
	contextLabel,
	onSubmit,
	onCancel,
}: {
	columnCount: number;
	defaultDay: number;
	/** "baseline" or "cenário {name}" — where this row will be written. */
	contextLabel: string;
	onSubmit: (draft: InsertDraft) => void;
	onCancel: () => void;
}) => {
	const [description, setDescription] = useState("");
	const [magnitude, setMagnitude] = useState("");
	const [isExpense, setIsExpense] = useState(true);
	const [day, setDay] = useState(String(defaultDay));

	const canSubmit = description.trim() !== "" && magnitude.trim() !== "";
	const submit = () => {
		if (!canSubmit) return;
		const parsedDay = Number(day);
		onSubmit({
			description: description.trim(),
			magnitude: magnitude.replace(/^-/, "").trim(),
			isExpense,
			day: Number.isFinite(parsedDay) && parsedDay >= 1 ? parsedDay : defaultDay,
		});
	};
	const onKeyDown = (e: React.KeyboardEvent) => {
		if (e.key === "Enter") submit();
		if (e.key === "Escape") onCancel();
	};

	return (
		<tr>
			<td
				colSpan={columnCount}
				style={{
					padding: "8px 10px",
					borderBottom: "1px solid var(--border)",
					background: "rgba(109,74,255,0.05)",
					boxShadow: "inset 0 2px 0 var(--purple)",
				}}
			>
				<div
					style={{
						display: "flex",
						gap: 8,
						flexWrap: "wrap",
						alignItems: "center",
					}}
				>
					<input
						autoFocus
						placeholder="descrição"
						value={description}
						onChange={(e) => setDescription(e.target.value)}
						onKeyDown={onKeyDown}
						className="mono"
						style={{ ...editorInputStyle, minWidth: 200 }}
					/>
					<span style={{ display: "inline-flex", gap: 4 }}>
						<button
							onClick={() => setIsExpense(true)}
							className="mono"
							aria-pressed={isExpense}
							style={toggleStyle(isExpense, "var(--rose)")}
						>
							despesa
						</button>
						<button
							onClick={() => setIsExpense(false)}
							className="mono"
							aria-pressed={!isExpense}
							style={toggleStyle(!isExpense, "var(--green)")}
						>
							receita
						</button>
					</span>
					<input
						inputMode="decimal"
						placeholder="0,00"
						value={magnitude}
						onChange={(e) => setMagnitude(e.target.value)}
						onKeyDown={onKeyDown}
						className="mono"
						style={{ ...editorInputStyle, width: 90 }}
					/>
					<label
						className="mono"
						style={{ fontSize: 11, color: "var(--muted)", display: "inline-flex", gap: 4, alignItems: "center" }}
					>
						dia
						<input
							inputMode="numeric"
							value={day}
							onChange={(e) => setDay(e.target.value)}
							onKeyDown={onKeyDown}
							className="mono"
							style={{ ...editorInputStyle, width: 44 }}
						/>
					</label>
					<button
						onClick={submit}
						disabled={!canSubmit}
						className="mono"
						style={{
							background: "var(--purple)",
							color: "#fff",
							border: "none",
							borderRadius: "var(--radius-sm)",
							padding: "6px 12px",
							cursor: "pointer",
							fontSize: 12,
							opacity: canSubmit ? 1 : 0.4,
						}}
					>
						salvar
					</button>
					<button
						onClick={onCancel}
						className="mono"
						style={{
							background: "transparent",
							color: "var(--muted)",
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-sm)",
							padding: "6px 12px",
							cursor: "pointer",
							fontSize: 12,
						}}
					>
						cancelar
					</button>
					<span className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
						grava em: {contextLabel}
					</span>
				</div>
			</td>
		</tr>
	);
};

const editorInputStyle: React.CSSProperties = {
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "6px 10px",
	fontSize: 12,
	background: "var(--card)",
};

const toggleStyle = (active: boolean, color: string): React.CSSProperties => ({
	background: active
		? `color-mix(in srgb, ${color} 14%, transparent)`
		: "transparent",
	color: active ? color : "var(--muted)",
	border: `1px solid ${active ? color : "var(--border)"}`,
	borderRadius: "var(--radius-full)",
	padding: "5px 12px",
	cursor: "pointer",
	fontSize: 12,
});
