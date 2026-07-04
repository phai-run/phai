import type { CardRow } from "../../bridge/api";
import { formatMoneyNumber, numeric } from "../../lib/format";

/**
 * A skeuomorphic Nubank Ultravioleta-style credit card that flips (CSS 3D,
 * .cc-flip) to its back when selected. Front: chip, contactless glyph, a masked
 * number, valid-thru and the Nubank wordmark on the dark iridescent-purple
 * plastic. Back: magnetic stripe, signature panel with a masked CVC, and the
 * cycle's key figures. Details not in the data (last four digits, the sheen
 * angle) are *deterministically* derived from the accountId, so a card always
 * looks the same — nothing real or sensitive is shown (digits are decorative).
 */

/** Stable non-negative hash of a string (decorative seed only). */
const seed = (s: string): number => {
	let h = 0;
	for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
	return h;
};

// The Nubank Ultravioleta plastic: near-black base with a deep violet rise. A
// per-card hue nudge (from the id) keeps two cards subtly distinguishable
// without leaving the ultravioleta family.
const cardGradient = (h: number): string => {
	const shift = h % 30; // 0..29° of hue variation
	return `linear-gradient(150deg, #141019 0%, #201636 42%, hsl(${262 + shift}, 55%, 26%) 100%)`;
};

/** "YYYY-MM(-DD)" → "MM/YY" for the valid-thru line. */
const validThru = (card: CardRow): string => {
	const src = card.dueDate ?? card.cycleMonth;
	if (!src) return "--/--";
	const mm = src.slice(5, 7);
	const yy = src.slice(2, 4);
	return `${mm}/${yy}`;
};

/** The Nubank wordmark, rendered as styled text (no shipped logo asset). */
const NubankMark = () => (
	<span style={{ display: "inline-flex", flexDirection: "column", alignItems: "flex-end", lineHeight: 1 }}>
		<span
			style={{
				fontFamily: "var(--font-display)",
				fontWeight: 800,
				fontSize: 15,
				letterSpacing: "-0.01em",
				color: "#fff",
				textShadow: "0 1px 2px rgba(0,0,0,0.35)",
			}}
		>
			nubank
		</span>
		<span
			className="mono"
			style={{ fontSize: 7, letterSpacing: "0.32em", color: "#c4b5fd", marginTop: 1 }}
		>
			ULTRAVIOLETA
		</span>
	</span>
);

