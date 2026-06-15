/**
 * The sync chip is now a button: clicking it forces a flush (onRetry) in any
 * state, not just on error — so a user who wants to be sure before shutting
 * down can poke it. It still reflects pending/error/synced state.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { SyncChip } from "../App";

afterEach(cleanup);

describe("SyncChip", () => {
	it("forces a sync via onRetry when clicked, even while synced", () => {
		const onRetry = vi.fn();
		render(<SyncChip pending={0} error={null} onRetry={onRetry} />);
		fireEvent.click(screen.getByRole("button"));
		expect(onRetry).toHaveBeenCalledTimes(1);
	});

	it("shows the pending count while writes are queued", () => {
		render(<SyncChip pending={3} error={null} />);
		expect(screen.getByRole("button").textContent).toContain("3 pending");
	});

	it("surfaces the error state", () => {
		render(<SyncChip pending={0} error="boom" />);
		expect(screen.getByRole("button").textContent).toContain("error · boom");
	});
});
