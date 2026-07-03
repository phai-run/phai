/**
 * Regression: POST /api/forecast responds with `{ forecastId }` (camelCase) for
 * BOTH create and envelope upsert. The forecast-create flush read the wrong key
 * (`forecast_id`), so `serverForecastId` came back `undefined` and the
 * `v1.ForecastCreateAcked` event failed schema validation — the inline "add
 * row" gesture errored on every write. Pin the response contract here.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";

const stubFetch = (bodyObj: unknown) =>
	vi.spyOn(globalThis, "fetch").mockResolvedValue({
		ok: true,
		status: 200,
		statusText: "OK",
		json: async () => bodyObj,
	} as unknown as Response);

afterEach(() => vi.restoreAllMocks());

describe("createForecast response contract", () => {
	it("reads the camelCase forecastId returned by the backend", async () => {
		stubFetch({ forecastId: "srv-123" });
		const res = await api.createForecast({
			description: "Presente",
			amount: "-120.50",
			due_date: "2026-07-18",
			category_id: "compras:presentes",
			ui_role: "planned_transaction",
		});
		// The flush copies this into forecastCreateAcked.serverForecastId, which
		// the LiveStore schema requires to be a string — undefined here was the bug.
		expect(res.forecastId).toBe("srv-123");
		expect(typeof res.forecastId).toBe("string");
	});

	it("posts to /api/forecast", async () => {
		const spy = stubFetch({ forecastId: "srv-1" });
		await api.createForecast({
			description: "x",
			amount: "-1.00",
			due_date: "2026-07-01",
		});
		expect(spy).toHaveBeenCalledWith(
			"/api/forecast",
			expect.objectContaining({ method: "POST" }),
		);
	});
});