export const SkeuomorphicCard = ({
	card,
	flipped,
	onToggle,
}: {
	card: CardRow;
	flipped: boolean;
	onToggle: () => void;
}) => {
	const h = seed(card.accountId);
	const gradient = cardGradient(h);
	const last4 = String(h % 10000).padStart(4, "0");
	const total = numeric(card.total);
	const limit = card.creditLimit != null ? numeric(card.creditLimit) : null;
	const used = card.usedAmount != null ? numeric(card.usedAmount) : null;
	const usedPct =
		limit && limit > 0 && used != null
			? Math.min(100, Math.round((used / limit) * 100))
			: null;

	const stateLabel =
		card.state === "aberta" ? "ABERTA" : card.state === "fechada" ? "FECHADA" : "EM DIA";
	const stateColor =
		card.state === "aberta" ? "#fbbf24" : card.state === "fechada" ? "#c4b5fd" : "#86efac";

	const faceBase: React.CSSProperties = {
		background: gradient,
		color: "#fff",
		padding: 18,
		display: "flex",
		flexDirection: "column",
	};

	return (
		<div className="cc-scene">
			<div
				className={`cc-flip${flipped ? " is-flipped" : ""}`}
				role="button"
				tabIndex={0}
				aria-pressed={flipped}
				aria-label={`${card.label} — ${flipped ? "ver frente" : "virar e ver detalhes"}`}
				onClick={onToggle}
				onKeyDown={(e) => {
					if (e.key === "Enter" || e.key === " ") {
						e.preventDefault();
						onToggle();
					}
				}}
			>
				{/* ── FRONT ── */}
				<div className="cc-face" style={faceBase}>
					{/* sheen overlay */}
					<div
						aria-hidden
						style={{
							position: "absolute",
							inset: 0,
							background:
								"radial-gradient(120% 80% at 15% 10%, rgba(255,255,255,0.22), transparent 55%)",
							pointerEvents: "none",
						}}
					/>
					<div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", zIndex: 1 }}>
						<span
							style={{
								fontFamily: "var(--font-display)",
								fontWeight: 700,
								fontSize: 14,
								letterSpacing: "0.01em",
								maxWidth: "62%",
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
								textShadow: "0 1px 2px rgba(0,0,0,0.3)",
							}}
						>
							{card.label}
						</span>
						<span
							className="mono"
							style={{
								fontSize: 9,
								fontWeight: 700,
								color: "#1a1a1a",
								background: stateColor,
								borderRadius: "var(--radius-full)",
								padding: "2px 8px",
							}}
						>
							{stateLabel}
						</span>
					</div>

					{/* chip + contactless */}
					<div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 14, zIndex: 1 }}>
						<span className="cc-chip" style={{ width: 38, height: 28 }} aria-hidden />
						<span aria-hidden style={{ fontSize: 18, opacity: 0.85, transform: "rotate(90deg)" }}>
							)))
						</span>
					</div>

					{/* number */}
					<div
						className="mono"
						style={{
							marginTop: "auto",
							fontSize: "clamp(13px, 3.4vw, 17px)",
							letterSpacing: "0.14em",
							textShadow: "0 1px 2px rgba(0,0,0,0.35)",
							zIndex: 1,
						}}
					>
						•••• •••• •••• {last4}
					</div>

					{/* holder + valid thru + network */}
					<div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-end", marginTop: 10, zIndex: 1 }}>
						<div style={{ minWidth: 0 }}>
							<div style={{ fontSize: 8, opacity: 0.7, letterSpacing: "0.14em" }}>VÁLIDO ATÉ</div>
							<div className="mono" style={{ fontSize: 12 }}>
								{validThru(card)}
							</div>
						</div>
						<div className="mono" style={{ fontSize: 11, opacity: 0.9, textAlign: "right" }}>
							<div style={{ fontSize: 8, opacity: 0.7, letterSpacing: "0.14em" }}>FATURA</div>
							<div style={{ fontWeight: 700 }}>{formatMoneyNumber(total)}</div>
						</div>
						<NubankMark />
					</div>
				</div>

				{/* ── BACK ── */}
				<div className="cc-face cc-face-back" style={{ ...faceBase, padding: 0 }}>
					{/* magnetic stripe */}
					<div aria-hidden style={{ height: 40, background: "#111", marginTop: 16 }} />
					{/* signature + CVC */}
					<div style={{ padding: "12px 18px 0", zIndex: 1 }}>
						<div style={{ display: "flex", alignItems: "center", gap: 8 }}>
							<div
								style={{
									flex: 1,
									height: 26,
									background: "repeating-linear-gradient(45deg, #f5f5f5, #f5f5f5 6px, #e5e5e5 6px, #e5e5e5 12px)",
									borderRadius: 3,
								}}
							/>
							<div
								className="mono"
								style={{
									background: "#fff",
									color: "#111",
									borderRadius: 3,
									padding: "4px 8px",
									fontSize: 11,
									fontWeight: 700,
								}}
							>
								{String(h % 1000).padStart(3, "0")}
							</div>
						</div>
						{/* figures */}
						<div style={{ display: "flex", gap: 16, marginTop: 14 }}>
							<BackFigure label="fatura" value={formatMoneyNumber(total)} />
							{card.installmentCount > 0 && (
								<BackFigure label="parcelas" value={String(card.installmentCount)} />
							)}
							{usedPct != null && <BackFigure label="limite" value={`${usedPct}%`} />}
						</div>
						{usedPct != null && (
							<div
								aria-hidden
								style={{
									marginTop: 10,
									height: 5,
									borderRadius: "var(--radius-full)",
									background: "rgba(255,255,255,0.25)",
									overflow: "hidden",
								}}
							>
								<div
									style={{
										width: `${usedPct}%`,
										height: "100%",
										background: usedPct >= 90 ? "#f87171" : "#fff",
									}}
								/>
							</div>
						)}
						<div className="mono" style={{ fontSize: 9.5, opacity: 0.7, marginTop: 12 }}>
							clique para voltar · detalhes abaixo ↓
						</div>
					</div>
				</div>
			</div>
		</div>
	);
};

const BackFigure = ({ label, value }: { label: string; value: string }) => (
	<div>
		<div style={{ fontSize: 8, opacity: 0.7, letterSpacing: "0.14em", textTransform: "uppercase" }}>
			{label}
		</div>
		<div className="mono" style={{ fontSize: 13, fontWeight: 700 }}>
			{value}
		</div>
	</div>
);
