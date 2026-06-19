import { useId, useRef, useState } from "react";
import { api } from "../bridge/api";

/**
 * First-run activation screen. A non-technical user attaches the encrypted
 * invite file the owner generated, types the passphrase, and lands in the
 * dashboard — no terminal, no GCP, no config files. Shown by the root gate when
 * `GET /api/status` reports the machine is not activated yet.
 */
export const Onboarding = ({ onActivated }: { onActivated: () => void }) => {
	const [token, setToken] = useState("");
	const [fileName, setFileName] = useState<string | null>(null);
	const [passphrase, setPassphrase] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const fileInput = useRef<HTMLInputElement>(null);
	const passId = useId();

	const readKeyFile = async (file: File) => {
		setError(null);
		const text = (await file.text()).trim();
		if (!text.startsWith("PHAI1E-")) {
			setError("Esse arquivo não parece uma chave do Phai (PHAI1E-…).");
			setToken("");
			setFileName(null);
			return;
		}
		setToken(text);
		setFileName(file.name);
	};

	const canActivate = token.startsWith("PHAI1E-") && passphrase.length > 0 && !busy;

	const activate = async () => {
		if (!canActivate) return;
		setBusy(true);
		setError(null);
		try {
			await api.activate(token, passphrase);
			onActivated();
		} catch (e) {
			setError(e instanceof Error ? e.message : String(e));
			setBusy(false);
		}
	};

	return (
		<div
			style={{
				minHeight: "100vh",
				display: "flex",
				alignItems: "center",
				justifyContent: "center",
				padding: 24,
			}}
		>
			<div
				style={{
					width: "100%",
					maxWidth: 440,
					background: "var(--card)",
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-xl)",
					padding: 32,
					display: "flex",
					flexDirection: "column",
					gap: 20,
				}}
			>
				<div style={{ textAlign: "center", display: "grid", gap: 8 }}>
					<span className="phi" style={{ fontSize: "3.5rem", lineHeight: 1 }}>
						φ
					</span>
					<h1 style={{ margin: 0, fontSize: 22, color: "var(--text)" }}>
						Ativar o Phai
					</h1>
					<p style={{ margin: 0, color: "var(--muted)", fontSize: 14 }}>
						Anexe a chave que você recebeu e digite a senha. O resto é
						automático.
					</p>
				</div>

				{/* Step 1 — attach the key file */}
				<div style={{ display: "grid", gap: 8 }}>
					<input
						ref={fileInput}
						type="file"
						accept=".phai,.txt,text/plain"
						style={{ display: "none" }}
						onChange={(e) => {
							const file = e.target.files?.[0];
							if (file) void readKeyFile(file);
						}}
					/>
					<button
						className="mono"
						type="button"
						onClick={() => fileInput.current?.click()}
						style={{
							padding: "14px 16px",
							borderRadius: "var(--radius-md)",
							border: `1px ${fileName ? "solid" : "dashed"} var(--border)`,
							background: "var(--surface)",
							color: fileName ? "var(--text)" : "var(--muted)",
							cursor: "pointer",
							textAlign: "left",
							fontSize: 13,
						}}
					>
						{fileName ? `🔑 ${fileName}` : "📎 Anexar arquivo da chave (.phai)"}
					</button>
				</div>

				{/* Step 2 — passphrase */}
				<div style={{ display: "grid", gap: 6 }}>
					<label
						htmlFor={passId}
						className="mono"
						style={{ fontSize: 12, color: "var(--muted)" }}
					>
						Senha da chave
					</label>
					<input
						id={passId}
						type="password"
						value={passphrase}
						autoComplete="off"
						placeholder="••••••••"
						onChange={(e) => setPassphrase(e.target.value)}
						onKeyDown={(e) => {
							if (e.key === "Enter") void activate();
						}}
						style={{
							padding: "12px 14px",
							borderRadius: "var(--radius-md)",
							border: "1px solid var(--border)",
							background: "var(--surface)",
							color: "var(--text)",
							fontSize: 14,
						}}
					/>
				</div>

				{error && (
					<div
						role="alert"
						className="mono"
						style={{
							fontSize: 12,
							color: "var(--rose)",
							background: "color-mix(in srgb, var(--rose) 12%, transparent)",
							border: "1px solid var(--rose)",
							borderRadius: "var(--radius-md)",
							padding: "10px 12px",
						}}
					>
						{error}
					</div>
				)}

				<button
					type="button"
					onClick={() => void activate()}
					disabled={!canActivate}
					style={{
						padding: "14px 16px",
						borderRadius: "var(--radius-md)",
						border: "none",
						background: canActivate ? "var(--green)" : "var(--border)",
						color: canActivate ? "var(--white)" : "var(--muted)",
						fontWeight: 700,
						fontSize: 15,
						cursor: canActivate ? "pointer" : "not-allowed",
					}}
				>
					{busy ? "Ativando…" : "Ativar"}
				</button>

				<p
					className="mono"
					style={{
						margin: 0,
						fontSize: 11,
						color: "var(--muted)",
						textAlign: "center",
					}}
				>
					A chave fica só neste computador. Nada é enviado para a internet além
					do seu próprio banco de dados.
				</p>
			</div>
		</div>
	);
};
