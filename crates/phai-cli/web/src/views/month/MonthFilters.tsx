import { useCallback } from "react";
import {
	COMMITMENT_TIER_LABELS,
	COMMITMENT_TIERS,
	type CommitmentTier,
} from "../../lib/derivations";
import { formatMoneyNumber } from "../../lib/format";

// ── Filter bar ─────────────────────────────────────────────────────────────

/** Tier accent colours (ADR-0030): locked=neutral, cancellable=amber, variable=green. */
const TIER_COLOR: Record<CommitmentTier, string> = {
	locked: "#9a9aae",
	cancellable: "var(--amber)",
	variable: "var(--green)",
};

const FilterDivider = () => (
	<span
		aria-hidden
		style={{
			width: 1,
			alignSelf: "stretch",
			minHeight: 20,
			background: "var(--border)",
			margin: "0 2px",
		}}
	/>
);

export const FilterBar = ({
	ui,
	textInput,
	setUi,
	onTextInput,
	owners,
	accounts,
	hasFilters,
}: {
	ui: {
		textFilter: string | null;
		accountFilter: string | null;
		ownerFilter: string | null;
		categoryFilter: string | null;
		installmentsOnly: boolean;
		subscriptionsOnly: boolean;
		unreviewedOnly: boolean;
		uncategorizedOnly: boolean;
		tierFilter: string | null;
	};
	textInput: string;
	setUi: (patch: Partial<typeof ui>) => void;
	onTextInput: (v: string) => void;
	owners: string[];
	accounts: ReadonlyArray<{ id: string; label: string; owner: string }>;
	hasFilters: boolean;
}) => {
	const handleTextChange = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) => onTextInput(e.target.value),
		[onTextInput],
	);

	const handleCategoryChange = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) =>
			setUi({ categoryFilter: e.target.value || null }),
		[setUi],
	);

	const handleAccountChange = useCallback(
		(e: React.ChangeEvent<HTMLSelectElement>) =>
			setUi({ accountFilter: e.target.value || null }),
		[setUi],
	);

	const handleOwnerChange = useCallback(
		(e: React.ChangeEvent<HTMLSelectElement>) =>
			setUi({ ownerFilter: e.target.value || null }),
		[setUi],
	);

	const toggleInstallments = useCallback(
		() => setUi({ installmentsOnly: !ui.installmentsOnly }),
		[setUi, ui.installmentsOnly],
	);

	const toggleSubscriptions = useCallback(
		() => setUi({ subscriptionsOnly: !ui.subscriptionsOnly }),
		[setUi, ui.subscriptionsOnly],
	);

	const toggleUnreviewed = useCallback(
		() => setUi({ unreviewedOnly: !ui.unreviewedOnly }),
		[setUi, ui.unreviewedOnly],
	);

	const toggleUncategorized = useCallback(
		() => setUi({ uncategorizedOnly: !ui.uncategorizedOnly }),
		[setUi, ui.uncategorizedOnly],
	);

	const setTier = useCallback(
		(tier: CommitmentTier) =>
			setUi({ tierFilter: ui.tierFilter === tier ? null : tier }),
		[setUi, ui.tierFilter],
	);

	const clearFilters = useCallback(
		() =>
			setUi({
				textFilter: null,
				categoryFilter: null,
				accountFilter: null,
				ownerFilter: null,
				installmentsOnly: false,
				subscriptionsOnly: false,
				unreviewedOnly: false,
				uncategorizedOnly: false,
				tierFilter: null,
			}),
		[setUi],
	);

	return (
		<div
			style={{
				display: "flex",
				flexWrap: "wrap",
				gap: 8,
				alignItems: "center",
				padding: "12px 0 8px",
			}}
		>
			{/* Text search */}
			<div style={{ position: "relative", flexGrow: 1, maxWidth: 260 }}>
				<span
					style={{
						position: "absolute",
						left: 9,
						top: "50%",
						transform: "translateY(-50%)",
						color: "var(--muted2)",
						fontSize: 12,
						pointerEvents: "none",
					}}
				>
					⌕
				</span>
				<input
					placeholder="search transactions…"
					value={textInput}
					onChange={handleTextChange}
					className="mono"
					style={{ ...inputStyle, paddingLeft: 26, width: "100%" }}
					aria-label="text search"
				/>
			</div>

			<FilterDivider />

			{/* Structural filters */}
			<input
				list="phai-cats"
				placeholder="category…"
				value={ui.categoryFilter ?? ""}
				onChange={handleCategoryChange}
				className="mono"
				style={{ ...inputStyle, color: "var(--cyan)", width: 150 }}
				aria-label="filter by category"
			/>

			{/* Account filter */}
			{accounts.length > 0 && (
				<select
					value={ui.accountFilter ?? ""}
					onChange={handleAccountChange}
					className="mono"
					style={selectStyle}
					aria-label="account"
				>
					<option value="">all · accounts</option>
					{accounts.map((a) => (
						<option key={a.id} value={a.id}>
							{a.label || a.id}
						</option>
					))}
				</select>
			)}

			{/* Owner filter */}
			{owners.length > 1 && (
				<select
					value={ui.ownerFilter ?? ""}
					onChange={handleOwnerChange}
					className="mono"
					style={selectStyle}
					aria-label="owner"
				>
					<option value="">all · owners</option>
					{owners.map((o) => (
						<option key={o} value={o}>
							{o}
						</option>
					))}
				</select>
			)}

			<FilterDivider />

			{/* Quick action chips */}
			<ToggleBtn
				active={ui.installmentsOnly}
				color="var(--amber)"
				onClick={toggleInstallments}
			>
				installments
			</ToggleBtn>
			<ToggleBtn
				active={ui.subscriptionsOnly}
				color="var(--cyan)"
				onClick={toggleSubscriptions}
			>
				subscriptions
			</ToggleBtn>
			<ToggleBtn
				active={ui.uncategorizedOnly}
				color="var(--rose)"
				onClick={toggleUncategorized}
			>
				uncategorized
			</ToggleBtn>
			<ToggleBtn
				active={ui.unreviewedOnly}
				color="var(--purple)"
				onClick={toggleUnreviewed}
			>
				unreviewed
			</ToggleBtn>

			<FilterDivider />

			{/* Controllability tiers (ADR-0030) — single-select */}
			{COMMITMENT_TIERS.map((tier) => (
				<ToggleBtn
					key={tier}
					active={ui.tierFilter === tier}
					color={TIER_COLOR[tier]}
					onClick={() => setTier(tier)}
				>
					{COMMITMENT_TIER_LABELS[tier]}
				</ToggleBtn>
			))}

			{/* Clear filters */}
			{hasFilters && (
				<button
					onClick={clearFilters}
					className="mono"
					style={{
						...pillStyle,
						color: "var(--rose)",
						borderColor: "var(--rose)",
					}}
				>
					× clear
				</button>
			)}
		</div>
	);
};

