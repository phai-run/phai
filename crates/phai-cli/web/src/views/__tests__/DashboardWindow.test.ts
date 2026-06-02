import { describe, expect, it } from "vitest";
import { planningYearWindow } from "../Dashboard";

describe("planningYearWindow", () => {
	it("requests January through December for a June dashboard", () => {
		const window = planningYearWindow(new Date("2026-06-02T12:00:00"));

		expect(window).toEqual({
			chartMonthsBack: 6,
			transactionMonthsBack: 5,
			monthsAhead: 6,
		});
	});

	it("uses the full current year in January", () => {
		const window = planningYearWindow(new Date("2026-01-15T12:00:00"));

		expect(window).toEqual({
			chartMonthsBack: 1,
			transactionMonthsBack: 0,
			monthsAhead: 11,
		});
	});
});
