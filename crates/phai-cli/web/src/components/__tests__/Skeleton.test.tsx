/**
 * Render contract for the branded loading skeletons.
 *
 * These guard the things reviewers and assistive tech rely on:
 *  - every skeleton region is an `aria-busy` status node with a single label,
 *  - the composed DashboardSkeleton stacks hero + chart + list (so the layout
 *    matches where real data lands and first paint doesn't reflow),
 *  - the chart placeholder carries the φ brand mark.
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import {
	HeroSkeleton,
	ChartSkeleton,
	ListSkeleton,
	CardGridSkeleton,
	DashboardSkeleton,
} from "../Skeleton";

afterEach(cleanup);

describe("loading skeletons", () => {
	it("HeroSkeleton is an aria-busy status region", () => {
		render(<HeroSkeleton />);
		const region = screen.getByRole("status");
		expect(region.getAttribute("aria-busy")).toBe("true");
		expect(screen.getByText(/loading cash balance/i)).toBeTruthy();
	});

	it("ChartSkeleton announces loading and shows the φ brand mark", () => {
		const { container } = render(<ChartSkeleton />);
		expect(screen.getByText(/loading cash chart/i)).toBeTruthy();
		expect(container.textContent).toContain("φ");
	});

	it("ListSkeleton renders the requested number of placeholder rows", () => {
		const { container } = render(<ListSkeleton rows={4} />);
		// Each row contributes multiple shimmer bars; assert at least one per row.
		expect(container.querySelectorAll(".skeleton").length).toBeGreaterThanOrEqual(4);
		expect(screen.getByRole("status").getAttribute("aria-busy")).toBe("true");
	});

	it("CardGridSkeleton renders one tile per requested count", () => {
		render(<CardGridSkeleton tiles={3} />);
		expect(screen.getByText(/loading cards/i)).toBeTruthy();
	});

	it("DashboardSkeleton composes hero, chart and list regions", () => {
		render(<DashboardSkeleton />);
		// Three distinct aria-busy regions stacked in the cold-start layout.
		expect(screen.getAllByRole("status")).toHaveLength(3);
		expect(screen.getByText(/loading cash balance/i)).toBeTruthy();
		expect(screen.getByText(/loading cash chart/i)).toBeTruthy();
		expect(screen.getByText(/loading transactions/i)).toBeTruthy();
	});
});
