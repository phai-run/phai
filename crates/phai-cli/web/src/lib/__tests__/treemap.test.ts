import { describe, expect, it } from "vitest";
import { squarify, type TreemapRect } from "../treemap";

const area = (r: TreemapRect) => r.w * r.h;

describe("squarify", () => {
	it("returns empty for empty input or degenerate rectangle", () => {
		expect(squarify([], 0, 0, 100, 100)).toEqual([]);
		expect(squarify([{ id: "a", value: 1 }], 0, 0, 0, 100)).toEqual([]);
	});

	it("a single item fills the whole rectangle", () => {
		const [r] = squarify([{ id: "a", value: 42 }], 0, 0, 100, 60);
		expect(r).toMatchObject({ id: "a", x: 0, y: 0 });
		expect(r!.w).toBeCloseTo(100);
		expect(r!.h).toBeCloseTo(60);
	});

	it("areas are proportional to values and tile the container", () => {
		const rects = squarify(
			[
				{ id: "a", value: 6 },
				{ id: "b", value: 3 },
				{ id: "c", value: 1 },
			],
			0,
			0,
			100,
			100,
		);
		const total = rects.reduce((s, r) => s + area(r), 0);
		expect(total).toBeCloseTo(100 * 100, 6);
		const byId = new Map(rects.map((r) => [r.id, r]));
		expect(area(byId.get("a")!)).toBeCloseTo(6000, 6);
		expect(area(byId.get("b")!)).toBeCloseTo(3000, 6);
		expect(area(byId.get("c")!)).toBeCloseTo(1000, 6);
	});

	it("skips zero and negative values", () => {
		const rects = squarify(
			[
				{ id: "a", value: 5 },
				{ id: "zero", value: 0 },
				{ id: "neg", value: -3 },
			],
			0,
			0,
			10,
			10,
		);
		expect(rects.map((r) => r.id)).toEqual(["a"]);
	});

	it("rects stay inside the container and do not overlap", () => {
		const rects = squarify(
			Array.from({ length: 9 }, (_, i) => ({
				id: `c${i}`,
				value: (i + 1) * 7,
			})),
			0,
			0,
			160,
			90,
		);
		for (const r of rects) {
			expect(r.x).toBeGreaterThanOrEqual(-1e-9);
			expect(r.y).toBeGreaterThanOrEqual(-1e-9);
			expect(r.x + r.w).toBeLessThanOrEqual(160 + 1e-6);
			expect(r.y + r.h).toBeLessThanOrEqual(90 + 1e-6);
		}
		// Pairwise overlap area must be ~zero.
		for (let i = 0; i < rects.length; i++) {
			for (let j = i + 1; j < rects.length; j++) {
				const a = rects[i]!;
				const b = rects[j]!;
				const ox = Math.max(
					0,
					Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x),
				);
				const oy = Math.max(
					0,
					Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y),
				);
				expect(ox * oy).toBeLessThan(1e-6);
			}
		}
	});

	it("keeps aspect ratios sane for a typical distribution", () => {
		const rects = squarify(
			[
				{ id: "moradia", value: 9300 },
				{ id: "saude", value: 7800 },
				{ id: "alimentacao", value: 7500 },
				{ id: "educacao", value: 4900 },
				{ id: "transporte", value: 2000 },
				{ id: "compras", value: 1600 },
				{ id: "assinaturas", value: 1500 },
				{ id: "outros", value: 600 },
			],
			0,
			0,
			100,
			56,
		);
		for (const r of rects) {
			const ratio = Math.max(r.w / r.h, r.h / r.w);
			expect(ratio).toBeLessThan(4.5);
		}
	});
});
