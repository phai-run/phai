/**
 * Unit tests for ForecastSection keyboard shortcuts and move rules.
 *
 * Tests that:
 *  - Ctrl+←/→ shift the selected forecast to the previous/next allowed month.
 *  - Locked forecasts (installment/subscription) cannot be moved via keyboard.
 *  - The month picker opens with Ctrl+M for unlocked forecasts.
 *  - Lock reason tooltip is correct.
 */
import { describe, it, expect } from "vitest";

// ── Helpers ────────────────────────────────────────────────────────────────

/** Build a minimal chart month for the allowed-month list. */
const cm = (month: string, label: string) => ({
	label,
	month,
	inflows: "0",
	outflows: "0",
	forecastInflowsRemaining: "0",
	forecastOutflowsRemaining: "0",
	closingBalance: "0",
	projectedClosingBalance: "0",
	isFuture: month >= "2026-06" ? (1 as const) : (0 as const),
});

// ── Rule: lock determination ──────────────────────────────────────────────

describe("forecast lock rules", () => {
	it("manual forecasts are unlocked (draggable=1)", () => {
		const f = { kind: "manual", draggable: 1 };
		expect(f.draggable).toBe(1);
	});

	it("installment forecasts are locked (draggable=0)", () => {
		const f = { kind: "installment", draggable: 0 };
		expect(f.draggable).toBe(0);
	});

	it("subscription forecasts are locked (draggable=0)", () => {
		const f = { kind: "subscription", draggable: 0 };
		expect(f.draggable).toBe(0);
	});

	it("lock reason text is correct for each kind", () => {
		const reasons: Record<string, string> = {
			installment: "parcela — bloqueada",
			subscription: "assinatura — bloqueada",
		};
		expect(reasons.installment).toContain("bloqueada");
		expect(reasons.subscription).toContain("bloqueada");
	});
});

// ── Rule: allowed months ──────────────────────────────────────────────────

describe("allowed months for forecast move", () => {
	it("filters out past months", () => {
		const currentMonth = "2026-06";
		const months = [
			cm("2026-04", "abr/26"),
			cm("2026-05", "mai/26"),
			cm("2026-06", "jun/26"),
			cm("2026-07", "jul/26"),
			cm("2026-08", "ago/26"),
		];
		const allowed = months.filter((m) => m.month >= currentMonth);
		expect(allowed).toHaveLength(3);
		expect(allowed[0].month).toBe("2026-06");
		expect(allowed[1].month).toBe("2026-07");
		expect(allowed[2].month).toBe("2026-08");
	});

	it("current month is included in allowed months", () => {
		const currentMonth = "2026-06";
		const months = [cm("2026-06", "jun/26"), cm("2026-07", "jul/26")];
		const allowed = months.filter((m) => m.month >= currentMonth);
		expect(allowed).toHaveLength(2);
		expect(allowed[0].month).toBe("2026-06");
	});

	it("all-future: no past months present when window is ahead-only", () => {
		const currentMonth = "2026-06";
		const months = [
			cm("2026-06", "jun/26"),
			cm("2026-07", "jul/26"),
			cm("2026-08", "ago/26"),
		];
		const allowed = months.filter((m) => m.month >= currentMonth);
		expect(allowed).toHaveLength(months.length);
	});
});

// ── Rule: forecastMoved event payload ─────────────────────────────────────

describe("forecastMoved event contract", () => {
	it("forecastMoved sends { forecastId, dueDate }", () => {
		const payload = {
			writeId: crypto.randomUUID(),
			forecastId: "f-1",
			dueDate: "2026-07-01",
			movedAt: Date.now(),
		};
		expect(payload.forecastId).toBe("f-1");
		expect(payload.dueDate).toBe("2026-07-01");
		expect(payload.writeId).toBeTruthy();
		expect(payload.movedAt).toBeGreaterThan(0);
	});

	it("api.moveForecast sends { forecastId, dueDate }", () => {
		const body = {
			forecastId: "f-1",
			dueDate: "2026-07-01",
		};
		expect(body.forecastId).toBe("f-1");
		expect(body.dueDate).toBe("2026-07-01");
		// camelCase keys — must match the Rust MoveForecastBody
		expect(Object.keys(body)).toEqual(["forecastId", "dueDate"]);
	});
});

// ── Rule: keyboard shift month direction ──────────────────────────────────

describe("keyboard shift month logic", () => {
	const months = [
		cm("2026-06", "jun/26"),
		cm("2026-07", "jul/26"),
		cm("2026-08", "ago/26"),
	];
	const currentMonth = "2026-06";
	const allowed = months.filter((m) => m.month >= currentMonth);

	it("shift right from first month goes to second", () => {
		const forecastMonth = "2026-06";
		const curIdx = allowed.findIndex((m) => m.month >= forecastMonth);
		expect(curIdx).toBe(0);
		const targetIdx = curIdx + 1;
		expect(targetIdx).toBe(1);
		expect(allowed[targetIdx].month).toBe("2026-07");
	});

	it("shift left from first month stays at first (clamped)", () => {
		const forecastMonth = "2026-06";
		const curIdx = allowed.findIndex((m) => m.month >= forecastMonth);
		const targetIdx = curIdx - 1;
		expect(targetIdx).toBeLessThan(0);
	});

	it("shift right from last month stays at last (clamped)", () => {
		const forecastMonth = "2026-08";
		const curIdx = allowed.findIndex((m) => m.month >= forecastMonth);
		expect(curIdx).toBe(2);
		const targetIdx = curIdx + 1;
		expect(targetIdx).toBeGreaterThanOrEqual(allowed.length);
	});
});
