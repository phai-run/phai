import { describe, expect, it } from "vitest";
import type { TxView } from "../../../lib/derivations";
import {
	csvAmountCell,
	sheetAmountLabel,
	sheetRowsCsv,
	sheetSignedTotal,
} from "../PlanilhaView";

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
				'tx-1,2026-06-10,"Store, monthly","Store ""A""","school\nfees",Conta Principal,educacao:escola,"-123,45",\n',
		);
	});
});

describe("csvAmountCell", () => {
	it("formats decimal amounts with a Brazilian decimal comma", () => {
		expect(csvAmountCell("589.89")).toBe("589,89");
	});

	it("preserves negative expense signs", () => {
		expect(csvAmountCell("-123.45")).toBe("-123,45");
	});
});

describe("sheetAmountLabel", () => {
	it("formats expenses with accounting parentheses", () => {
		expect(sheetAmountLabel("-4891.03").replace(/\s/g, " ")).toBe(
			"(R$ 4.891,03)",
		);
	});

	it("keeps income as a positive amount", () => {
		expect(sheetAmountLabel("8723.14").replace(/\s/g, " ")).toBe(
			"R$ 8.723,14",
		);
	});
});

describe("sheetSignedTotal", () => {
	it("sums the visible sheet rows as a signed total", () => {
		expect(
			sheetSignedTotal([
				tx({ id: "tx-expense-1", amount: "-123.45" }),
				tx({ id: "tx-expense-2", amount: "-0.55" }),
				tx({ id: "tx-income", amount: "50.00" }),
			]),
		).toBe(-74);
	});
});
