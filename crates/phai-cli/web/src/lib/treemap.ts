/**
 * Squarified treemap layout (Bruls, Huizing & van Wijk). Pure geometry — no
 * React, no DOM — so the categorias drill-down can be unit-tested exactly.
 *
 * Items are laid out in rows along the shorter side of the remaining
 * rectangle; a row is closed when adding the next item would worsen the worst
 * aspect ratio in the row. Output coordinates live in the same unit space as
 * the input rectangle (callers typically use percentages).
 */

export interface TreemapItem {
	id: string;
	/** Non-negative magnitude. Zero/negative items are skipped. */
	value: number;
}

export interface TreemapRect extends TreemapItem {
	x: number;
	y: number;
	w: number;
	h: number;
}

/** Worst aspect ratio of a row of areas laid along a side of length `len`. */
const worst = (row: number[], len: number): number => {
	const s = row.reduce((a, b) => a + b, 0);
	if (s === 0 || len === 0) return Infinity;
	let max = 0;
	let min = Infinity;
	for (const r of row) {
		if (r > max) max = r;
		if (r < min) min = r;
	}
	const s2 = s * s;
	const l2 = len * len;
	return Math.max((l2 * max) / s2, s2 / (l2 * min));
};

/**
 * Lay `items` inside the rectangle `(x, y, w, h)`. Returns one rect per
 * positive-value item, sorted by value descending (stable for ties by id).
 * Areas are proportional to values; rects tile the rectangle exactly.
 */
export const squarify = (
	items: ReadonlyArray<TreemapItem>,
	x: number,
	y: number,
	w: number,
	h: number,
): TreemapRect[] => {
	const positive = items
		.filter((i) => i.value > 0)
		.sort((a, b) => b.value - a.value || (a.id < b.id ? -1 : 1));
	if (positive.length === 0 || w <= 0 || h <= 0) return [];

	const total = positive.reduce((s, i) => s + i.value, 0);
	const scale = (w * h) / total;
	// Work in areas from here on.
	const areas = positive.map((i) => i.value * scale);

	const out: TreemapRect[] = [];
	let rx = x;
	let ry = y;
	let rw = w;
	let rh = h;
	let idx = 0;

	while (idx < positive.length) {
		const len = Math.min(rw, rh);
		// Grow the row while the worst aspect ratio does not degrade.
		const row: number[] = [areas[idx]!];
		let next = idx + 1;
		while (next < positive.length) {
			const candidate = [...row, areas[next]!];
			if (worst(candidate, len) > worst(row, len)) break;
			row.push(areas[next]!);
			next += 1;
		}

		// Fix the row along the shorter side.
		const rowSum = row.reduce((a, b) => a + b, 0);
		const thickness = rowSum / len;
		let offset = 0;
		for (let k = 0; k < row.length; k++) {
			const extent = row[k]! / thickness;
			const item = positive[idx + k]!;
			if (rw >= rh) {
				// Row is a vertical strip on the left edge.
				out.push({
					...item,
					x: rx,
					y: ry + offset,
					w: thickness,
					h: extent,
				});
			} else {
				// Row is a horizontal strip on the top edge.
				out.push({
					...item,
					x: rx + offset,
					y: ry,
					w: extent,
					h: thickness,
				});
			}
			offset += extent;
		}

		if (rw >= rh) {
			rx += thickness;
			rw -= thickness;
		} else {
			ry += thickness;
			rh -= thickness;
		}
		idx = next;
	}

	return out;
};
