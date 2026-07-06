import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/// <reference types="vitest/config" />

// Built bundle is embedded into the `phai` binary and served from `phai serve`
// at the site root. Relative asset URLs keep it mount-point agnostic.
// Cross-origin isolation headers — required for OPFS + SharedWorker (LiveStore)
// during local `vite dev` / `vite preview`. Mirrors what `phai serve` sends in
// production (serve_assets.rs): `require-corp`, not `credentialless`, because
// WebKit/WKWebView (the native desktop shell engine) ignores `credentialless`.
// Fonts are self-hosted, so there are no cross-origin subresources. ADR-0039.
const crossOriginIsolation = {
	"Cross-Origin-Opener-Policy": "same-origin",
	"Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
	base: "./",
	worker: { format: "es" },
	server: { headers: crossOriginIsolation },
	preview: { headers: crossOriginIsolation },
	optimizeDeps: {
		exclude: ["@livestore/wa-sqlite"],
	},
	build: {
		outDir: "dist",
		emptyOutDir: true,
		target: "es2022",
		rollupOptions: {
			output: {
				manualChunks(id) {
					if (!id.includes("node_modules")) return undefined;
					if (id.includes("@livestore")) return "vendor-livestore";
					if (id.includes("framer-motion")) return "vendor-motion";
					if (id.includes("react")) return "vendor-react";
					return "vendor";
				},
			},
		},
	},
	plugins: [react()],
	test: {
		environment: "jsdom",
		globals: false,
		include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
		setupFiles: [],
	},
});
