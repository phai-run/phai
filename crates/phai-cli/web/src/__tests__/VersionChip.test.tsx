import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import { VersionChip } from "../App";

afterEach(() => {
	vi.useRealTimers();
	cleanup();
});

describe("VersionChip", () => {
	it("renders the current version", () => {
		render(<VersionChip currentVersion="5.40.0" updateAvailable={false} applyUpdate={vi.fn()} />);
		expect(screen.getByRole("button").textContent).toContain("v5.40.0");
	});

	it("calls applyUpdate when clicked", () => {
		const applyUpdate = vi.fn().mockResolvedValue(undefined);
		render(<VersionChip currentVersion="5.40.0" updateAvailable={false} applyUpdate={applyUpdate} />);
		fireEvent.click(screen.getByRole("button"));
		expect(applyUpdate).toHaveBeenCalledTimes(1);
	});

	it("shows checking state and disables the button while pending", () => {
		let resolve!: () => void;
		const promise = new Promise<void>((r) => { resolve = r; });
		render(<VersionChip currentVersion="5.40.0" updateAvailable={false} applyUpdate={() => promise} />);
		fireEvent.click(screen.getByRole("button"));
		expect(screen.getByRole("button")).toHaveProperty("disabled", true);
		expect(screen.getByRole("button").textContent).toContain("⟳");
		resolve();
	});

	it("clears checking state when the promise resolves", async () => {
		render(<VersionChip currentVersion="5.40.0" updateAvailable={false} applyUpdate={() => Promise.resolve()} />);
		fireEvent.click(screen.getByRole("button"));
		await waitFor(() => expect(screen.getByRole("button")).toHaveProperty("disabled", false));
		expect(screen.getByRole("button").textContent).toContain("v5.40.0");
	});

	it("shows error title and then returns to idle", async () => {
		vi.useFakeTimers();
		render(<VersionChip currentVersion="5.40.0" updateAvailable={false} applyUpdate={() => Promise.reject(new Error("boom"))} />);
		fireEvent.click(screen.getByRole("button"));
		await waitFor(() => expect(screen.getByRole("button")).toHaveAttribute("title", "boom"));
		vi.advanceTimersByTime(4_000);
		await waitFor(() => expect(screen.getByRole("button")).toHaveAttribute("title", "Verificar atualizações agora"));
	});
});
