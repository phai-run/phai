import { describe, expect, it } from "vitest";
import { monthTheme } from "../monthTheme";

describe("monthTheme", () => {
	it("maps each month to a stable, distinct accent", () => {
		const accents = new Set<string>();
		for (let m = 1; m <= 12; m++) {
			const key = `2026-${String(m).padStart(2, "0")}`;
			const t = monthTheme(key);
			expect(t.month).toBe(m);
			expect(t.accent).toMatch(/^#[0-9a-f]{6}$/i);
			expect(t.glyph.length).toBeGreaterThan(0);
			accents.add(t.accent);
		}
		expect(accents.size).toBe(12); // no two months share a hue
	});

	it("is deterministic across calls", () => {
		expect(monthTheme("2026-07")).toEqual(monthTheme("2026-07"));
	});

	it("derives a translucent tint from the accent", () => {
		expect(monthTheme("2026-07").tint).toMatch(/^rgba\(\d+, \d+, \d+, [\d.]+\)$/);
	});

	it("falls back to January for a malformed month", () => {
		expect(monthTheme("garbage").month).toBe(1);
	});
});
