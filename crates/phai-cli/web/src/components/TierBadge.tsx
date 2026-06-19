import {
	COMMITMENT_TIER_LABELS,
	type CommitmentTier,
} from "../lib/derivations";

/**
 * Controllability tier visual language (ADR-0030), shared across the sheet,
 * the categories treemap, and the planning workspace so a tier reads the same
 * everywhere. The palette matches the sheet's tier filter chips.
 */
export const TIER_COLOR: Record<CommitmentTier, string> = {
	locked: "#9a9aae",
	cancellable: "var(--amber)",
	variable: "var(--green)",
};

export const TIER_ICON: Record<CommitmentTier, string> = {
	locked: "🔒",
	cancellable: "↻",
	variable: "〜",
};

/**
 * A small pill marking a transaction's commitment tier. `compact` drops the
 * label (icon only) for dense rows; the full label is always in the tooltip.
 */
export const TierBadge = ({
	tier,
	compact = false,
}: {
	tier: CommitmentTier;
	compact?: boolean;
}) => {
	const color = TIER_COLOR[tier];
	const label = COMMITMENT_TIER_LABELS[tier];
	return (
		<span
			className="mono"
			title={`tier: ${label}`}
			style={{
				display: "inline-flex",
				alignItems: "center",
				gap: 4,
				padding: compact ? "0 5px" : "1px 7px",
				borderRadius: "var(--radius-full)",
				fontSize: 10,
				fontWeight: 600,
				color,
				background: `color-mix(in srgb, ${color} 14%, transparent)`,
				border: `1px solid color-mix(in srgb, ${color} 32%, transparent)`,
				whiteSpace: "nowrap",
				lineHeight: 1.5,
			}}
		>
			<span aria-hidden>{TIER_ICON[tier]}</span>
			{!compact && label}
		</span>
	);
};
