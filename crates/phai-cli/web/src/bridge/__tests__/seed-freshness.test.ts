/**
 * Seed-freshness decision logic.
 *
 * The v5.6.0 regression: stale-while-revalidate stamps in localStorage were
 * neither versioned nor cross-checked against the store's contents, so after
 * an upgrade reset the OPFS store, "fresh" stamps blocked the reseed and the
 * app rendered empty. These tests pin the two invariants that prevent that:
 *  - a fresh stamp never wins over an empty table;
 *  - stamps are keyed by store version + binary version, so any upgrade
 *    invalidates them.
 */
import { describe, expect, it } from "vitest";
import { STORE_ID } from "../../livestore/schema";
import { seedStampKey, shouldSkipSeed, sweepStaleSeedStamps } from "../sync";

// vitest's jsdom localStorage is a non-functional stub; back the sweep's
// injectable StampStorage with a Map instead.
const makeFakeStorage = () => {
	const entries = new Map<string, string>();
	return {
		get length() {
			return entries.size;
		},
		key: (i: number) => [...entries.keys()][i] ?? null,
		getItem: (k: string) => entries.get(k) ?? null,
		setItem: (k: string, v: string) => void entries.set(k, v),
		removeItem: (k: string) => void entries.delete(k),
	};
};

describe("shouldSkipSeed", () => {
	const maxAgeMs = 5 * 60 * 1000;
	const now = 1_000_000_000;

	it("skips when the stamp is fresh and data is present", () => {
		expect(
			shouldSkipSeed({
				now,
				stampedAt: now - 1000,
				maxAgeMs,
				isMissingData: false,
			}),
		).toBe(true);
	});

	it("fetches when the stamp is fresh but the table is empty", () => {
		expect(
			shouldSkipSeed({
				now,
				stampedAt: now - 1000,
				maxAgeMs,
				isMissingData: true,
			}),
		).toBe(false);
	});

	it("fetches when the stamp is stale", () => {
		expect(
			shouldSkipSeed({
				now,
				stampedAt: now - maxAgeMs - 1,
				maxAgeMs,
				isMissingData: false,
			}),
		).toBe(false);
	});

	it("fetches at exactly maxAgeMs (window is exclusive)", () => {
		expect(
			shouldSkipSeed({
				now,
				stampedAt: now - maxAgeMs,
				maxAgeMs,
				isMissingData: false,
			}),
		).toBe(false);
	});

	it("fetches when there is no stamp at all", () => {
		expect(
			shouldSkipSeed({ now, stampedAt: 0, maxAgeMs, isMissingData: false }),
		).toBe(false);
	});
});

describe("seedStampKey", () => {
	it("embeds the store id and the binary version", () => {
		const key = seedStampKey("tx:12:3", "5.6.1");
		expect(key).toBe(`phai:lastSync:${STORE_ID}:5.6.1:tx:12:3`);
	});

	it("differs across binary versions for the same cache key", () => {
		expect(seedStampKey("chart:6:6", "5.6.0")).not.toBe(
			seedStampKey("chart:6:6", "5.6.1"),
		);
	});

	it("does not collide with the legacy un-versioned format", () => {
		// 5.5.0 wrote `phai:lastSync:tx:12:3`; the versioned key must not read it.
		expect(seedStampKey("tx:12:3", "5.6.1")).not.toBe("phai:lastSync:tx:12:3");
	});
});

describe("sweepStaleSeedStamps", () => {
	it("removes legacy and other-version stamps, keeps current ones", () => {
		const storage = makeFakeStorage();
		storage.setItem("phai:lastSync:tx:12:3", "111"); // legacy 5.5.0 format
		storage.setItem(seedStampKey("tx:12:3", "5.6.0"), "222"); // old binary
		storage.setItem(seedStampKey("tx:12:3", "5.6.1"), "333"); // current
		storage.setItem("phai.bridgeIdentity", "keep-me"); // unrelated key

		sweepStaleSeedStamps("5.6.1", storage);

		expect(storage.getItem("phai:lastSync:tx:12:3")).toBeNull();
		expect(storage.getItem(seedStampKey("tx:12:3", "5.6.0"))).toBeNull();
		expect(storage.getItem(seedStampKey("tx:12:3", "5.6.1"))).toBe("333");
		expect(storage.getItem("phai.bridgeIdentity")).toBe("keep-me");
	});
});
