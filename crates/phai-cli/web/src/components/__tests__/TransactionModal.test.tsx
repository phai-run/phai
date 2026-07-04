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

		fireEvent.change(screen.getByPlaceholderText("descrição"), {
			target: { value: "Updated description" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Semelhantes (2)" }));
		fireEvent.click(
			await screen.findByRole("button", { name: "selecionar todas (2)" }),
		);
		fireEvent.click(
			screen.getByRole("button", {
				name: "Salvar (também em 2 selecionadas)",
			}),
		);

		// The source tx carries its commitment tier (here none → null); the bulk
		// writes to similar txs strip it so each row keeps its own tier.
		expect(onSubmit).toHaveBeenCalledWith({
			...expectedPatch,
			commitmentTier: null,
		});
		expect(commitMock).toHaveBeenCalledTimes(2);
		expect(
			commitMock.mock.calls.map(([event]) => event.args.transactionId),
		).toEqual(["similar-1", "similar-2"]);
		expect(commitMock.mock.calls.map(([event]) => event.args.patch)).toEqual([
			expectedPatch,
			expectedPatch,
		]);
	});

	it("preserves the locked tier on the edited tx but does not force it onto similar (regression)", async () => {
		const onSubmit = vi.fn();
		const locked = { ...tx("current", "2026-01-10"), commitmentTier: "locked" };

		render(
			<TransactionModal
				tx={locked}
				overlay={undefined}
				similarTxs={[tx("similar-1", "2026-02-10")]}
				overlayById={new Map()}
				categories={["moradia"]}
				onSubmit={onSubmit}
				onClose={vi.fn()}
			/>,
		);

		fireEvent.change(screen.getByPlaceholderText("descrição"), {
			target: { value: "Aluguel do apê" },
		});
		fireEvent.click(screen.getByRole("button", { name: "Semelhantes (1)" }));
		fireEvent.click(
			await screen.findByRole("button", { name: "selecionar todas (1)" }),
		);
		fireEvent.click(
			screen.getByRole("button", { name: "Salvar (também em 1 selecionadas)" }),
		);

		// Source keeps locked …
		expect(onSubmit).toHaveBeenCalledWith(
			expect.objectContaining({ commitmentTier: "locked" }),
		);
		// … similar row's write carries no tier (keeps its own).
		const bulkPatch = commitMock.mock.calls[0][0].args.patch;
		expect(bulkPatch.commitmentTier).toBeUndefined();
	});
});
