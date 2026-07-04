import { useState } from "react";
import { categoryEmoji } from "../../lib/categoryEmoji";

/**
 * Positional insert (design E): the round "+" that appears on the boundary of a
 * hovered row, and the inline editor it opens. The editor is a single full-width
 * band (one `colSpan` cell) rendered as a soft rounded mini-form — not a set of
 * per-column inputs — so creating a planned row reads as a deliberate little
 * form instead of a raw table row with a hard accent line. The day decides where
 * the row ultimately sorts; the visual position is cosmetic.
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
 * The inline editor, laid out as a single band spanning every column. The amount
 * input carries its own sign — a leading "-" means despesa, anything positive an
 * entrada (a segmented toggle mirrors and controls that sign explicitly). The
 * day is validated against the month's length.
 */
export const InsertRowEditor = ({
	defaultDay,
	maxDay,
	contextLabel,
	colSpan,
	accent = "var(--purple)",
	onSubmit,
	onCancel,
}: {
	defaultDay: number;
	/** Days in the sheet's month (e.g. 28 for Feb) — the day input's upper bound. */
	maxDay: number;
	/** "baseline" or "cenário {name}" — where this row will be written. */
	contextLabel: string;
	/** Columns to span so the band fills the whole table width. */
	colSpan: number;
	/** The month's accent (ties the editor to its sheet); defaults to brand purple. */
	accent?: string;
	onSubmit: (draft: InsertDraft) => void;
	onCancel: () => void;
}) => {
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [category, setCategory] = useState("");
	const [day, setDay] = useState(String(defaultDay));
	// Explicit despesa/entrada toggle; the amount sign stays in sync so typing a
	// leading "-" and clicking the toggle mean the same thing.
	const [expense, setExpense] = useState(true);

	const isExpense = amount.trim().startsWith("-") || (!amount.trim().startsWith("+") && expense);
	const magnitude = amount.replace(/^[+-]/, "").trim();
	const parsedDay = Number(day);
	const dayValid =
		Number.isInteger(parsedDay) && parsedDay >= 1 && parsedDay <= maxDay;
	const canSubmit = description.trim() !== "" && magnitude !== "" && dayValid;

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

	const amountColor = magnitude
		? isExpense
			? "var(--rose)"
			: "var(--green)"
		: "var(--muted)";

	return (
		<tr>
			<td colSpan={colSpan} style={{ padding: "6px 4px" }}>
				<div
					role="form"
					aria-label="nova linha da planilha"
					style={{
						display: "flex",
						flexWrap: "wrap",
						alignItems: "center",
						gap: 10,
						padding: "12px 14px",
						borderRadius: "var(--radius-md)",
						border: "1px solid var(--border)",
						borderLeft: `3px solid ${accent}`,
						background: "var(--surface)",
						boxShadow: "0 4px 16px rgba(21,19,31,0.06)",
					}}
				>
					{/* leading label chip */}
					<span
						className="mono"
						style={{
							fontSize: 10.5,
							fontWeight: 600,
							letterSpacing: "0.08em",
							textTransform: "uppercase",
							color: accent,
							border: `1px solid ${accent}`,
							borderRadius: "var(--radius-full)",
							padding: "2px 9px",
							whiteSpace: "nowrap",
						}}
					>
						＋ nova linha
					</span>

					{/* despesa / entrada segmented toggle */}
					<div
						role="radiogroup"
						aria-label="tipo"
						style={{
							display: "inline-flex",
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-full)",
							overflow: "hidden",
							flexShrink: 0,
						}}
					>
						{[
							{ key: true, label: "despesa", on: "var(--rose)" },
							{ key: false, label: "entrada", on: "var(--green)" },
						].map((opt) => {
							const active = isExpense === opt.key;
							return (
								<button
									key={String(opt.key)}
									type="button"
									role="radio"
									aria-checked={active}
									onClick={() => {
										setExpense(opt.key);
										// Re-sign the amount so the toggle and the text agree.
										setAmount(magnitude ? (opt.key ? `-${magnitude}` : magnitude) : "");
									}}
									className="mono"
									style={{
										border: "none",
										padding: "5px 12px",
										fontSize: 11.5,
										cursor: "pointer",
										background: active ? opt.on : "transparent",
										color: active ? "#fff" : "var(--muted)",
									}}
								>
									{opt.label}
								</button>
							);
						})}
					</div>

					{/* descrição — grows to fill */}
					<input
						autoFocus
						placeholder="descrição"
						value={description}
						onChange={(e) => setDescription(e.target.value)}
						onKeyDown={onKeyDown}
						className="mono sheet-insert-input"
						style={{ ...inputStyle, flex: "1 1 180px", minWidth: 160 }}
					/>

					{/* categoria */}
					<div style={{ display: "flex", alignItems: "center", gap: 6, flexShrink: 0 }}>
						<input
							list="sheet-forecast-categories"
							placeholder="categoria"
							value={category}
							onChange={(e) => setCategory(e.target.value)}
							onKeyDown={onKeyDown}
							className="mono sheet-insert-input"
							style={{ ...inputStyle, width: 150 }}
						/>
						{category.trim() && (
							<span aria-hidden style={{ fontSize: 15 }}>
								{categoryEmoji(category.trim(), !isExpense)}
							</span>
						)}
					</div>

					{/* dia */}
					<label
						className="mono"
						style={{ display: "flex", alignItems: "center", gap: 5, fontSize: 11, color: "var(--muted)", flexShrink: 0 }}
					>
						dia
						<input
							inputMode="numeric"
							aria-label="dia"
							aria-invalid={!dayValid}
							value={day}
							onChange={(e) => setDay(e.target.value.replace(/[^\d]/g, ""))}
							onKeyDown={onKeyDown}
							title={dayValid ? undefined : `dia inválido (1–${maxDay})`}
							className="mono sheet-insert-input"
							style={{
								...inputStyle,
								width: 46,
								textAlign: "center",
								borderColor: dayValid ? "var(--border)" : "var(--rose)",
							}}
						/>
					</label>

					{/* valor */}
					<input
						inputMode="decimal"
						aria-label="valor"
						placeholder={isExpense ? "-0,00" : "0,00"}
						value={amount}
						onChange={(e) => setAmount(e.target.value)}
						onKeyDown={onKeyDown}
						className="mono sheet-insert-input"
						style={{
							...inputStyle,
							width: 120,
							textAlign: "right",
							fontWeight: 600,
							color: amountColor,
							flexShrink: 0,
						}}
					/>

					{/* actions */}
					<div style={{ display: "flex", gap: 6, flexShrink: 0, marginLeft: "auto" }}>
						<button
							onClick={submit}
							disabled={!canSubmit}
							className="mono pressable"
							title="salvar (Enter)"
							style={{
								background: accent,
								color: "#fff",
								border: "none",
								borderRadius: "var(--radius-sm)",
								padding: "7px 16px",
								cursor: canSubmit ? "pointer" : "not-allowed",
								fontSize: 12.5,
								fontWeight: 600,
								opacity: canSubmit ? 1 : 0.4,
							}}
						>
							✓ salvar
						</button>
						<button
							onClick={onCancel}
							className="mono"
							title="cancelar (Esc)"
							style={{
								background: "transparent",
								color: "var(--muted)",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
								padding: "7px 12px",
								cursor: "pointer",
								fontSize: 12.5,
							}}
						>
							cancelar
						</button>
					</div>

					{/* context hint — full-width footnote */}
					<div
						className="mono"
						style={{
							flexBasis: "100%",
							fontSize: 10.5,
							color: "var(--muted)",
						}}
					>
						grava em <strong style={{ color: "var(--text)" }}>{contextLabel}</strong>
						{" · "}Enter salva · Esc cancela
					</div>
				</div>
			</td>
		</tr>
	);
};

const inputStyle: React.CSSProperties = {
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "7px 10px",
	fontSize: 13,
	background: "var(--card)",
};
