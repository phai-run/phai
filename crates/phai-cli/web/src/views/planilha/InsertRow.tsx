import { useState } from "react";
import { categoryEmoji } from "../../lib/categoryEmoji";
import { tdStyle } from "./sheetShared";

/**
 * Positional insert (design E): the round "+" that appears on the boundary of
 * a hovered row, and the inline editor row it opens. The editor's inputs line
 * up with the sheet's columns (descrição · categoria · dia · valor · ações) so
 * editing reads like the table it sits inside. The visual position is cosmetic
 * — the day decides where the row sorts.
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
	/** Positive magnitude, decimal string as typed (sign stripped). */
	magnitude: string;
	/** Derived from the leading "-" of the amount input. */
	isExpense: boolean;
	/** Day of month (1..daysInMonth); validated by the editor. */
	day: number;
	/** Chosen category id, or null. */
	categoryId: string | null;
}

/**
 * The inline editor row: one input per sheet column. The amount input carries
 * its own sign — a leading "-" means despesa, anything positive means entrada
 * (no separate toggle). The day is validated against the month's length.
 */
export const InsertRowEditor = ({
	defaultDay,
	maxDay,
	contextLabel,
	onSubmit,
	onCancel,
}: {
	defaultDay: number;
	/** Days in the sheet's month (e.g. 28 for Feb) — the day input's upper bound. */
	maxDay: number;
	/** "baseline" or "cenário {name}" — where this row will be written. */
	contextLabel: string;
	onSubmit: (draft: InsertDraft) => void;
	onCancel: () => void;
}) => {
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [category, setCategory] = useState("");
	const [day, setDay] = useState(String(defaultDay));

	const isExpense = amount.trim().startsWith("-");
	const magnitude = amount.replace(/^[+-]/, "").trim();
	const parsedDay = Number(day);
	const dayValid =
		Number.isInteger(parsedDay) && parsedDay >= 1 && parsedDay <= maxDay;
	const canSubmit =
		description.trim() !== "" && magnitude !== "" && dayValid;

	const submit = () => {
		if (!canSubmit) return;
		onSubmit({
			description: description.trim(),
			magnitude,
			isExpense,
			day: parsedDay,
			categoryId: category.trim() || null,
		});
	};
	const onKeyDown = (e: React.KeyboardEvent) => {
		if (e.key === "Enter") submit();
		if (e.key === "Escape") onCancel();
	};

	const cell: React.CSSProperties = {
		...tdStyle,
		background: "rgba(109,74,255,0.05)",
		verticalAlign: "middle",
	};

	return (
		<tr style={{ boxShadow: "inset 0 2px 0 var(--purple)" }}>
			{/* origin */}
			<td style={{ ...cell, textAlign: "center", color: "var(--purple)" }}>
				<span className="mono" style={{ fontSize: 13 }}>
					＋
				</span>
			</td>

			{/* descrição */}
			<td style={cell}>
				<input
					autoFocus
					placeholder="descrição"
					value={description}
					onChange={(e) => setDescription(e.target.value)}
					onKeyDown={onKeyDown}
					className="mono"
					style={{ ...editorInputStyle, width: "100%", minWidth: 140 }}
				/>
				<div
					className="mono"
					style={{ fontSize: 10.5, color: "var(--muted)", marginTop: 3 }}
				>
					grava em: {contextLabel} · use “-” para despesa
				</div>
			</td>

			{/* categoria */}
			<td style={cell}>
				<input
					list="sheet-forecast-categories"
					placeholder={category ? "" : "categoria"}
					value={category}
					onChange={(e) => setCategory(e.target.value)}
					onKeyDown={onKeyDown}
					className="mono"
					style={{ ...editorInputStyle, width: "100%", minWidth: 120 }}
				/>
				{category.trim() && (
					<span
						className="mono"
						aria-hidden
						style={{ fontSize: 11, color: "var(--muted)" }}
					>
						{categoryEmoji(category.trim(), !isExpense)} {category.trim()}
					</span>
				)}
			</td>

			{/* dia */}
			<td style={{ ...cell, textAlign: "center" }}>
				<input
					inputMode="numeric"
					aria-label="dia"
					aria-invalid={!dayValid}
					value={day}
					onChange={(e) => setDay(e.target.value.replace(/[^\d]/g, ""))}
					onKeyDown={onKeyDown}
					title={dayValid ? undefined : `dia inválido (1–${maxDay})`}
					className="mono"
					style={{
						...editorInputStyle,
						width: 44,
						textAlign: "center",
						borderColor: dayValid ? "var(--border)" : "var(--rose)",
					}}
				/>
			</td>

			{/* valor */}
			<td style={{ ...cell, textAlign: "right" }}>
				<input
					inputMode="decimal"
					placeholder="-0,00"
					value={amount}
					onChange={(e) => setAmount(e.target.value)}
					onKeyDown={onKeyDown}
					className="mono"
					style={{
						...editorInputStyle,
						width: 110,
						textAlign: "right",
						color: magnitude
							? isExpense
								? "var(--rose)"
								: "var(--green)"
							: undefined,
					}}
				/>
			</td>

			{/* ações */}
			<td style={{ ...cell, textAlign: "right", whiteSpace: "nowrap" }}>
				<button
					onClick={submit}
					disabled={!canSubmit}
					className="mono"
					title="salvar (Enter)"
					aria-label="salvar"
					style={{
						background: "var(--purple)",
						color: "#fff",
						border: "none",
						borderRadius: "var(--radius-sm)",
						padding: "5px 9px",
						cursor: canSubmit ? "pointer" : "not-allowed",
						fontSize: 13,
						opacity: canSubmit ? 1 : 0.4,
						marginRight: 4,
					}}
				>
					✓
				</button>
				<button
					onClick={onCancel}
					className="mono"
					title="cancelar (Esc)"
					aria-label="cancelar"
					style={{
						background: "transparent",
						color: "var(--muted)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-sm)",
						padding: "5px 9px",
						cursor: "pointer",
						fontSize: 13,
					}}
				>
					✕
				</button>
			</td>
		</tr>
	);
};

const editorInputStyle: React.CSSProperties = {
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "6px 8px",
	fontSize: 12.5,
	background: "var(--card)",
};
