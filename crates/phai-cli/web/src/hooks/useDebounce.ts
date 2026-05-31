import { useEffect, useRef, useState } from "react";

/**
 * Debounce a value by `delayMs` milliseconds.
 *
 * Returns the debounced value, which only updates after the input stops
 * changing for `delayMs`. The internal timer ref is cleaned up on unmount.
 */
export const useDebounce = <T>(value: T, delayMs: number): T => {
	const [debounced, setDebounced] = useState(value);
	const timer = useRef<ReturnType<typeof setTimeout>>(null);

	useEffect(() => {
		timer.current = setTimeout(() => setDebounced(value), delayMs);
		return () => {
			if (timer.current !== null) clearTimeout(timer.current);
		};
	}, [value, delayMs]);

	return debounced;
};
