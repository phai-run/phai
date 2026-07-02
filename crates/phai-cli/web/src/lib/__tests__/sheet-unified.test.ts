/**
 * Unified sheet derivations (design B/F + write routing):
 *  - applyScenarioToMonthRows — the client-side scenario overlay (ADR-0037)
 *  - sortUnifiedRows + localStorage persistence of sort/filters
 *  - routeSheetDelete / routeSheetAmountEdit / routeSheetAdd — the pure
 *    baseline-vs-scenario decision the view commits events from
 */
import { describe, expect, it } from "vitest";
import {
	applyScenarioToMonthRows,
	DEFAULT_SHEET_LOCAL_FILTERS,
	forecastSheetOrigin,
	matchesSheetLocalFilters,
	monthDiff,
	readSheetLocalFilters,
	readSheetSort,
	routeSheetAdd,
	routeSheetAmountEdit,
	routeSheetDelete,
	SHEET_FILTERS_STORAGE_KEY,
	SHEET_SORT_STORAGE_KEY,
	sortUnifiedRows,
	writeSheetLocalFilters,
	writeSheetSort,
	type PlannedSheetRow,
	type ScenarioChangeLike,
	type SheetForecastLike,
	type SheetRowRef,
} from "../derivations";

// ── Fixtures (synthetic only) ───────────────────────────────────────────────

const forecast = (
	overrides: Partial<SheetForecastLike> = {},
): SheetForecastLike => ({
	forecastId: "f1",
	dueDate: "2026-08-10",
	description: "streaming",
	amount: "-49.90",
	categoryId: "lazer",
	accountId: null,
	status: "ativo",
	kind: "manual",
	templateId: null,
	...overrides,
});

