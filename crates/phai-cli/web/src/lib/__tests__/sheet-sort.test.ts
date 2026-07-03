/**
 * Unit tests for the planilha column sorting + row labels.
 *
 * All data is synthetic (AGENTS.md §1). Amounts are decimal strings; sums use
 * integer-cent math so assertions are exact.
 */
import { describe, expect, it } from "vitest";
import {
	sheetLabel,
	sortForSheet,
	type AccountInfo,
	type TxView,
} from "../derivations";

const tx = (over: Partial<TxView> & { id: string }): TxView => ({
	accountId: "acc-1",
	postedAt: "2026-06-05",
	amount: "-100.00",
	rawDescription: "RAW DESC",
	description: null,
	merchantName: null,
	purpose: null,
	categoryId: null,
	month: "2026-06",
	paymentStatus: "posted",
	installmentMarker: null,
	reviewed: 0,
	isInstallment: 0,
	isSubscription: 0,
	...over,
});

const noOverlay = new Map();
const noAccounts = new Map<string, AccountInfo>();

// ── sortForSheet ────────────────────────────────────────────────────────────

describe("sortForSheet", () => {
	const rows = [
		tx({ id: "a", amount: "-50.00", postedAt: "2026-06-03", categoryId: "b-cat" }),
		tx({ id: "b", amount: "-150.00", postedAt: "2026-06-01", categoryId: "a-cat" }),
		tx({ id: "c", amount: "75.00", postedAt: "2026-06-02", categoryId: null }),
	];

	it("sorts by amount ascending (most negative first)", () => {
		const sorted = sortForSheet(rows, { key: "amount", dir: 1 }, noOverlay, noAccounts);
		expect(sorted.map((t) => t.id)).toEqual(["b", "a", "c"]);
	});

	it("sorts by date descending", () => {
		const sorted = sortForSheet(rows, { key: "date", dir: -1 }, noOverlay, noAccounts);
		expect(sorted.map((t) => t.id)).toEqual(["a", "c", "b"]);
	});

	it("sorts by category using the overlay-effective value", () => {
		const overlay = new Map([
			[
				"c",
				{
					transactionId: "c",
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: "zz-cat",
				},
			],
		]);
		const sorted = sortForSheet(rows, { key: "category", dir: 1 }, overlay, noAccounts);
		expect(sorted.map((t) => t.id)).toEqual(["b", "a", "c"]);
	});

	it("does not mutate the input", () => {
		const before = rows.map((t) => t.id);
		sortForSheet(rows, { key: "amount", dir: 1 }, noOverlay, noAccounts);
		expect(rows.map((t) => t.id)).toEqual(before);
	});
});

describe("sheetLabel", () => {
	it("prefers human description, then merchant, then raw", () => {
		expect(sheetLabel(tx({ id: "x", description: "Almoço" }))).toBe("Almoço");
		expect(sheetLabel(tx({ id: "y", merchantName: "Bistrô" }))).toBe("Bistrô");
		expect(sheetLabel(tx({ id: "z" }))).toBe("RAW DESC");
	});
});
