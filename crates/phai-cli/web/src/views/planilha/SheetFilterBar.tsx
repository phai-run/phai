import React, { useEffect, useMemo, useRef, useState } from "react";
import { formatMoneyNumber } from "../../lib/format";
import {
	accountFilterIds,
	COMMITMENT_TIER_LABELS,
	COMMITMENT_TIERS,
	DEFAULT_SHEET_LOCAL_FILTERS,
	type CommitmentTier,
	type SheetLocalFilters,
} from "../../lib/derivations";

/**
 * Sheet filter bar — one calm row instead of the old two-storey wall of chips.
 * The everyday controls (search + accounts) stay inline; everything else
 * (origem, tipo, sem-categoria, comprometimento) folds into a single "filtros"
 * popover that carries a badge with the count of active refinements. The filter
 * *state* is unchanged — this is purely a layout/affordance rework, so no
 * STORE_VERSION or persistence change.
 */

const ORIGIN_OPTIONS: Array<{ value: SheetLocalFilters["origin"]; label: string }> = [
	{ value: "all", label: "todas" },
	{ value: "real", label: "realizado" },
	{ value: "installment", label: "parcela" },
	{ value: "recurring", label: "recorrente" },
	{ value: "fixed", label: "fixa" },
	{ value: "manual", label: "previsto manual" },
	{ value: "scenario", label: "cenário" },
];

const FLOW_OPTIONS: Array<{ value: SheetLocalFilters["flow"]; label: string }> = [
	{ value: "all", label: "todos" },
	{ value: "in", label: "entradas" },
	{ value: "out", label: "saídas" },
];

export interface SheetFilterState {
	textFilter: string | null;
	accountFilter: string | null;
	ownerFilter: string | null;
	categoryFilter: string | null;
	uncategorizedOnly: boolean;
	unreviewedOnly: boolean;
	installmentsOnly: boolean;
	subscriptionsOnly: boolean;
	tierFilter: string | null;
}

export const SheetFilterBar = ({
	ui,
	setUi,
	accounts,
	localFilters,
	setLocalFilters,
	count,
	hasActiveFilters,
	filteredTotal,
	onExportCsv,
	accent = "var(--purple)",
	leading,
}: {
	ui: SheetFilterState;
	setUi: (patch: Partial<SheetFilterState>) => void;
	accounts: ReadonlyArray<{ id: string; label: string }>;
	localFilters: SheetLocalFilters;
	setLocalFilters: React.Dispatch<React.SetStateAction<SheetLocalFilters>>;
	count: number;
	hasActiveFilters: boolean;
	filteredTotal: number;
	onExportCsv: () => void;
	accent?: string;
	/** Optional controls rendered at the row start (e.g. the scenario selector). */
	leading?: React.ReactNode;
}) => {
	// How many refinements live *inside* the popover are active (search and
	// accounts stay outside, so they don't count toward the popover badge).
	const activeCount =
		(localFilters.origin !== "all" ? 1 : 0) +
		(localFilters.flow !== "all" ? 1 : 0) +
		(ui.uncategorizedOnly ? 1 : 0) +
		(ui.tierFilter ? 1 : 0);

	const resetAll = () => {
		setLocalFilters(DEFAULT_SHEET_LOCAL_FILTERS);
		setUi({ uncategorizedOnly: false, tierFilter: null });
	};

	return (
		<div
			style={{
				display: "flex",
				gap: 8,
				alignItems: "center",
				flexWrap: "wrap",
				padding: "12px 0",
			}}
		>
			{leading}
			<input
				type="search"
				placeholder={`buscar ${count} linhas…`}
				value={ui.textFilter ?? ""}
				onChange={(e) => setUi({ textFilter: e.target.value || null })}
				className="mono"
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-full)",
					padding: "7px 14px",
					fontSize: 12,
					minWidth: 200,
					flex: "1 1 220px",
					maxWidth: 340,
					background: "var(--card)",
				}}
			/>

			<AccountMultiSelect
				accounts={accounts}
				value={ui.accountFilter}
				onChange={(next) => setUi({ accountFilter: next })}
			/>

			<FiltersPopover
				activeCount={activeCount}
				accent={accent}
				localFilters={localFilters}
				setLocalFilters={setLocalFilters}
				uncategorizedOnly={ui.uncategorizedOnly}
				tierFilter={ui.tierFilter}
				setUi={setUi}
				onReset={resetAll}
			/>

			{/* Right cluster: filtered sum (only while filtering) + export. */}
			<div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 8 }}>
				{hasActiveFilters && (
					<div
						className="mono"
						aria-live="polite"
						style={{
							display: "flex",
							alignItems: "center",
							gap: 8,
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-full)",
							padding: "5px 12px",
							background: "var(--card)",
							fontSize: 12,
							color: "var(--muted)",
						}}
					>
						<span>soma filtrada</span>
						<strong
							style={{
								color: filteredTotal >= 0 ? "var(--green)" : "var(--rose)",
								fontWeight: 700,
								fontVariantNumeric: "tabular-nums",
							}}
						>
							{formatMoneyNumber(filteredTotal)}
						</strong>
					</div>
				)}
				<button
					className="mono pressable"
					title="exportar CSV das transações filtradas"
					aria-label="exportar CSV"
					style={{
						border: "1px solid var(--border)",
						background: "transparent",
						color: "var(--muted)",
						borderRadius: "var(--radius-full)",
						padding: "6px 12px",
						cursor: count === 0 ? "not-allowed" : "pointer",
						fontSize: 12,
						opacity: count === 0 ? 0.5 : 1,
					}}
					onClick={onExportCsv}
					disabled={count === 0}
				>
					↧ CSV
				</button>
			</div>
		</div>
	);
};

