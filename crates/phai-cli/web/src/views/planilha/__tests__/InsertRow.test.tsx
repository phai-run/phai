import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { InsertRowEditor, type InsertDraft } from "../InsertRow";

afterEach(cleanup);

const renderEditor = (onSubmit: (d: InsertDraft) => void) =>
	render(
		<table>
			<tbody>
				<InsertRowEditor
					defaultDay={10}
					maxDay={31}
					contextLabel="baseline"
					colSpan={6}
					onSubmit={onSubmit}
					onCancel={() => {}}
				/>
			</tbody>
		</table>,
	);

describe("InsertRowEditor", () => {
	it("defaults to despesa and submits a negative-intent draft", () => {
		const onSubmit = vi.fn();
		renderEditor(onSubmit);
		fireEvent.change(screen.getByPlaceholderText("descrição"), {
			target: { value: "Aluguel" },
		});
		fireEvent.change(screen.getByLabelText("valor"), { target: { value: "1200" } });
		fireEvent.click(screen.getByTitle("salvar (Enter)"));
		expect(onSubmit).toHaveBeenCalledTimes(1);
		expect(onSubmit.mock.calls[0][0]).toMatchObject({
			description: "Aluguel",
			magnitude: "1200",
			isExpense: true,
			day: 10,
		});
	});

	it("the entrada toggle flips the sign of the submitted draft", () => {
		const onSubmit = vi.fn();
		renderEditor(onSubmit);
		fireEvent.change(screen.getByPlaceholderText("descrição"), {
			target: { value: "Salário" },
		});
		fireEvent.change(screen.getByLabelText("valor"), { target: { value: "5000" } });
		fireEvent.click(screen.getByRole("radio", { name: "entrada" }));
		fireEvent.click(screen.getByTitle("salvar (Enter)"));
		expect(onSubmit.mock.calls[0][0]).toMatchObject({
			description: "Salário",
			isExpense: false,
		});
	});

	it("blocks submit until description, amount and a valid day are present", () => {
		const onSubmit = vi.fn();
		renderEditor(onSubmit);
		// no description/amount yet → Enter does nothing
		fireEvent.keyDown(screen.getByPlaceholderText("descrição"), { key: "Enter" });
		expect(onSubmit).not.toHaveBeenCalled();
		// invalid day (> maxDay) also blocks
		fireEvent.change(screen.getByPlaceholderText("descrição"), {
			target: { value: "x" },
		});
		fireEvent.change(screen.getByLabelText("valor"), { target: { value: "10" } });
		fireEvent.change(screen.getByLabelText("dia"), { target: { value: "99" } });
		fireEvent.click(screen.getByTitle("salvar (Enter)"));
		expect(onSubmit).not.toHaveBeenCalled();
	});
});
