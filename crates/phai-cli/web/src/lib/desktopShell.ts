/**
 * True when the app runs inside the native desktop shell (Pake/Tauri WKWebView,
 * ADR-0039) rather than a normal browser tab. The shell hides the macOS title
 * bar, so the traffic-light buttons float over the top-left of the page — the
 * header insets itself when this is true so the logo clears them.
 */
export const isDesktopShell = (): boolean => {
	if (typeof window === "undefined") return false;
	const w = window as unknown as Record<string, unknown>;
	if (w.__TAURI_INTERNALS__ != null || w.__TAURI__ != null || w.isTauri === true) {
		return true;
	}
	return /\bpake\b/i.test(navigator.userAgent);
};
