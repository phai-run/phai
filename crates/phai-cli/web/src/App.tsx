import { useClientDocument } from "@livestore/react";
import { tables } from "./livestore/schema";
import { useBridgeSync } from "./bridge/sync";
import { useUnsyncedGuard } from "./hooks/useUnsyncedGuard";
import { DndProvider } from "./lib/dnd";
import { Dashboard } from "./views/Dashboard";
import { ViewErrorBoundary } from "./components/ErrorBoundary";

/**
 * App shell — unified full-width workspace. A single Dashboard view replaces
 * the old Revisão/Planejamento split: the chart is always at the top (sticky,
 * compresses on scroll) and the month detail with categorised transactions is
 * below it. The DndProvider wraps everything so forecasts can be dragged from
 * the detail section up to the sticky chart even while scrolled.
 */
export const App = () => {
	const [ui] = useClientDocument(tables.ui);
	void ui; // read to trigger LiveStore hydration
	const sync = useBridgeSync();
	// Warn before closing while writes haven't reached the bridge yet.
	useUnsyncedGuard(sync.pending);

	return (
		<>
			<a
				href="#main-content"
				className="mono"
				style={{
					position: "absolute",
					top: -100,
					left: 8,
					background: "var(--purple)",
					color: "#fff",
					padding: "8px 16px",
					borderRadius: "var(--radius-sm)",
					zIndex: 100,
					transition: "top 150ms",
				}}
				onFocus={(e) => {
					(e.target as HTMLElement).style.top = "8px";
				}}
				onBlur={(e) => {
					(e.target as HTMLElement).style.top = "-100px";
				}}
			>
				Skip to content
			</a>

			<header
				style={{
					maxWidth: "var(--container)",
					margin: "0 auto",
					padding: "0 clamp(24px, 3vw, 32px)",
					display: "flex",
					alignItems: "center",
					gap: 16,
					height: 60,
					borderBottom: "1px solid var(--border)",
				}}
			>
				<span className="phi" style={{ fontSize: "1.75rem" }}>
					φ
				</span>
				<strong
					style={{
						fontFamily: "var(--font-display)",
						fontSize: "1.25rem",
						letterSpacing: "-0.02em",
					}}
				>
					phai
				</strong>
				<div style={{ marginLeft: "auto" }}>
					<SyncChip
						pending={sync.pending}
						error={sync.error}
						onRetry={sync.retry}
					/>
				</div>
			</header>

			<DndProvider>
				<main id="main-content">
					<ViewErrorBoundary viewName="Dashboard">
						<Dashboard />
					</ViewErrorBoundary>
				</main>
			</DndProvider>
		</>
	);
};

export const SyncChip = ({
	pending,
	error,
	onRetry,
}: {
	pending: number;
	error: string | null;
	onRetry?: () => void;
}) => {
	const color = error
		? "var(--rose)"
		: pending > 0
			? "var(--amber)"
			: "var(--green)";
	const label = error
		? `error · ${error}`
		: pending > 0
			? `${pending} pending`
			: "synced";
	const title = error
		? "Sync failed — click to retry"
		: pending > 0
			? `${pending} change(s) still syncing — click to force a sync`
			: "All changes saved — click to re-check";
	return (
		<button
			type="button"
			className="mono"
			onClick={() => onRetry?.()}
			title={title}
			aria-label={title}
			style={{
				fontSize: 11,
				color,
				display: "flex",
				alignItems: "center",
				gap: 6,
				background: "transparent",
				border: "none",
				padding: 0,
				cursor: "pointer",
			}}
		>
			<span
				style={{ width: 6, height: 6, borderRadius: "50%", background: color }}
			/>
			{!error && pending > 0 && (
				<span
					style={{
						display: "inline-block",
						animation: "spin 1s linear infinite",
					}}
				>
					⟳
				</span>
			)}
			{label}
		</button>
	);
};
