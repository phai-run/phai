/**
 * Unit tests for effectiveTx — the optimistic-overlay merge that makes an
 * unflushed edit reflect everywhere (sheet, treemap, sums), not just the modal.
 */
import { describe, expect, it } from "vitest";
import { buildOverlayMap, effectiveTx, type TxView } from "../derivations";

const mk = (patch: Partial<TxView> = {}): TxView => ({
	id: "t1",
	accountId: "acc",
	postedAt: "2026-06-05",
	amount: "-100.00",
	rawDescription: "RAW DESC",
	description: "seed desc",
	merchantName: "seed merchant",
	purpose: "seed purpose",
	categoryId: "mercado",
	month: "2026-06",
	paymentStatus: "posted",
	reviewed: 1,
	isInstallment: 0,
	isSubscription: 0,
	...patch,
});

describe("effectiveTx", () => {
	it("returns the same tx when there is no overlay", () => {
		const tx = mk();
		expect(effectiveTx(tx, buildOverlayMap([]))).toBe(tx);
	});

	it("applies edited description/merchant/purpose/category over the seed", () => {
		const overlay = buildOverlayMap([
			{
				transactionId: "t1",
				description: "Aluguel",
				merchantName: "Imobiliária",
				purpose: "Moradia",
				categoryId: "moradia:aluguel",
			},
		]);
		const out = effectiveTx(mk(), overlay);
		expect(out.description).toBe("Aluguel");
		expect(out.merchantName).toBe("Imobiliária");
		expect(out.purpose).toBe("Moradia");
		expect(out.categoryId).toBe("moradia:aluguel");
	});

	it("does not clobber the seed with unset (null) patch fields", () => {
		// A category-only edit leaves the other fields null in the overlay.
		const overlay = buildOverlayMap([
			{
				transactionId: "t1",
				description: null,
				merchantName: null,
				purpose: null,
				categoryId: "moradia:aluguel",
			},
		]);
		const out = effectiveTx(mk(), overlay);
		expect(out.description).toBe("seed desc"); // preserved
		expect(out.categoryId).toBe("moradia:aluguel"); // applied
	});

	it("carries the commitment-tier override through", () => {
		const overlay = buildOverlayMap([
			{
				transactionId: "t1",
				description: null,
				merchantName: null,
				purpose: null,
				categoryId: null,
				commitmentTier: "locked",
			},
		]);
		expect(effectiveTx(mk(), overlay).commitmentTier).toBe("locked");
	});
});
