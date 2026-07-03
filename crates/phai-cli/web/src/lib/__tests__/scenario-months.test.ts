/**
 * Tests for scenarioChangesByMonth — the pure derivation that turns a
 * scenario's typed deltas into per-month tooltip items (label + signed saldo
 * delta) for the chart hover card (ADR-0037).
 */
import { describe, expect, it } from "vitest";
import {
	addMonths,
	scenarioChangesByMonth,
	type ScenarioChangeLike,
	type SheetForecastLike,
} from "../derivations";

// ── Fixtures (synthetic) ────────────────────────────────────────────────────

const change = (
	overrides: Partial<ScenarioChangeLike> & { changeId: string; kind: string },
): ScenarioChangeLike => ({
	targetForecastId: null,
	targetTemplateId: null,
	month: null,
	effectiveFrom: null,
	amount: null,
	monthsCount: null,
	description: null,
	categoryId: null,
	accountId: null,
	...overrides,
});

const forecast = (
	overrides: Partial<SheetForecastLike> & { forecastId: string },
): SheetForecastLike => ({
	dueDate: "2026-08-10",
	description: "aluguel",
	amount: "-2000.00",
	categoryId: null,
	accountId: null,
	status: "ativo",
	kind: "fixed",
	templateId: null,
	...overrides,
});

// ── addMonths ───────────────────────────────────────────────────────────────

describe("addMonths", () => {
	it("adds within the year", () => {
		expect(addMonths("2026-07", 2)).toBe("2026-09");
	});

	it("rolls over the year boundary", () => {
		expect(addMonths("2026-11", 3)).toBe("2027-02");
	});

	it("zero is identity", () => {
		expect(addMonths("2026-07", 0)).toBe("2026-07");
	});
});

// ── scenarioChangesByMonth ──────────────────────────────────────────────────

describe("scenarioChangesByMonth", () => {
	it("places an add_one_shot on its month with the signed amount", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-1",
					kind: "add_one_shot",
					month: "2026-09",
					amount: "-1500.00",
					description: "reforma banheiro",
				}),
			],
			[],
		);
		expect(map.get("2026-09")).toEqual([
			{ changeId: "chg-1", label: "reforma banheiro", delta: -1500 },
		]);
		expect(map.size).toBe(1);
	});

	it("spreads a hypothetical_installment across its months with n/N labels", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-2",
					kind: "hypothetical_installment",
					effectiveFrom: "2026-11",
					monthsCount: 3,
					amount: "-500.00",
					description: "notebook",
				}),
			],
			[],
		);
		expect(map.get("2026-11")).toEqual([
			{ changeId: "chg-2", label: "notebook 1/3", delta: -500 },
		]);
		expect(map.get("2026-12")).toEqual([
			{ changeId: "chg-2", label: "notebook 2/3", delta: -500 },
		]);
		expect(map.get("2027-01")).toEqual([
			{ changeId: "chg-2", label: "notebook 3/3", delta: -500 },
		]);
		expect(map.has("2027-02")).toBe(false);
	});

	it("adjust_amount lands on the target forecast's month with delta = new − old", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-3",
					kind: "adjust_amount",
					targetForecastId: "fc-1",
					amount: "-2500.00",
				}),
			],
			[forecast({ forecastId: "fc-1", dueDate: "2026-08-10" })],
		);
		// -2500 - (-2000) = -500 (the scenario makes the month R$500 worse)
		expect(map.get("2026-08")).toEqual([
			{ changeId: "chg-3", label: "aluguel", delta: -500 },
		]);
	});

	it("skip_forecast frees the forecast's amount (delta = −amount)", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-4",
					kind: "skip_forecast",
					targetForecastId: "fc-1",
				}),
			],
			[forecast({ forecastId: "fc-1", dueDate: "2026-10-05" })],
		);
		// Skipping a -2000 expense frees +2000.
		expect(map.get("2026-10")).toEqual([
			{ changeId: "chg-4", label: "aluguel", delta: 2000 },
		]);
	});

	it("end_template frees every active template forecast from effectiveFrom on", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-5",
					kind: "end_template",
					targetTemplateId: "tpl-1",
					effectiveFrom: "2026-09",
				}),
			],
			[
				forecast({
					forecastId: "fc-a",
					templateId: "tpl-1",
					dueDate: "2026-08-15",
					description: "academia",
					amount: "-120.00",
				}),
				forecast({
					forecastId: "fc-b",
					templateId: "tpl-1",
					dueDate: "2026-09-15",
					description: "academia",
					amount: "-120.00",
				}),
				forecast({
					forecastId: "fc-c",
					templateId: "tpl-1",
					dueDate: "2026-10-15",
					description: "academia",
					amount: "-120.00",
					status: "descartado", // inactive — must be ignored
				}),
				forecast({
					forecastId: "fc-d",
					templateId: "tpl-2", // other template — must be ignored
					dueDate: "2026-09-15",
				}),
			],
		);
		expect(map.has("2026-08")).toBe(false); // before the cutoff
		expect(map.get("2026-09")).toEqual([
			{ changeId: "chg-5", label: "academia", delta: 120 },
		]);
		expect(map.has("2026-10")).toBe(false); // discarded forecast ignored
	});

	it("ignores changes whose target forecast is missing", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-6",
					kind: "adjust_amount",
					targetForecastId: "fc-gone",
					amount: "-100.00",
				}),
				change({
					changeId: "chg-7",
					kind: "skip_forecast",
					targetForecastId: "fc-gone",
				}),
			],
			[],
		);
		expect(map.size).toBe(0);
	});

	it("sorts a month's items by absolute delta descending", () => {
		const map = scenarioChangesByMonth(
			[
				change({
					changeId: "chg-small",
					kind: "add_one_shot",
					month: "2026-09",
					amount: "-100.00",
					description: "pequeno",
				}),
				change({
					changeId: "chg-big",
					kind: "add_one_shot",
					month: "2026-09",
					amount: "3000.00",
					description: "bônus",
				}),
			],
			[],
		);
		expect(map.get("2026-09")?.map((i) => i.changeId)).toEqual([
			"chg-big",
			"chg-small",
		]);
	});
});
