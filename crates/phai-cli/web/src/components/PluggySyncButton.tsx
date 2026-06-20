import { useEffect, useState } from "react";
import { api, type SyncResult } from "../bridge/api";

/**
 * Header button that pulls fresh transactions from Pluggy (runs the CLI sync via
 * the bridge) and shows what came in. Renders nothing until the bridge reports a
 * configured Pluggy sync (`syncAvailable`).
 */
export const PluggySyncButton = () => {
	const [available, setAvailable] = useState(false);
	const [busy, setBusy] = useState(false);
	const [result, setResult] = useState<SyncResult | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let live = true;
		api
			.status()
			.then((s) => live && setAvailable(s.syncAvailable))
			.catch(() => {});
		return () => {
			live = false;
		};
	}, []);

	if (!available) return null;

	const runSync = async () => {
		if (busy) return;
		setBusy(true);
		setError(null);
		setResult(null);
		try {
			setResult(await api.sync());
		} catch (e) {
			setError(e instanceof Error ? e.message : String(e));
		} finally {
			setBusy(false);
		}
	};

	const count = result?.new_transactions_count ?? 0;
	const items = result?.new_transactions ?? [];

	return (
		<>
			<button
				className="mono"
				type="button"
				onClick={() => void runSync()}
				disabled={busy}
				title="puxar transações novas da Pluggy"
				style={{
					display: "inline-flex",
					alignItems: "center",
					gap: 5,
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-full)",
					background: "var(--card)",
					color: busy ? "var(--muted)" : "var(--text)",
					fontSize: 11,
					padding: "3px 10px",
					cursor: busy ? "default" : "pointer",
				}}
			>
				<span
					aria-hidden
					style={{
						display: "inline-block",
						animation: busy ? "spin 0.9s linear infinite" : "none",
					}}
				>
					↻
				</span>
				{busy ? "sincronizando…" : "sync"}
			</button>

			{(result || error) && (
				<div
					role="dialog"
					aria-label="resultado da sincronização"
					style={{
						position: "fixed",
						top: 48,
						right: 12,
						zIndex: 100,
						width: 320,
						maxHeight: "70vh",
						overflowY: "auto",
						background: "var(--card)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-lg)",
						boxShadow: "0 12px 40px rgba(0,0,0,0.22)",
						padding: 16,
					}}
				>
					<div
						style={{
							display: "flex",
							justifyContent: "space-between",
							alignItems: "center",
							marginBottom: 10,
						}}
					>
						<strong style={{ fontSize: 14 }}>
							{error
								? "Falha na sincronização"
								: count > 0
									? `${count} nova${count === 1 ? "" : "s"}`
									: "Tudo em dia"}
						</strong>
						<button
							type="button"
							onClick={() => {
								setResult(null);
								setError(null);
							}}
							aria-label="fechar"
							style={{
								background: "transparent",
								border: "none",
								cursor: "pointer",
								color: "var(--muted)",
								fontSize: 16,
							}}
						>
							×
						</button>
					</div>

					{error ? (
						<div
							className="mono"
							style={{ fontSize: 12, color: "var(--rose)", lineHeight: 1.4 }}
						>
							{error}
						</div>
					) : count === 0 ? (
						<div
							className="mono"
							style={{ fontSize: 12, color: "var(--muted)" }}
						>
							Nenhuma transação nova da Pluggy.
						</div>
					) : (
						<>
							<div style={{ display: "grid", gap: 6, marginBottom: 12 }}>
								{items.slice(0, 12).map((t, i) => (
									<div
										key={i}
										className="mono"
										style={{
											display: "flex",
											justifyContent: "space-between",
											gap: 10,
											fontSize: 11,
										}}
									>
										<span
											style={{
												overflow: "hidden",
												textOverflow: "ellipsis",
												whiteSpace: "nowrap",
												color: "var(--text)",
											}}
										>
											{t.description ?? "—"}
										</span>
										<span style={{ color: "var(--muted)", flexShrink: 0 }}>
											{t.amount ?? ""}
										</span>
									</div>
								))}
								{items.length > 12 && (
									<span
										className="mono"
										style={{ fontSize: 11, color: "var(--muted)" }}
									>
										+{items.length - 12} mais…
									</span>
								)}
							</div>
							<button
								type="button"
								onClick={() => window.location.reload()}
								className="mono"
								style={{
									width: "100%",
									padding: "9px 12px",
									borderRadius: "var(--radius-md)",
									border: "none",
									background: "var(--green)",
									color: "var(--bg)",
									fontWeight: 700,
									fontSize: 13,
									cursor: "pointer",
								}}
							>
								Recarregar e revisar
							</button>
						</>
					)}
				</div>
			)}
		</>
	);
};
