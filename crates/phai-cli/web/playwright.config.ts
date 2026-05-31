import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright config for phai web e2e smoke tests.
 *
 * Targets the local `phai serve` SPA when running in dev mode.
 * Run with: pnpm test:e2e
 *
 * Prerequisites:
 *   1. Start the dev server:  pnpm dev
 *   2. Start the phai bridge: cargo run -p phai-cli -- serve
 */

export default defineConfig({
	testDir: "./e2e",
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 2 : 0,
	workers: process.env.CI ? 1 : undefined,
	reporter: "list",

	use: {
		baseURL: process.env.PLAYWRIGHT_BASE_URL ?? "http://localhost:5173",
	},

	projects: [
		{
			name: "chromium",
			use: { ...devices["Desktop Chrome"] },
		},
	],

	// Dev server is started externally (pnpm dev), not by Playwright.
	// No webServer config — the SPA + bridge require separate processes.
});