// ── Filters popover ──────────────────────────────────────────────────────────

const FiltersPopover = ({
	activeCount,
	accent,
	localFilters,
	setLocalFilters,
	uncategorizedOnly,
	tierFilter,
	setUi,
	onReset,
}: {
	activeCount: number;
	accent: string;
	localFilters: SheetLocalFilters;
	setLocalFilters: React.Dispatch<React.SetStateAction<SheetLocalFilters>>;
	uncategorizedOnly: boolean;
	tierFilter: string | null;
	setUi: (patch: Partial<SheetFilterState>) => void;
	onReset: () => void;
}) => {
	const [open, setOpen] = useState(false);
	const ref = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (!open) return;
		const onDoc = (e: MouseEvent) => {
			if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
		};
		const onEsc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
		document.addEventListener("mousedown", onDoc);
		document.addEventListener("keydown", onEsc);
		return () => {
			document.removeEventListener("mousedown", onDoc);
			document.removeEventListener("keydown", onEsc);
		};
	}, [open]);

	const active = activeCount > 0;
	return (
		<div ref={ref} style={{ position: "relative" }}>
			<button
				type="button"
				aria-haspopup="dialog"
				aria-expanded={open}
				onClick={() => setOpen((v) => !v)}
				className="mono pressable"
				style={{
					display: "inline-flex",
					alignItems: "center",
					gap: 7,
					border: `1px solid ${active ? accent : "var(--border)"}`,
					background: active ? "color-mix(in srgb, var(--purple) 8%, transparent)" : "var(--card)",
					color: active ? accent : "var(--muted)",
					borderRadius: "var(--radius-full)",
					padding: "6px 14px",
					cursor: "pointer",
					fontSize: 12,
					whiteSpace: "nowrap",
				}}
			>
				<span aria-hidden>⚙</span>
				filtros
				{active && (
					<span
						className="mono"
						style={{
							background: accent,
							color: "#fff",
							borderRadius: "var(--radius-full)",
							minWidth: 16,
							height: 16,
							display: "inline-flex",
							alignItems: "center",
							justifyContent: "center",
							fontSize: 10,
							padding: "0 4px",
						}}
					>
						{activeCount}
					</span>
				)}
			</button>
			{open && (
				<div
					role="dialog"
					aria-label="filtros da planilha"
					style={{
						position: "absolute",
						top: "calc(100% + 6px)",
						right: 0,
						zIndex: 50,
						width: 288,
						background: "var(--card)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-md)",
						boxShadow: "0 12px 32px rgba(21,19,31,0.16)",
						padding: 14,
						display: "flex",
						flexDirection: "column",
						gap: 14,
					}}
				>
					<Segment
						label="tipo"
						options={FLOW_OPTIONS}
						value={localFilters.flow}
						onChange={(flow) => setLocalFilters((f) => ({ ...f, flow }))}
						accent={accent}
					/>
					<Field label="origem">
						<select
							aria-label="filtrar por origem"
							value={localFilters.origin}
							onChange={(e) =>
								setLocalFilters((f) => ({
									...f,
									origin: e.target.value as SheetLocalFilters["origin"],
								}))
							}
							className="mono select-pill"
							style={{
								width: "100%",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
								padding: "7px 10px",
								fontSize: 12,
								backgroundColor: "var(--card)",
							}}
						>
							{ORIGIN_OPTIONS.map((o) => (
								<option key={o.value} value={o.value}>
									{o.label}
								</option>
							))}
						</select>
					</Field>
					<Field label="comprometimento">
						<div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
							{COMMITMENT_TIERS.map((tier) => (
								<TierChip
									key={tier}
									tier={tier}
									active={tierFilter === tier}
									onClick={() =>
										setUi({ tierFilter: tierFilter === tier ? null : tier })
									}
								/>
							))}
						</div>
					</Field>
					<label
						className="mono"
						style={{
							display: "flex",
							alignItems: "center",
							gap: 8,
							fontSize: 12,
							cursor: "pointer",
							color: "var(--text)",
						}}
					>
						<input
							type="checkbox"
							checked={uncategorizedOnly}
							onChange={(e) => setUi({ uncategorizedOnly: e.target.checked })}
							style={{ accentColor: accent }}
						/>
						só sem categoria
					</label>
					<div
						style={{
							display: "flex",
							justifyContent: "space-between",
							alignItems: "center",
							borderTop: "1px solid var(--border)",
							paddingTop: 10,
						}}
					>
						<button
							type="button"
							className="mono"
							onClick={onReset}
							disabled={activeCount === 0}
							style={{
								background: "transparent",
								border: "none",
								color: activeCount ? "var(--purple)" : "var(--muted2)",
								cursor: activeCount ? "pointer" : "default",
								fontSize: 12,
								padding: 0,
							}}
						>
							limpar tudo
						</button>
						<button
							type="button"
							className="mono pressable"
							onClick={() => setOpen(false)}
							style={{
								background: accent,
								border: "none",
								color: "#fff",
								borderRadius: "var(--radius-full)",
								padding: "5px 14px",
								cursor: "pointer",
								fontSize: 12,
							}}
						>
							concluir
						</button>
					</div>
				</div>
			)}
		</div>
	);
};

