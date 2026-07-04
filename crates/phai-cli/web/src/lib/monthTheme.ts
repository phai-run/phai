/**
 * Per-month visual theme — a discreet way to tell each month's sheet apart at a
 * glance. Every "YYYY-MM" maps to a stable accent hue and a small thematic glyph
 * (Brazilian seasons: summer in Jan, winter in Jul). Kept intentionally subtle:
 * views use `accent` for a thin hairline/tint and `glyph` as a single icon by
 * the heading — never a full-bleed illustration. Deterministic (no randomness),
 * so the same month always looks the same across sessions and devices.
 */
export interface MonthTheme {
	/** 1..12. */
	month: number;
	/** Accent colour (AA-safe on white) used for hairlines and tints. */
	accent: string;
	/** A soft translucent wash of the accent, for large surfaces. */
	tint: string;
	/** One thematic emoji — the month's "season" in Brazil. */
	glyph: string;
	/** Short season word, for the heading's title attribute / a11y. */
	season: string;
}

// Twelve distinct, muted hues walking the colour wheel so adjacent months never
// collide. Paired with a season glyph that matches the Southern-hemisphere
// calendar (Jan = high summer, Jul = deep winter).
const THEMES: ReadonlyArray<Omit<MonthTheme, "month" | "tint">> = [
	{ accent: "#e11d48", glyph: "🌞", season: "verão" }, // Jan
	{ accent: "#c026d3", glyph: "🎭", season: "verão / carnaval" }, // Feb
	{ accent: "#7c3aed", glyph: "🍂", season: "outono" }, // Mar
	{ accent: "#4f46e5", glyph: "🌧️", season: "outono" }, // Apr
	{ accent: "#0369a1", glyph: "🍁", season: "outono" }, // May
	{ accent: "#0891b2", glyph: "🔥", season: "inverno / festa junina" }, // Jun
	{ accent: "#0d9488", glyph: "❄️", season: "inverno" }, // Jul
	{ accent: "#15803d", glyph: "🌬️", season: "inverno" }, // Aug
	{ accent: "#65a30d", glyph: "🌱", season: "primavera" }, // Sep
	{ accent: "#b45309", glyph: "🌸", season: "primavera" }, // Oct
	{ accent: "#c2410c", glyph: "🌺", season: "primavera" }, // Nov
	{ accent: "#be123c", glyph: "🎄", season: "verão / festas" }, // Dec
];

/** Translucent wash of a hex accent (for tinted surfaces). */
const washOf = (hex: string, alpha = 0.06): string => {
	const n = hex.replace("#", "");
	const r = parseInt(n.slice(0, 2), 16);
	const g = parseInt(n.slice(2, 4), 16);
	const b = parseInt(n.slice(4, 6), 16);
	return `rgba(${r}, ${g}, ${b}, ${alpha})`;
};

/** Theme for a "YYYY-MM" (or a bare month number). Falls back to the purple brand. */
export const monthTheme = (month: string): MonthTheme => {
	const m = Number(month.slice(5, 7) || month);
	const idx = Number.isFinite(m) && m >= 1 && m <= 12 ? m - 1 : 0;
	const base = THEMES[idx];
	return {
		month: idx + 1,
		accent: base.accent,
		tint: washOf(base.accent),
		glyph: base.glyph,
		season: base.season,
	};
};
