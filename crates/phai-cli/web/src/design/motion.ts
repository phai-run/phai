/**
 * Shared motion system — one place for the app's animation vocabulary so every
 * surface feels consistent: fast, springy, never sluggish. Finance is usually
 * dull; a little life (count-ups, gentle springs, staggered reveals) makes it
 * pleasant without getting in the way.
 *
 * Everything here honours `prefers-reduced-motion`: when the user opts out,
 * durations collapse to ~0 and count-ups jump straight to the final value.
 */
import { useEffect, useRef, useState } from "react";

/** True when the user asked the OS to minimise motion. */
export const prefersReducedMotion = (): boolean => {
	if (typeof window === "undefined" || !window.matchMedia) return false;
	return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
};

// ── framer-motion presets ───────────────────────────────────────────────────

/** Snappy spring for expand/collapse and layout shifts. */
export const springExpand = {
	type: "spring",
	stiffness: 520,
	damping: 36,
	mass: 0.8,
} as const;

/** Quick ease for fades. */
export const fast = { duration: 0.16, ease: "easeOut" } as const;

/** Fade-up reveal — pair with a stagger for lists. */
export const fadeUp = {
	initial: { opacity: 0, y: 6 },
	animate: { opacity: 1, y: 0 },
	exit: { opacity: 0, y: 6 },
	transition: fast,
} as const;

/** Per-child delay for staggered list reveals. */
export const stagger = (index: number, step = 0.02): number => index * step;

// ── count-up hook ─────────────────────────────────────────────────────────

const easeOutCubic = (t: number): number => 1 - (1 - t) ** 3;

/**
 * Animate a number from its previous value to `target` over `durationMs` with
 * an ease-out curve (requestAnimationFrame). Returns the current value to
 * render. Honours reduced-motion (returns `target` immediately). Great for the
 * headline money figures so they "roll" when the selected month changes.
 */
export const useCountUp = (target: number, durationMs = 420): number => {
	const [value, setValue] = useState(target);
	const fromRef = useRef(target);
	const rafRef = useRef(0);

	useEffect(() => {
		if (prefersReducedMotion() || durationMs <= 0) {
			setValue(target);
			fromRef.current = target;
			return;
		}
		const from = fromRef.current;
		if (from === target) return;
		const start = performance.now();
		const tick = (now: number) => {
			const t = Math.min(1, (now - start) / durationMs);
			const v = from + (target - from) * easeOutCubic(t);
			setValue(v);
			if (t < 1) {
				rafRef.current = requestAnimationFrame(tick);
			} else {
				fromRef.current = target;
			}
		};
		rafRef.current = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(rafRef.current);
	}, [target, durationMs]);

	return value;
};
