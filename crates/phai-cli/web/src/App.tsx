import { useCallback, useEffect, useState } from "react";
import { useClientDocument } from "@livestore/react";
import { tables } from "./livestore/schema";
import { useBridgeSync } from "./bridge/sync";
import { useUnsyncedGuard } from "./hooks/useUnsyncedGuard";
import { useUpdateCheck } from "./hooks/useUpdateCheck";
import { DndProvider } from "./lib/dnd";
import { Dashboard } from "./views/Dashboard";
import { ViewErrorBoundary } from "./components/ErrorBoundary";
import { PluggySyncButton } from "./components/PluggySyncButton";
import { UpdateBanner } from "./components/UpdateBanner";
import { SearchPalette } from "./components/SearchPalette";

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
	const update = useUpdateCheck();
	const [searchOpen, setSearchOpen] = useState(false);

	// Global keyboard shortcuts: Cmd/Ctrl+K for search
	useEffect(() => {
		const onKeyDown = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key === "k") {
				e.preventDefault();
				setSearchOpen((v) => !v);
			}
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, []);

	const closeSearch = useCallback(() => setSearchOpen(false), []);

	return (
		<>
			<UpdateBanner update={update} />

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
					position: "sticky",
					top: 0,
					zIndex: 40,
					background: "color-mix(in srgb, var(--bg) 88%, transparent)",
					backdropFilter: "blur(8px)",
					borderBottom: "1px solid var(--border)",
				}}
			>
				<div
					style={{
						maxWidth: "var(--container)",
						margin: "0 auto",
						padding: "0 clamp(24px, 3vw, 32px)",
						display: "flex",
						alignItems: "center",
						gap: 16,
						height: 56,
					}}
				>
					<span className="phi" style={{ fontSize: "1.6rem" }}>
						φ
					</span>
					<strong
						style={{
							fontFamily: "var(--font-display)",
							fontSize: "1.2rem",
							letterSpacing: "-0.02em",
						}}
					>
						phai
					</strong>

					{/* Right cluster: search · sync · version — the page's top controls. */}
					<div
						style={{
							marginLeft: "auto",
							display: "flex",
							alignItems: "center",
							gap: 12,
						}}
					>
						<button
							type="button"
							onClick={() => setSearchOpen(true)}
							className="mono"
							title="Buscar transações (Cmd+K)"
							style={{
								display: "flex",
								alignItems: "center",
								gap: 6,
								background: "var(--surface)",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-full)",
								padding: "5px 12px",
								cursor: "pointer",
								color: "var(--muted)",
								fontSize: 12,
								transition: "border-color 150ms",
							}}
						>
							<span style={{ fontSize: 13 }}>/</span>
							Buscar…
							<kbd
								style={{
									fontSize: 10,
									background: "var(--bg)",
									border: "1px solid var(--border)",
									borderRadius: 4,
									padding: "1px 5px",
									marginLeft: 4,
								}}
							>
								{"⌘"}K
							</kbd>
						</button>
						<span aria-hidden style={{ width: 1, height: 20, background: "var(--border)" }} />
						<PluggySyncButton />
						<SyncChip pending={sync.pending} error={sync.error} onRetry={sync.retry} />
						{update.currentVersion && (
							<span
								className="mono"
								title="versão em execução"
								style={{
									fontSize: 11,
									color: update.updateAvailable ? "var(--purple)" : "var(--muted2)",
								}}
							>
								v{update.currentVersion}
							</span>
						)}
					</div>
				</div>
			</header>

			<DndProvider>
				<main id="main-content">
					<ViewErrorBoundary viewName="Dashboard">
						<Dashboard />
					</ViewErrorBoundary>
				</main>
			</DndProvider>

			<SearchPalette open={searchOpen} onClose={closeSearch} />
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
				className={pending > 0 && !error ? "sync-dot-pulse" : undefined}
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
