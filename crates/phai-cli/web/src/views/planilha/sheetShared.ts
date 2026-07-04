/**
 * Shared, view-free helpers of the unified sheet: CSV export, amount labels
 * and the cell styles both row kinds (real transaction / planned) render with.
 */
import type React from "react";
import { formatMoneyNumber, toCents } from "../../lib/format";
import { sheetLabel, type TxView } from "../../lib/derivations";

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

export const downloadCsv = (filename: string, csv: string) => {
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

// ── Cell styles shared by both row kinds ────────────────────────────────────

export const tdStyle: React.CSSProperties = {
	padding: "6px 10px",
	verticalAlign: "top",
	// Row separator lives on the td: tr borders don't render with
	// border-collapse: separate (which the sticky header requires).
	borderBottom: "1px solid var(--border)",
};

export const thStyle: React.CSSProperties = {
	padding: "8px 10px",
	textAlign: "left",
	fontWeight: 500,
	// Sticky lives on each th, not the tr: with collapsed table borders some
	// engines skip painting a sticky row's background, so body rows scrolled
	// through it. The th needs its own opaque background. Offset by the sticky
	// app header (56px) so column headers pin just below it, not under it.
	position: "sticky",
	top: 56,
	zIndex: 2,
	background: "var(--card)",
	boxShadow: "0 1px 0 var(--border)",
};

/** Category chip shared by transaction and planned rows. */
export const categoryChipStyle = (hasCategory: boolean): React.CSSProperties => ({
	background: hasCategory ? "var(--chip, #f1eefc)" : "transparent",
	color: hasCategory ? "var(--purple)" : "var(--amber)",
	border: hasCategory ? "1px solid transparent" : "1px dashed var(--amber)",
	borderRadius: "var(--radius-full)",
	padding: "3px 10px",
	fontSize: 12,
	maxWidth: 200,
	overflow: "hidden",
	textOverflow: "ellipsis",
	whiteSpace: "nowrap",
});

/** Small round icon button for the hover action cell. */
export const rowActionBtnStyle: React.CSSProperties = {
	background: "transparent",
	color: "var(--muted)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "2px 8px",
	cursor: "pointer",
	fontSize: 11,
	lineHeight: "16px",
};
