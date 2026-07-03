import type { CardRow } from "../../bridge/api";
import { formatMoneyNumber, numeric } from "../../lib/format";

/**
 * A skeuomorphic credit-card face that flips (CSS 3D, .cc-flip) to its back when
 * selected. Front: chip, contactless glyph, a masked number, holder + valid-thru
 * and a network mark. Back: magnetic stripe, signature panel with a masked CVC,
 * and the cycle's key figures. Every visual detail that isn't in the data (last
 * four digits, the gradient, the network guess) is *deterministically* derived
 * from the accountId, so a given card always looks the same — but nothing real
 * or sensitive is ever shown (the digits are decorative, seeded from the id).
 */

/** Stable non-negative hash of a string (decorative seed only). */
const seed = (s: string): number => {
	let h = 0;
	for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
	return h;
};

// A spread of tasteful card gradients; the accountId picks one deterministically
// so different cards read as visually distinct plastic.
const GRADIENTS: ReadonlyArray<string> = [
	"linear-gradient(135deg, #1f2937 0%, #4b5563 100%)", // graphite
	"linear-gradient(135deg, #4c1d95 0%, #7c3aed 100%)", // violet
	"linear-gradient(135deg, #0f766e 0%, #14b8a6 100%)", // teal
	"linear-gradient(135deg, #7c2d12 0%, #b45309 100%)", // copper
	"linear-gradient(135deg, #831843 0%, #be185d 100%)", // magenta
	"linear-gradient(135deg, #1e3a8a 0%, #2563eb 100%)", // sapphire
	"linear-gradient(135deg, #052e16 0%, #15803d 100%)", // forest
	"linear-gradient(135deg, #18181b 0%, #3f3f46 100%)", // onyx
];

type Network = "visa" | "mastercard" | "elo" | "amex" | "generic";

const guessNetwork = (label: string): Network => {
	const l = label.toLowerCase();
	if (l.includes("visa")) return "visa";
	if (l.includes("master") || l.includes("nubank") || l.includes("nu ")) return "mastercard";
	if (l.includes("elo")) return "elo";
	if (l.includes("amex") || l.includes("american")) return "amex";
	return "generic";
};

/** "YYYY-MM(-DD)" → "MM/YY" for the valid-thru line. */
const validThru = (card: CardRow): string => {
	const src = card.dueDate ?? card.cycleMonth;
	if (!src) return "--/--";
	const mm = src.slice(5, 7);
	const yy = src.slice(2, 4);
	return `${mm}/${yy}`;
};

const NetworkMark = ({ network }: { network: Network }) => {
	if (network === "mastercard") {
		return (
			<span aria-label="mastercard" style={{ display: "inline-flex", alignItems: "center" }}>
				<span style={{ width: 22, height: 22, borderRadius: "50%", background: "#eb001b" }} />
				<span
					style={{
						width: 22,
						height: 22,
						borderRadius: "50%",
						background: "#f79e1b",
						marginLeft: -10,
						mixBlendMode: "multiply",
					}}
				/>
			</span>
		);
	}
	const text =
		network === "visa"
			? "VISA"
			: network === "elo"
				? "elo"
				: network === "amex"
					? "AMEX"
					: "•• bank";
	return (
		<span
			style={{
				fontFamily: "var(--font-display)",
				fontStyle: network === "visa" ? "italic" : "normal",
				fontWeight: 800,
				fontSize: 17,
				letterSpacing: network === "visa" ? "0.06em" : "0.02em",
				color: "#fff",
				textShadow: "0 1px 2px rgba(0,0,0,0.3)",
			}}
		>
			{text}
		</span>
	);
};

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
	const gradient = GRADIENTS[h % GRADIENTS.length];
	const network = guessNetwork(card.label);
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
						<NetworkMark network={network} />
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