const change = (
	overrides: Partial<ScenarioChangeLike> = {},
): ScenarioChangeLike => ({
	changeId: "chg-1",
	kind: "add_one_shot",
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

// ── applyScenarioToMonthRows (design B) ─────────────────────────────────────

describe("applyScenarioToMonthRows", () => {
	it("with no changes returns the month's active forecasts as planned rows", () => {
		const rows = applyScenarioToMonthRows(
			[
				forecast(),
				forecast({ forecastId: "f2", dueDate: "2026-09-01" }), // other month
				forecast({ forecastId: "f3", status: "realizado" }), // realized → out
				forecast({ forecastId: "f4", status: "descartado" }), // discarded → out
			],
			[],
			"2026-08",
		);
		expect(rows.map((r) => r.id)).toEqual(["f1"]);
		expect(rows[0].skipped).toBe(false);
		expect(rows[0].origin).toBe("manual");
	});

	it("add_one_shot of the month becomes a scenario row; other months don't", () => {
		const rows = applyScenarioToMonthRows(
			[],
			[
				change({ month: "2026-08", amount: "-2000.00", description: "viagem" }),
				change({ changeId: "chg-2", month: "2026-09", amount: "-1.00" }),
			],
			"2026-08",
		);
		expect(rows).toHaveLength(1);
		expect(rows[0]).toMatchObject({
			id: "chg-1",
			origin: "scenario",
			description: "viagem",
			amount: "-2000.00",
			changeId: "chg-1",
			forecastId: null,
		});
	});

	it("hypothetical_installment yields n/N rows inside the window only", () => {
		const inst = change({
			kind: "hypothetical_installment",
			effectiveFrom: "2026-08",
			monthsCount: 3,
			amount: "-500.00",
			description: "carro",
		});
		const at = (month: string) =>
			applyScenarioToMonthRows([], [inst], month);
		expect(at("2026-07")).toHaveLength(0); // before the window
		expect(at("2026-08")[0].installmentLabel).toBe("1/3");
		expect(at("2026-10")[0].installmentLabel).toBe("3/3");
		expect(at("2026-11")).toHaveLength(0); // past the last installment
	});

	it("adjust_amount replaces the amount and keeps originalAmount", () => {
		const rows = applyScenarioToMonthRows(
			[forecast()],
			[
				change({
					kind: "adjust_amount",
					targetForecastId: "f1",
					amount: "-25.00",
				}),
			],
			"2026-08",
		);
		expect(rows[0].amount).toBe("-25.00");
		expect(rows[0].originalAmount).toBe("-49.90");
		expect(rows[0].adjustChangeId).toBe("chg-1");
		expect(rows[0].skipped).toBe(false);
	});

	it("skip_forecast marks the row skipped (kept for the strikethrough)", () => {
		const rows = applyScenarioToMonthRows(
			[forecast()],
			[change({ kind: "skip_forecast", targetForecastId: "f1" })],
			"2026-08",
		);
		expect(rows).toHaveLength(1);
		expect(rows[0].skipped).toBe(true);
		expect(rows[0].skipChangeId).toBe("chg-1");
	});

	it("end_template skips template rows from effectiveFrom on, not before", () => {
		const tpl = forecast({
			forecastId: "f-aug",
			templateId: "tpl-1",
			kind: "template",
		});
		const end = change({
			kind: "end_template",
			targetTemplateId: "tpl-1",
			effectiveFrom: "2026-08",
		});
		expect(applyScenarioToMonthRows([tpl], [end], "2026-08")[0].skipped).toBe(
			true,
		);
		// A July row of the same template is untouched (cutoff is later).
		const july = forecast({
			forecastId: "f-jul",
			dueDate: "2026-07-10",
			templateId: "tpl-1",
			kind: "template",
		});
		expect(
			applyScenarioToMonthRows(
				[july],
				[change({ kind: "end_template", targetTemplateId: "tpl-1", effectiveFrom: "2026-08" })],
				"2026-07",
			)[0].skipped,
		).toBe(false);
	});
});

describe("forecastSheetOrigin / monthDiff", () => {
	it("classifies forecast kinds onto the origin axis", () => {
		expect(forecastSheetOrigin({ kind: "installment", templateId: "t" })).toBe(
			"installment",
		);
		expect(forecastSheetOrigin({ kind: "fixed", templateId: "t" })).toBe("fixed");
		expect(forecastSheetOrigin({ kind: "manual", templateId: null })).toBe(
			"manual",
		);
		expect(forecastSheetOrigin({ kind: "manual", templateId: "t" })).toBe(
			"recurring",
		);
		expect(forecastSheetOrigin({ kind: "subscription", templateId: null })).toBe(
			"recurring",
		);
	});

	it("monthDiff counts calendar months across year boundaries", () => {
		expect(monthDiff("2026-08", "2026-08")).toBe(0);
		expect(monthDiff("2026-11", "2027-02")).toBe(3);
		expect(monthDiff("2026-08", "2026-07")).toBe(-1);
		expect(Number.isNaN(monthDiff("garbage", "2026-08"))).toBe(true);
	});
});

// ── Unified sorting + persistence (design F) ────────────────────────────────

const row = (
	overrides: Partial<{
		id: string;
		date: string;
		description: string;
		account: string;
		category: string | null;
		amount: string;
		origin: string;
	}> = {},
) => ({
	id: "r1",
	date: "2026-08-10",
	description: "mercado",
	account: "banco",
	category: "alimentacao",
	amount: "-50.00",
	origin: "real",
	...overrides,
});

describe("sortUnifiedRows", () => {
	it("sorts real and planned rows together by date", () => {
		const rows = [
			row({ id: "a", date: "2026-08-20" }),
			row({ id: "b", date: "2026-08-05", origin: "manual" }),
			row({ id: "c", date: "2026-08-12", origin: "scenario" }),
		];
		expect(
			sortUnifiedRows(rows, { key: "date", dir: 1 }).map((r) => r.id),
		).toEqual(["b", "c", "a"]);
		expect(
			sortUnifiedRows(rows, { key: "date", dir: -1 }).map((r) => r.id),
		).toEqual(["a", "c", "b"]);
	});

	it("sorts by amount, origin and flow", () => {
		const rows = [
			row({ id: "exp", amount: "-100.00", origin: "scenario" }),
			row({ id: "inc", amount: "250.00", origin: "real" }),
			row({ id: "small", amount: "-10.00", origin: "manual" }),
		];
		expect(
			sortUnifiedRows(rows, { key: "amount", dir: 1 }).map((r) => r.id),
		).toEqual(["exp", "small", "inc"]);
		expect(
			sortUnifiedRows(rows, { key: "origin", dir: 1 }).map((r) => r.origin),
		).toEqual(["real", "manual", "scenario"]);
		// flow ascending = income first.
		expect(
			sortUnifiedRows(rows, { key: "flow", dir: 1 })[0].id,
		).toBe("inc");
	});

	it("is stable for equal keys (date desc, then id)", () => {
		const rows = [
			row({ id: "b", amount: "-10.00", date: "2026-08-01" }),
			row({ id: "a", amount: "-10.00", date: "2026-08-01" }),
		];
		expect(
			sortUnifiedRows(rows, { key: "amount", dir: 1 }).map((r) => r.id),
		).toEqual(["a", "b"]);
	});
});

/** Minimal in-memory Storage mock. */
const mockStorage = () => {
	const map = new Map<string, string>();
	return {
		getItem: (k: string) => map.get(k) ?? null,
		setItem: (k: string, v: string) => void map.set(k, v),
		dump: () => map,
	};
};

describe("sheet sort persistence (localStorage)", () => {
	it("round-trips {col,dir} through storage", () => {
		const storage = mockStorage();
		writeSheetSort(storage, { key: "amount", dir: 1 });
		expect(JSON.parse(storage.dump().get(SHEET_SORT_STORAGE_KEY)!)).toEqual({
			col: "amount",
			dir: 1,
		});
		expect(readSheetSort(storage)).toEqual({ key: "amount", dir: 1 });
	});

	it("rejects corrupt or unknown payloads", () => {
		const storage = mockStorage();
		expect(readSheetSort(storage)).toBeNull();
		storage.setItem(SHEET_SORT_STORAGE_KEY, "not json");
		expect(readSheetSort(storage)).toBeNull();
		storage.setItem(SHEET_SORT_STORAGE_KEY, JSON.stringify({ col: "nope", dir: 1 }));
		expect(readSheetSort(storage)).toBeNull();
		storage.setItem(SHEET_SORT_STORAGE_KEY, JSON.stringify({ col: "date", dir: 2 }));
		expect(readSheetSort(storage)).toBeNull();
	});
});

describe("sheet local filters persistence + matching", () => {
	it("round-trips origin/flow and falls back to defaults", () => {
		const storage = mockStorage();
		expect(readSheetLocalFilters(storage)).toEqual(DEFAULT_SHEET_LOCAL_FILTERS);
		writeSheetLocalFilters(storage, { origin: "scenario", flow: "out" });
		expect(JSON.parse(storage.dump().get(SHEET_FILTERS_STORAGE_KEY)!)).toEqual({
			origin: "scenario",
			flow: "out",
		});
		expect(readSheetLocalFilters(storage)).toEqual({
			origin: "scenario",
			flow: "out",
		});
		storage.setItem(SHEET_FILTERS_STORAGE_KEY, JSON.stringify({ origin: "x", flow: "y" }));
		expect(readSheetLocalFilters(storage)).toEqual(DEFAULT_SHEET_LOCAL_FILTERS);
	});

	it("matchesSheetLocalFilters filters by origin and flow", () => {
		const real = { amount: "-10.00", origin: "real" };
		const income = { amount: "100.00", origin: "manual" };
		expect(matchesSheetLocalFilters(real, DEFAULT_SHEET_LOCAL_FILTERS)).toBe(true);
		expect(matchesSheetLocalFilters(real, { origin: "manual", flow: "all" })).toBe(false);
		expect(matchesSheetLocalFilters(income, { origin: "manual", flow: "all" })).toBe(true);
		expect(matchesSheetLocalFilters(real, { origin: "all", flow: "in" })).toBe(false);
		expect(matchesSheetLocalFilters(income, { origin: "all", flow: "in" })).toBe(true);
		expect(matchesSheetLocalFilters(income, { origin: "all", flow: "out" })).toBe(false);
	});
});

// ── Write routing (baseline vs. scenario) ───────────────────────────────────

const manualRef: SheetRowRef = {
	origin: "manual",
	forecastId: "f1",
	templateId: null,
	changeId: null,
};
const templateRef: SheetRowRef = {
	origin: "recurring",
	forecastId: "f2",
	templateId: "tpl-1",
	changeId: null,
};
const scenarioRef: SheetRowRef = {
	origin: "scenario",
	forecastId: null,
	templateId: null,
	changeId: "chg-1",
};

describe("routeSheetDelete", () => {
	it("baseline: manual one-shot deletes, template row discards for the month", () => {
		expect(routeSheetDelete(manualRef, "month", "2026-08", null)).toEqual({
			kind: "baselineDelete",
			forecastId: "f1",
		});
		expect(routeSheetDelete(templateRef, "month", "2026-08", null)).toEqual({
			kind: "baselineDiscard",
			forecastId: "f2",
		});
	});

	it("baseline: 'de {mês} em diante' ends the template from the month", () => {
		expect(routeSheetDelete(templateRef, "onward", "2026-08", null)).toEqual({
			kind: "baselineEndTemplate",
			templateId: "tpl-1",
			effectiveFrom: "2026-08",
		});
	});

	it("scenario: deletes become skip_forecast / end_template deltas", () => {
		expect(routeSheetDelete(manualRef, "month", "2026-08", "scn-1")).toEqual({
			kind: "scenarioSkip",
			forecastId: "f1",
		});
		expect(routeSheetDelete(templateRef, "month", "2026-08", "scn-1")).toEqual({
			kind: "scenarioSkip",
			forecastId: "f2",
		});
		expect(routeSheetDelete(templateRef, "onward", "2026-08", "scn-1")).toEqual({
			kind: "scenarioEndTemplate",
			templateId: "tpl-1",
			effectiveFrom: "2026-08",
		});
	});

	it("a scenario-added row removes its own change in any context", () => {
		expect(routeSheetDelete(scenarioRef, "month", "2026-08", "scn-1")).toEqual({
			kind: "scenarioRemoveChange",
			changeId: "chg-1",
		});
		expect(routeSheetDelete(scenarioRef, "onward", "2026-08", "scn-1")).toEqual({
			kind: "scenarioRemoveChange",
			changeId: "chg-1",
		});
	});
});

describe("routeSheetAmountEdit / routeSheetAdd", () => {
	it("baseline edits patch the forecast; scenario edits become adjust_amount", () => {
		expect(routeSheetAmountEdit(manualRef, null)).toEqual({
			kind: "baselinePatch",
			forecastId: "f1",
		});
		expect(routeSheetAmountEdit(templateRef, "scn-1")).toEqual({
			kind: "scenarioAdjust",
			forecastId: "f2",
		});
		expect(routeSheetAmountEdit(scenarioRef, "scn-1")).toEqual({
			kind: "scenarioReplaceOneShot",
			changeId: "chg-1",
		});
	});

	it("adds route to forecastCreate (baseline) or add_one_shot (scenario)", () => {
		expect(routeSheetAdd(null)).toBe("forecastCreate");
		expect(routeSheetAdd("scn-1")).toBe("scenarioAddOneShot");
	});
});

// Type guard: PlannedSheetRow stays exported for the view layer.
const _typecheck: PlannedSheetRow | null = null;
void _typecheck;
