import { useClientDocument } from "@livestore/react";
import { tables } from "./livestore/schema";
import { useBridgeSync } from "./bridge/sync";
import { DndProvider } from "./lib/dnd";
import { Planning } from "./views/Planning";
import { Review } from "./views/Review";
import { ViewErrorBoundary } from "./components/ErrorBoundary";

type View = "review" | "planning";

const VIEWS: { id: View; label: string }[] = [
	{ id: "review", label: "Revisão" },
	{ id: "planning", label: "Planejamento" },
];

/**
 * App shell — a full-width responsive workspace (DESIGN.md "Layout"). The shell
 * caps at `min(1680px, 96vw)` with 24–32px gutters; the views own their own
 * responsive grids. Two views: Revisão (the transaction list + live-sum filters)
 * and Planejamento (the cash-evolution chart spine + the selected month's plan,
 * with drag-and-drop forecast re-dating).
 */
export const App = () => {
	const [{ view }, setUi] = useClientDocument(tables.ui);
	const sync = useBridgeSync();

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
				Pular para conteúdo
			</a>
			<div
				style={{
					maxWidth: "var(--container)",
					margin: "0 auto",
					padding: "0 clamp(24px, 3vw, 32px)",
				}}
			>
				<header
					style={{
						display: "flex",
						alignItems: "center",
						gap: 16,
						padding: "28px 0 20px",
						borderBottom: "1px solid var(--border)",
					}}
				>
					<span className="phi" style={{ fontSize: "2rem" }}>
						φ
					</span>
					<strong
						style={{
							fontFamily: "var(--font-display)",
							fontSize: "1.4rem",
							letterSpacing: "-0.02em",
						}}
					>
						phai
					</strong>
					<nav style={{ display: "flex", gap: 8, marginLeft: "auto" }}>
						{VIEWS.map((v) => (
							<button
								key={v.id}
								onClick={() => setUi({ view: v.id })}
								className="mono"
								style={{
									background:
										view === v.id ? "rgba(109,74,255,0.08)" : "transparent",
									color: view === v.id ? "var(--purple)" : "var(--muted)",
									border: `1px solid ${view === v.id ? "rgba(109,74,255,0.25)" : "var(--border)"}`,
									borderRadius: "var(--radius-full)",
									padding: "6px 18px",
									cursor: "pointer",
									fontSize: 13,
									transition: "border-color 150ms, color 150ms",
								}}
							>
								{v.label}
							</button>
						))}
					</nav>
				</header>

				<nav className="mobile-nav" style={{ display: "none" }}>
					{VIEWS.map((v) => (
						<button
							key={v.id}
							onClick={() => setUi({ view: v.id })}
							className="mono"
							style={{
								background:
									view === v.id ? "rgba(109,74,255,0.08)" : "transparent",
								color: view === v.id ? "var(--purple)" : "var(--muted)",
								border: `1px solid ${view === v.id ? "rgba(109,74,255,0.25)" : "var(--border)"}`,
								borderRadius: "var(--radius-full)",
								padding: "8px 24px",
								cursor: "pointer",
								fontSize: 13,
							}}
						>
							{v.id === "review" ? "☰" : "▤"} {v.label}
						</button>
					))}
				</nav>
				<style>{`
        @media (max-width: 639px) {
          header nav { display: none; }
          .mobile-nav {
            display: flex !important;
            position: fixed; bottom: 0; left: 0; right: 0; z-index: 50;
            background: var(--bg); border-top: 1px solid var(--border);
            padding: 8px 16px; justify-content: center; gap: 12px;
            padding-bottom: calc(8px + env(safe-area-inset-bottom, 0px));
          }
          main { padding-bottom: 100px !important; }
        }
      `}</style>

				<SyncChip
					pending={sync.pending}
					error={sync.error}
					onRetry={sync.retry}
				/>

				<DndProvider>
					<main id="main-content" style={{ padding: "20px 0 80px" }}>
						<ViewErrorBoundary viewName="Revisão">
							{view === "review" && <Review />}
						</ViewErrorBoundary>
						<ViewErrorBoundary viewName="Planejamento">
							{view === "planning" && <Planning />}
						</ViewErrorBoundary>
					</main>
				</DndProvider>
			</div>
		</>
	);
};

const SyncChip = ({
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
		? `sync com erro — ${error}`
		: pending > 0
			? `${pending} pendente${pending === 1 ? "" : "s"} de sync`
			: "sincronizado";
	return (
		<div
			className="mono"
			style={{
				fontSize: 12,
				color,
				padding: "12px 0",
				display: "flex",
				alignItems: "center",
				gap: 8,
			}}
		>
			<span
				style={{ width: 7, height: 7, borderRadius: "50%", background: color }}
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
			{error && onRetry && (
				<button
					onClick={onRetry}
					className="mono"
					style={{
						background: "transparent",
						border: "1px solid currentColor",
						borderRadius: "var(--radius-full)",
						padding: "2px 10px",
						cursor: "pointer",
						fontSize: 11,
						color: "inherit",
					}}
				>
					⟳ tentar novamente
				</button>
			)}
		</div>
	);
};
