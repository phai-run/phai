import type { UpdateCheck } from "../hooks/useUpdateCheck";

export const UpdateBanner = ({ update }: { update: UpdateCheck }) => {
	if (update.state === "restarting") {
		return (
			<div style={overlayStyle}>
				<div style={overlayCardStyle}>
					<span style={{ fontSize: 22 }}>&#x21bb;</span>
					<span>Reiniciando...</span>
				</div>
			</div>
		);
	}

	if (!update.updateAvailable && update.state !== "error") return null;

	return (
		<div style={bannerStyle}>
			{update.state === "error" && update.error ? (
				<span style={{ color: "var(--red, #c00)" }}>{update.error}</span>
			) : update.state === "updating" ? (
				<span>Atualizando...</span>
			) : (
				<>
					<span>
						v{update.currentVersion} &rarr; v{update.latestVersion}{" "}
						disponível
					</span>
					<button
						type="button"
						onClick={update.applyUpdate}
						style={buttonStyle}
					>
						Atualizar agora
					</button>
				</>
			)}
		</div>
	);
};

const bannerStyle: React.CSSProperties = {
	position: "fixed",
	bottom: 16,
	right: 16,
	zIndex: 100,
	display: "flex",
	alignItems: "center",
	gap: 10,
	background: "var(--bg, #fff)",
	border: "1px solid var(--purple, #7c3aed)",
	borderRadius: "var(--radius-full, 20px)",
	padding: "6px 16px",
	boxShadow: "0 4px 16px rgba(0,0,0,0.15)",
	fontSize: 13,
	fontFamily: "var(--font-mono, monospace)",
};

const buttonStyle: React.CSSProperties = {
	background: "var(--purple, #7c3aed)",
	color: "#fff",
	border: "none",
	borderRadius: "var(--radius-sm, 6px)",
	padding: "4px 12px",
	fontSize: 12,
	cursor: "pointer",
	fontFamily: "inherit",
};

const overlayStyle: React.CSSProperties = {
	position: "fixed",
	inset: 0,
	zIndex: 200,
	display: "flex",
	alignItems: "center",
	justifyContent: "center",
	background: "rgba(0,0,0,0.4)",
	backdropFilter: "blur(4px)",
};

const overlayCardStyle: React.CSSProperties = {
	display: "flex",
	alignItems: "center",
	gap: 12,
	background: "var(--bg, #fff)",
	borderRadius: "var(--radius-md, 12px)",
	padding: "24px 40px",
	fontSize: 18,
	fontFamily: "var(--font-mono, monospace)",
	boxShadow: "0 8px 32px rgba(0,0,0,0.2)",
};
