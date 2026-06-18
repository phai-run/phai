import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
	TransactionModal,
	type ReviewPatch,
} from "../TransactionModal";
import type { TxView } from "../../lib/derivations";

const { commitMock } = vi.hoisted(() => ({
	commitMock: vi.fn(),
}));

vi.mock("@livestore/react", () => ({
	useStore: () => ({
		store: { commit: commitMock },
	}),
}));

afterEach(() => {
	cleanup();
	commitMock.mockReset();
});

const tx = (id: string, postedAt: string): TxView => ({
	id,
	accountId: "checking",
	postedAt,
	amount: "-100.00",
	rawDescription: `Raw ${id}`,
	description: `Description ${id}`,
	merchantName: "Merchant",
	purpose: null,
	categoryId: "moradia",
	month: postedAt.slice(0, 7),
	paymentStatus: "posted",
	reviewed: 0,
	isInstallment: 0,
	isSubscription: 0,
});

describe("TransactionModal", () => {
	it("saves the current transaction and selected similar transactions together", async () => {
		const onSubmit = vi.fn();
		const expectedPatch: ReviewPatch = {
			description: "Updated description",
			merchantName: "Merchant",
			purpose: null,
			categoryId: "moradia",
		};

		render(
			<TransactionModal
				tx={tx("current", "2026-01-10")}
				overlay={undefined}
				similarTxs={[
					tx("similar-1", "2026-02-10"),
					tx("similar-2", "2026-03-10"),
				]}
				overlayById={new Map()}
				categories={["moradia"]}
				onSubmit={onSubmit}
				onClose={vi.fn()}
			/>,
		);

		fireEvent.change(screen.getByPlaceholderText("description"), {
			target: { value: "Updated description" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Similar (2)" }));
		fireEvent.click(
			await screen.findByRole("button", { name: "select all (2)" }),
		);
		fireEvent.click(
			screen.getByRole("button", {
				name: "Salvar (também em 2 selecionadas)",
			}),
		);

		expect(onSubmit).toHaveBeenCalledWith(expectedPatch);
		expect(commitMock).toHaveBeenCalledTimes(2);
		expect(
			commitMock.mock.calls.map(([event]) => event.args.transactionId),
		).toEqual(["similar-1", "similar-2"]);
		expect(commitMock.mock.calls.map(([event]) => event.args.patch)).toEqual([
			expectedPatch,
			expectedPatch,
		]);
	});
});