const Field = ({ label, children }: { label: string; children: React.ReactNode }) => (
	<div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
		<span
			className="mono"
			style={{
				fontSize: 10,
				letterSpacing: "0.1em",
				textTransform: "uppercase",
				color: "var(--muted)",
			}}
		>
			{label}
		</span>
		{children}
	</div>
);

const Segment = <T extends string>({
	label,
	options,
	value,
	onChange,
	accent,
}: {
	label: string;
	options: Array<{ value: T; label: string }>;
	value: T;
	onChange: (value: T) => void;
	accent: string;
}) => (
	<Field label={label}>
		<div
			role="radiogroup"
			aria-label={label}
			style={{
				display: "inline-flex",
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-full)",
				overflow: "hidden",
			}}
		>
			{options.map((o) => {
				const on = value === o.value;
				return (
					<button
						key={o.value}
						type="button"
						role="radio"
						aria-checked={on}
						onClick={() => onChange(o.value)}
						className="mono"
						style={{
							flex: 1,
							border: "none",
							padding: "6px 10px",
							fontSize: 11.5,
							cursor: "pointer",
							background: on ? accent : "transparent",
							color: on ? "#fff" : "var(--muted)",
						}}
					>
						{o.label}
					</button>
				);
			})}
		</div>
	</Field>
);

const TIER_COLOR: Record<CommitmentTier, string> = {
	locked: "#9a9aae",
	cancellable: "var(--amber)",
	variable: "var(--green)",
};

