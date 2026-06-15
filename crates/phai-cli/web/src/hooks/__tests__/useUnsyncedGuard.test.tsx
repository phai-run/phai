/**
 * The unsynced-close guard: while writes are queued, closing the tab must arm
 * the browser's beforeunload prompt; once synced, navigation is never blocked.
 */
import { describe, it, expect, afterEach } from "vitest";
import { renderHook, cleanup } from "@testing-library/react";
import { useUnsyncedGuard } from "../useUnsyncedGuard";

afterEach(cleanup);

const fireBeforeUnload = (): Event => {
	const event = new Event("beforeunload", { cancelable: true });
	window.dispatchEvent(event);
	return event;
};

describe("useUnsyncedGuard", () => {
	it("arms the prompt when writes are pending", () => {
		renderHook(() => useUnsyncedGuard(3));
		expect(fireBeforeUnload().defaultPrevented).toBe(true);
	});

	it("does not block navigation when synced", () => {
		renderHook(() => useUnsyncedGuard(0));
		expect(fireBeforeUnload().defaultPrevented).toBe(false);
	});

	it("disarms when the pending count drops to zero", () => {
		const { rerender } = renderHook(({ n }) => useUnsyncedGuard(n), {
			initialProps: { n: 2 },
		});
		expect(fireBeforeUnload().defaultPrevented).toBe(true);
		rerender({ n: 0 });
		expect(fireBeforeUnload().defaultPrevented).toBe(false);
	});

	it("removes the listener on unmount", () => {
		const { unmount } = renderHook(() => useUnsyncedGuard(1));
		unmount();
		expect(fireBeforeUnload().defaultPrevented).toBe(false);
	});
});
