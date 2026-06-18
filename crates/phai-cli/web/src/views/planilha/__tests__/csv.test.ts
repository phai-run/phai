import { describe, expect, it } from "vitest";
import type { TxView } from "../../../lib/derivations";
import { sheetRowsCsv } from "../PlanilhaView";

const tx = (overrides: Partial<TxView> = {}): TxView => ({
	id: "tx-1",
	accountId: "acct-1",
	postedAt: "2026-06-10",
	amount: "-123.45",
	rawDescription: "RAW STORE",
	description: "Store, monthly",
	merchantName: 'Store "A"',
	purpose: "school\nfees",
	categoryId: "educacao:escola",
	month: "2026-06",
	paymentStatus: "posted",
	reviewed: 1,
	isInstallment: 0,
	isSubscription: 0,
	...overrides,
});

describe("sheetRowsCsv", () => {
	it("exports visible sheet rows with stable columns and CSV escaping", () => {
		const csv = sheetRowsCsv(
			[tx()],
			new Map([["acct-1", { label: "Conta Principal" }]]),
		);

		expect(csv).toBe(
			'transaction_id,posted_at,description,merchant_name,purpose,account,category_id,amount,installment\n' +
				'tx-1,2026-06-10,"Store, monthly","Store ""A""","school\nfees",Conta Principal,educacao:escola,-123.45,\n',
		);
	});
});