const TierChip = ({
	tier,
	active,
	onClick,
}: {
	tier: CommitmentTier;
	active: boolean;
	onClick: () => void;
}) => {
	const c = TIER_COLOR[tier];
	return (
		<button
			type="button"
			className="mono pressable"
			aria-pressed={active}
			onClick={onClick}
			style={{
				background: active ? c : "transparent",
				color: active ? "#1a1a1a" : "var(--muted)",
				border: `1px solid ${active ? c : "var(--border)"}`,
				borderRadius: "var(--radius-full)",
				padding: "4px 12px",
				cursor: "pointer",
				fontSize: 12,
			}}
		>
			{COMMITMENT_TIER_LABELS[tier]}
		</button>
	);
};

// ── Account multi-select (moved here from PlanilhaView) ──────────────────────
// The account filter accepts many accounts at once. The selection persists as a
// comma-joined id list in `ui.accountFilter` (single id = one-element list), so
// the ui-doc schema — and STORE_VERSION — stays untouched.

const AccountMultiSelect = ({
	accounts,
	value,
	onChange,
}: {
	accounts: ReadonlyArray<{ id: string; label: string }>;
	value: string | null;
	onChange: (next: string | null) => void;
}) => {
	const [open, setOpen] = useState(false);
	const ref = useRef<HTMLDivElement>(null);
	const selected = useMemo(() => new Set(accountFilterIds(value)), [value]);

	useEffect(() => {
		if (!open) return;
		const onDoc = (e: MouseEvent) => {
			if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
		};
		document.addEventListener("mousedown", onDoc);
		return () => document.removeEventListener("mousedown", onDoc);
	}, [open]);

	const toggle = (id: string) => {
		const next = new Set(selected);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		onChange(next.size ? Array.from(next).join(",") : null);
	};

	const label =
		selected.size === 0
			? "todas as contas"
			: selected.size === 1
				? (accounts.find((a) => selected.has(a.id))?.label ?? "1 conta")
				: `${selected.size} contas`;

	return (
		<div ref={ref} style={{ position: "relative" }}>
			<button
				type="button"
				aria-haspopup="listbox"
				aria-expanded={open}
				onClick={() => setOpen((v) => !v)}
				className="mono select-pill"
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-full)",
					padding: "6px 32px 6px 12px",
					fontSize: 12,
					backgroundColor: selected.size ? "var(--chip, #f1eefc)" : "var(--card)",
					color: selected.size ? "var(--purple)" : "var(--text)",
					cursor: "pointer",
					maxWidth: 200,
					whiteSpace: "nowrap",
					overflow: "hidden",
					textOverflow: "ellipsis",
					textAlign: "left",
				}}
			>
				{label}
			</button>
			{open && (
				<div
					role="listbox"
					aria-multiselectable
					style={{
						position: "absolute",
						top: "calc(100% + 6px)",
						left: 0,
						zIndex: 50,
						minWidth: 220,
						maxHeight: 300,
						overflow: "auto",
						background: "var(--card)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-md)",
						boxShadow: "0 8px 24px rgba(21,19,31,0.18)",
						padding: 6,
					}}
				>
					{selected.size > 0 && (
						<button
							type="button"
							className="mono"
							onClick={() => onChange(null)}
							style={accountOptionStyle(false)}
						>
							<span style={{ width: 16 }}>×</span>
							<span>limpar seleção</span>
						</button>
					)}
					{accounts.map((a) => {
						const on = selected.has(a.id);
						return (
							<button
								key={a.id}
								type="button"
								role="option"
								aria-selected={on}
								className="mono"
								onClick={() => toggle(a.id)}
								style={accountOptionStyle(on)}
							>
								<span style={{ width: 16 }}>{on ? "✓" : ""}</span>
								<span
									style={{
										overflow: "hidden",
										textOverflow: "ellipsis",
										whiteSpace: "nowrap",
									}}
								>
									{a.label}
								</span>
							</button>
						);
					})}
				</div>
			)}
		</div>
	);
};

const accountOptionStyle = (active: boolean): React.CSSProperties => ({
	display: "flex",
	alignItems: "center",
	gap: 8,
	width: "100%",
	textAlign: "left",
	background: active ? "var(--chip, #f1eefc)" : "transparent",
	color: active ? "var(--purple)" : "var(--text)",
	border: "none",
	borderRadius: "var(--radius-sm)",
	padding: "7px 8px",
	cursor: "pointer",
	fontSize: 12.5,
});
