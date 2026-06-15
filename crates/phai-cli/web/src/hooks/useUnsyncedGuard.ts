import { useEffect } from "react";

/**
 * Warn before the tab closes/reloads while writes are still queued for the
 * bridge.
 *
 * Queued writes live in LiveStore's OPFS store, so they are NOT lost when the
 * tab closes — they flush on the next open. But they have not yet reached the
 * system of record (BigQuery), so closing now and reopening on another machine
 * (or after a cache clear / fresh install) would silently miss them. While
 * `pendingCount > 0` we arm the browser's native "leave site?" dialog; once
 * everything is synced the guard disarms so normal navigation is never blocked.
 */
export const useUnsyncedGuard = (pendingCount: number): void => {
	useEffect(() => {
		if (pendingCount <= 0) return;
		const handler = (event: BeforeUnloadEvent) => {
			event.preventDefault();
			// Legacy browsers only show the prompt when returnValue is set.
			event.returnValue = "";
		};
		window.addEventListener("beforeunload", handler);
		return () => window.removeEventListener("beforeunload", handler);
	}, [pendingCount]);
};