export const FilterSummary = ({
	count,
	saidas,
	entradas,
	selectedCount,
}: {
	count: number;
	saidas: number;
	entradas: number;
	selectedCount?: number;
}) => (
	<div
		className="mono"
		style={{
			fontSize: 11,
			color: "var(--muted)",
			padding: "6px 0 8px",
			display: "flex",
			gap: 14,
			flexWrap: "wrap",
		}}
	>
		<span>
			{count} transaction{count !== 1 ? "s" : ""}
		</span>
		{selectedCount != null && selectedCount > 0 && (
			<span style={{ color: "var(--purple)" }}>
				{selectedCount} selected
			</span>
		)}
		{saidas > 0 && (
			<span style={{ color: "var(--rose)" }}>
				expenses {formatMoneyNumber(-saidas)}
			</span>
		)}
		{entradas > 0 && (
			<span style={{ color: "var(--cyan)" }}>
				income {formatMoneyNumber(entradas)}
			</span>
		)}
		{(saidas > 0 || entradas > 0) && (
			<span
				style={{
					color: entradas - saidas >= 0 ? "var(--green)" : "var(--rose)",
				}}
			>
				net {entradas - saidas >= 0 ? "+" : ""}
				{formatMoneyNumber(entradas - saidas)}
			</span>
		)}
	</div>
);


const inputStyle: React.CSSProperties = {
	background: "var(--bg)",
	color: "var(--white)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "5px 9px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
	outline: "none",
};

const pillStyle: React.CSSProperties = {
	background: "transparent",
	color: "var(--muted)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "4px 12px",
	cursor: "pointer",
	fontSize: 11,
	fontFamily: "var(--font-mono)",
};

const selectStyle: React.CSSProperties = {
	...inputStyle,
	cursor: "pointer",
	paddingRight: 6,
};

export const ToggleBtn = ({
	active,
	color,
	onClick,
	children,
}: {
	active: boolean;
	color: string;
	onClick: () => void;
	children: React.ReactNode;
}) => (
	<button
		onClick={onClick}
		className="mono"
		style={{
			...pillStyle,
			color: active ? color : "var(--muted)",
			border: `1px solid ${active ? color : "var(--border)"}`,
			background: active ? `${color}14` : "transparent",
		}}
	>
		{children}
	</button>
);
