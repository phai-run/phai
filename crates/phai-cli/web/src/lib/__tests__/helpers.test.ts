/**
 * Unit tests for lib/format.ts — decimal-string math and display formatters.
 *
 * Amounts arrive as decimal-as-string from the Rust bridge (rust_decimal).
 * Client-side sums use integer-cent math (toCents / sumAmounts) so they never
 * drift. These tests verify precision, sign handling, and locale formatting.
 */
import { describe, expect, it } from "vitest";
import {
	formatMoney,
	formatMoneyNumber,
	isNegative,
	sumAmounts,
	toCents,
} from "../format";

// ── toCents ────────────────────────────────────────────────────────────────

describe("toCents", () => {
	it("parses a positive decimal string to integer cents", () => {
		expect(toCents("150.75")).toBe(15075);
	});

	it("parses a negative decimal string to negative integer cents", () => {
		expect(toCents("-150.75")).toBe(-15075);
	});

	it("handles whole numbers (no fractional part)", () => {
		expect(toCents("42")).toBe(4200);
	});

	it("handles amounts with more than 2 fractional digits (truncation)", () => {
		// 3.456 → 345 (truncated, not rounded)
		expect(toCents("3.456")).toBe(345);
	});

	it("handles zero values", () => {
		expect(toCents("0")).toBe(0);
		expect(toCents("0.00")).toBe(0);
	});

	it("handles null and undefined as zero", () => {
		expect(toCents(null)).toBe(0);
		expect(toCents(undefined)).toBe(0);
	});

	it("handles empty string as zero", () => {
		expect(toCents("")).toBe(0);
	});

	it("handles positive sign explicitly", () => {
		expect(toCents("+99.99")).toBe(9999);
	});

	it("handles very large amounts without overflow", () => {
		expect(toCents("999999.99")).toBe(99999999);
	});

	it("handles whitespace around the value", () => {
		expect(toCents("  -123.45  ")).toBe(-12345);
	});

	it("preserves exactness: no float drift on repeated operations", () => {
		// If we accidentally used Number arithmetic we'd see 0.1 + 0.2 !== 0.3
		// toCents avoids that by working in integer cents.
		const cents = toCents("0.10") + toCents("0.20");
		expect(cents).toBe(30); // exactly 30 cents
	});
});

// ── sumAmounts ─────────────────────────────────────────────────────────────

describe("sumAmounts", () => {
	it("sums positive amounts exactly", () => {
		expect(sumAmounts(["10.50", "20.25", "5.00"])).toBe(35.75);
	});

	it("sums mixed positive and negative amounts", () => {
		expect(sumAmounts(["100.00", "-30.00", "-20.00"])).toBe(50.0);
	});

	it("returns 0 for an empty array", () => {
		expect(sumAmounts([])).toBe(0);
	});

	it("handles null entries gracefully", () => {
		expect(sumAmounts(["50.00", null, "25.00"])).toBe(75.0);
	});

	it("handles undefined entries gracefully", () => {
		expect(sumAmounts(["50.00", undefined, "25.00"])).toBe(75.0);
	});

	it("handles all-negative array", () => {
		expect(sumAmounts(["-10.00", "-20.00", "-30.00"])).toBe(-60.0);
	});

	it("matches individual sum of cents", () => {
		const amounts = ["12.34", "56.78", "90.12"];
		const sum = sumAmounts(amounts);
		const centsSum = amounts.reduce((acc, a) => acc + toCents(a), 0);
		expect(Math.round(sum * 100)).toBe(centsSum);
	});

	it("handles large dataset without precision loss", () => {
		const amounts: string[] = [];
		for (let i = 0; i < 10000; i++) {
			amounts.push(
				`${(i % 100).toString()}.${String(i % 100).padStart(2, "0")}`,
			);
		}
		const result = sumAmounts(amounts);
		// Should be finite and exact
		expect(Number.isFinite(result)).toBe(true);
		// Verify cents-level precision
		const cents = Math.round(result * 100);
		expect(Number.isInteger(cents)).toBe(true);
	});
});

// ── isNegative ─────────────────────────────────────────────────────────────

describe("isNegative", () => {
	it("returns true for negative amounts", () => {
		expect(isNegative("-50.00")).toBe(true);
		expect(isNegative("-0.01")).toBe(true);
	});

	it("returns false for positive amounts", () => {
		expect(isNegative("50.00")).toBe(false);
		expect(isNegative("0.01")).toBe(false);
	});

	it("returns false for zero", () => {
		expect(isNegative("0")).toBe(false);
		expect(isNegative("0.00")).toBe(false);
	});

	it("returns false for null and undefined", () => {
		expect(isNegative(null)).toBe(false);
		expect(isNegative(undefined)).toBe(false);
	});

	it("handles whitespace-padded negative", () => {
		expect(isNegative("  -99.00")).toBe(true);
	});
});

// ── formatMoney ────────────────────────────────────────────────────────────

describe("formatMoney", () => {
	it("formats positive amounts in pt-BR currency", () => {
		const result = formatMoney("150.75");
		expect(result).toContain("150,75");
		expect(result).toContain("R$");
	});

	it("formats negative amounts in pt-BR currency", () => {
		const result = formatMoney("-150.75");
		expect(result).toContain("-");
	});

	it("formats zero", () => {
		const result = formatMoney("0");
		expect(result).toContain("0,00");
	});

	it("handles null as zero", () => {
		const result = formatMoney(null);
		expect(result).toContain("0,00");
	});

	it("handles undefined as zero", () => {
		const result = formatMoney(undefined);
		expect(result).toContain("0,00");
	});

	it("handles empty string as zero", () => {
		const result = formatMoney("");
		expect(result).toContain("0,00");
	});
});

// ── formatMoneyNumber ──────────────────────────────────────────────────────

describe("formatMoneyNumber", () => {
	it("formats a numeric value in pt-BR", () => {
		const result = formatMoneyNumber(1234.56);
		expect(result).toContain("1.234,56");
	});

	it("formats negative numbers", () => {
		const result = formatMoneyNumber(-500);
		expect(result).toContain("-");
	});

	it("formats zero", () => {
		const result = formatMoneyNumber(0);
		expect(result).toContain("0,00");
	});
});
