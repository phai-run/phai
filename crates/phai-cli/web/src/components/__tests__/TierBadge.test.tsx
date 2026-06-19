/**
 * TierBadge is the shared visual for a transaction's commitment tier. It must
 * render the right label/icon per tier and stay icon-only in compact mode
 * (dense sheet rows) while keeping the full label in the tooltip.
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import { TierBadge, TIER_ICON } from "../TierBadge";

afterEach(cleanup);

describe("TierBadge", () => {
	it("renders label + icon for each tier", () => {
		render(<TierBadge tier="locked" />);
		render(<TierBadge tier="cancellable" />);
		render(<TierBadge tier="variable" />);
		expect(screen.getByText("locked")).toBeTruthy();
		expect(screen.getByText("cancellable")).toBeTruthy();
		expect(screen.getByText("variable")).toBeTruthy();
		expect(screen.getByText(TIER_ICON.locked)).toBeTruthy();
	});

	it("hides the label in compact mode but keeps it in the tooltip", () => {
		const { container } = render(<TierBadge tier="locked" compact />);
		expect(screen.queryByText("locked")).toBeNull();
		expect(screen.getByText(TIER_ICON.locked)).toBeTruthy();
		expect(container.querySelector('[title="tier: locked"]')).toBeTruthy();
	});
});
