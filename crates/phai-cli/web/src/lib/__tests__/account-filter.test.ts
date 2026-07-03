/**
 * Unit tests for the multi-select account filter helpers. The selection is a
 * comma-joined id list stored in `ui.accountFilter` (single id = one-element
 * list) so the ui-doc schema stays untouched. Empty/null means "all accounts".
 */
import { describe, expect, it } from "vitest";
import { accountFilterIds, matchesAccountFilter } from "../derivations";

describe("accountFilterIds", () => {
	it("returns an empty list for null / empty / whitespace", () => {
		expect(accountFilterIds(null)).toEqual([]);
		expect(accountFilterIds("")).toEqual([]);
		expect(accountFilterIds("   ")).toEqual([]);
	});

	it("parses a single id (backward-compatible with the old string filter)", () => {
		expect(accountFilterIds("acc-1")).toEqual(["acc-1"]);
	});

	it("splits a comma-joined list and trims blanks", () => {
		expect(accountFilterIds("acc-1, acc-2 ,acc-3")).toEqual([
			"acc-1",
			"acc-2",
			"acc-3",
		]);
		expect(accountFilterIds("acc-1,,acc-2,")).toEqual(["acc-1", "acc-2"]);
	});
});

describe("matchesAccountFilter", () => {
	it("passes every account when no filter is set", () => {
		expect(matchesAccountFilter(null, "acc-1")).toBe(true);
		expect(matchesAccountFilter("", "acc-9")).toBe(true);
	});

	it("matches a single selected account", () => {
		expect(matchesAccountFilter("acc-1", "acc-1")).toBe(true);
		expect(matchesAccountFilter("acc-1", "acc-2")).toBe(false);
	});

	it("matches any account in a multi-select", () => {
		expect(matchesAccountFilter("acc-1,acc-2", "acc-2")).toBe(true);
		expect(matchesAccountFilter("acc-1,acc-2", "acc-3")).toBe(false);
	});

	it("excludes the empty account id when a filter is active", () => {
		expect(matchesAccountFilter("acc-1", "")).toBe(false);
	});
});
