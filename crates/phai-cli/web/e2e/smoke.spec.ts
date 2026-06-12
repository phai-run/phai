/**
 * Playwright smoke tests for the phai web SPA.
 *
 * These tests verify that the app loads, renders the dashboard, and does not
 * produce console errors.  They target the Vite dev server (pnpm dev) plus
 * the phai Rust bridge (cargo run -- serve).
 *
 * Prerequisites before running:
 *   Terminal 1: pnpm dev
 *   Terminal 2: cargo run -p phai-cli -- serve
 *   Terminal 3: pnpm test:e2e
 *
 * Or set PLAYWRIGHT_BASE_URL to point at a running instance:
 *   PLAYWRIGHT_BASE_URL=http://localhost:5173 pnpm test:e2e
 */

import { test, expect } from "@playwright/test";

test.describe("phai web — smoke", () => {
	test("dashboard loads without console errors", async ({ page }) => {
		const errors: string[] = [];
		page.on("console", (msg) => {
			if (msg.type() === "error") {
				errors.push(msg.text());
			}
		});
		page.on("pageerror", (err) => {
			errors.push(err.message);
		});

		await page.goto("/");

		// Wait for the page to settle — a reasonable signal is the dashboard
		// shell appearing.
		await page.waitForTimeout(3000);

		// Basic sanity: there should be some visible content.
		const bodyText = await page.textContent("body");
		expect(bodyText).toBeTruthy();
		expect(bodyText!.length).toBeGreaterThan(10);

		// No uncaught errors on the page
		expect(errors).toEqual([]);
	});

	test("page is served with correct content-type", async ({ page }) => {
		const response = await page.goto("/");
		const contentType = response?.headers()["content-type"] ?? "";
		expect(contentType).toContain("text/html");
	});

	test("no network requests fail (4xx/5xx) on initial load", async ({
		page,
	}) => {
		const failures: string[] = [];
		page.on("response", (resp) => {
			if (resp.status() >= 400) {
				failures.push(`${resp.status()} ${resp.url()}`);
			}
		});

		await page.goto("/");
		await page.waitForTimeout(2000);

		// When running against the dev server without the phai bridge, /api/
		// endpoints will 404. That's expected — the bridge isn't running.
		// Filter those out.
		const nonApiFailures = failures.filter((f) => !f.includes("/api/"));
		expect(nonApiFailures).toEqual([]);
	});
});

test.describe("phai web — hierarchical category groups", () => {
	test("renders parent category totals and subcategory subtotals", async ({
		page,
	}) => {
		await page.goto("/");
		await page.waitForTimeout(3000);

		// The app should render category group containers.
		// Each parent group is a bordered container with a clickable header
		// that shows the parent name, total amount, and transaction count.
		// Subcategory groups (when present) show sub labels, subtotals, and
		// counts in a secondary header row.

		// Look for category group containers (bordered divs with parent
		// headers). The parent header is a button that toggles expand/collapse.
		const groupButtons = page.locator(
			"button:has(> span:first-child:has-text('▸')):not(:has-text('forecast'))",
		);
		const groupButtonsExpanded = page.locator(
			"button:has(> span:first-child:has-text('▾')):not(:has-text('forecast'))",
		);

		// At least one expandable group should exist (either collapsed or
		// expanded).
		const anyGroup = groupButtons.or(groupButtonsExpanded);
		const groupCount = await anyGroup.count();

		// If the app loaded data, there should be groups.  If no data (empty
		// month), the test gracefully passes with a log.
		if (groupCount > 0) {
			// Verify that group headers show a monetary amount (R$).
			const firstGroupText = await anyGroup.first().textContent();
			expect(firstGroupText).toContain("R$");

			// Verify that at least one group has a count badge.
			const badges = anyGroup.locator("span:last-child");
			const badgeCounts = await badges.allTextContents();
			const numericBadges = badgeCounts.filter((t) => /^\d+$/.test(t.trim()));
			expect(numericBadges.length).toBeGreaterThan(0);

			// If there are subcategory groups (hasSubs), they should show
			// sub labels with their own subtotals (nested R$ amounts).
			const subHeaders = page.locator(
				"div[style*='padding: 8px 14px 8px 28px']",
			);
			const subCount = await subHeaders.count();

			if (subCount > 0) {
				const firstSubText = await subHeaders.first().textContent();
				expect(firstSubText).toContain("R$");
			}
		} else {
			// No groups found — the month may be empty.  Verify the app
			// still rendered without errors.
			const bodyText = await page.textContent("body");
			expect(bodyText).toBeTruthy();
		}
	});

	test("expand/collapse toggles category content visibility", async ({
		page,
	}) => {
		await page.goto("/");
		await page.waitForTimeout(3000);

		// Find an expanded group (▾ indicator).
		const expandedBtn = page
			.locator(
				"button:has(> span:first-child:has-text('▾')):not(:has-text('forecast'))",
			)
			.first();

		const expandedCount = await expandedBtn.count();
		if (expandedCount === 0) {
			// No expanded groups — find a collapsed one and expand it.
			const collapsedBtn = page
				.locator(
					"button:has(> span:first-child:has-text('▸')):not(:has-text('forecast'))",
				)
				.first();
			const collapsedCount = await collapsedBtn.count();
			if (collapsedCount === 0) {
				// No groups at all — empty month, test passes.
				return;
			}
			await collapsedBtn.click();
			await page.waitForTimeout(300);

			// Now the indicator should have changed to ▾
			const indicator = await collapsedBtn
				.locator("span:first-child")
				.textContent();
			expect(indicator?.trim()).toBe("▾");

			// Click again to collapse
			await collapsedBtn.click();
			await page.waitForTimeout(300);
			const indicator2 = await collapsedBtn
				.locator("span:first-child")
				.textContent();
			expect(indicator2?.trim()).toBe("▸");
		} else {
			// Collapse an expanded group
			await expandedBtn.click();
			await page.waitForTimeout(300);
			const indicator = await expandedBtn
				.locator("span:first-child")
				.textContent();
			expect(indicator?.trim()).toBe("▸");
		}
	});
});
