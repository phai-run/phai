import { queryDb } from "@livestore/livestore";
import { useStore, useQuery, useClientDocument } from "@livestore/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { events, tables } from "../livestore/schema";
import { useTransactionsSeed } from "../bridge/sync";
import {
	amountColor,
	formatMoney,
	formatMoneyNumber,
	isNegative,
	sumAmounts,
} from "../lib/format";
import {
	Card,
	EmptyState,
	ErrorNote,
	Label,
	Pill,
	Select,
	TextInput,
	ViewHeader,
} from "../components/ui";

const ACCENT = "var(--purple)";

const txAll$ = queryDb(tables.transactions.orderBy("postedAt", "desc"));
const overlay$ = queryDb(tables.reviewOverlay);
const categories$ = queryDb(tables.categories.orderBy("id", "asc"));
const accounts$ = queryDb(tables.accounts.orderBy("label", "asc"));

interface Patch {
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
}

interface TxView {
	id: string;
	accountId: string;
	postedAt: string;
	amount: string;
	rawDescription: string;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
	month: string;
	paymentStatus: string;
	reviewed: number;
	isInstallment: number;
	isSubscription: number;
}

/**
 * Revisão — the transaction list with live-sum filters. The full window is
 * seeded into LiveStore once; every filter and the running sums (saídas /
 * entradas) are computed **client-side** so the list reacts instantly with no
 * network round-trip. Inline edits commit a `reviewSubmitted` event (optimistic
 * overlay + queued for flush).
 *
 * Layout: a primary list plus a sticky side rail (filters + running totals) on
 * wide screens (DESIGN.md multi-pane); single column below 1024px.
 */
export const Review = () => {
	const { store } = useStore();
	const [ui, setUi] = useClientDocument(tables.ui);
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const overlay = useQuery(overlay$);
	const categories = useQuery(categories$);
	const accounts = useQuery(accounts$);

	// Seed the whole window once; window controls re-seed (rare).
	const seed = useTransactionsSeed(ui.monthsBack, ui.monthsAhead);

	const overlayById = useMemo(
		() => new Map(overlay.map((o) => [o.transactionId, o])),
		[overlay],
	);
	const accountById = useMemo(
		() => new Map(accounts.map((a) => [a.id, a])),
		[accounts],
	);
	const categoryIds = useMemo(() => categories.map((c) => c.id), [categories]);
	const owners = useMemo(
		() => Array.from(new Set(accounts.map((a) => a.owner).filter(Boolean))),
		[accounts],
	);

	// Effective category for a row = overlay (optimistic edit) over the seed.
	const effectiveCategory = (tx: TxView) =>
		overlayById.get(tx.id)?.categoryId ?? tx.categoryId;

	// All filtering is client-side over the seeded window — instant, no round-trip.
	const filtered = useMemo(() => {
		const cat = ui.categoryFilter?.trim().toLowerCase() || null;
		const merchant = ui.merchantFilter?.trim().toLowerCase() || null;
		return txRows.filter((tx) => {
			if (ui.unreviewedOnly && tx.reviewed) return false;
			if (ui.subscriptionsOnly && !tx.isSubscription) return false;
			if (ui.installmentsOnly && !tx.isInstallment) return false;
			if (ui.accountFilter && tx.accountId !== ui.accountFilter) return false;
			if (ui.ownerFilter) {
				const owner = accountById.get(tx.accountId)?.owner ?? "";
				if (owner !== ui.ownerFilter) return false;
			}
			if (cat) {
				const c = (effectiveCategory(tx) ?? "").toLowerCase();
				if (!c.includes(cat)) return false;
			}
			if (merchant) {
				const m = (tx.merchantName ?? tx.rawDescription ?? "").toLowerCase();
				if (!m.includes(merchant)) return false;
			}
			return true;
		});
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [txRows, overlayById, accountById, ui]);

	// Running sums for the current selection (exact integer-cents math).
	const sums = useMemo(() => {
		const out = filtered
			.filter((t) => isNegative(t.amount))
			.map((t) => t.amount);
		const inc = filtered
			.filter((t) => !isNegative(t.amount))
			.map((t) => t.amount);
		return { saidas: Math.abs(sumAmounts(out)), entradas: sumAmounts(inc) };
	}, [filtered]);

	const cursor = Math.min(ui.cursor, Math.max(0, filtered.length - 1));
	const focusRef = useRef<(() => void) | null>(null);

	// Keyboard navigation: ↑/↓ move cursor, Enter focuses selected category.
	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			const tag = (e.target as HTMLElement | null)?.tagName;
			const typing = tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA";
			if (e.key === "ArrowDown" && !typing) {
				e.preventDefault();
				setUi({ cursor: Math.min(cursor + 1, filtered.length - 1) });
			} else if (e.key === "ArrowUp" && !typing) {
				e.preventDefault();
				setUi({ cursor: Math.max(cursor - 1, 0) });
			} else if (e.key === "Enter" && !typing) {
				e.preventDefault();
				focusRef.current?.();
			}
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [cursor, filtered.length, setUi]);

	const submit = (transactionId: string, patch: Patch) =>
		store.commit(
			events.reviewSubmitted({
				writeId: crypto.randomUUID(),
				transactionId,
				patch,
				submittedAt: Date.now(),
			}),
		);

	const activeFilterCount = [
		ui.unreviewedOnly,
		ui.subscriptionsOnly,
		ui.installmentsOnly,
		!!ui.ownerFilter,
		!!ui.accountFilter,
		!!ui.categoryFilter,
		!!ui.merchantFilter,
	].filter(Boolean).length;

	return (
		<div
			className="review-grid"
			style={{ display: "grid", gap: 24, alignItems: "start" }}
		>
			<div style={{ minWidth: 0 }}>
				<ViewHeader title="Revisão" count={filtered.length} accent={ACCENT} />
				{activeFilterCount > 0 && (
					<span
						className="mono"
						style={{ color: "var(--muted)", fontSize: 12, marginLeft: 8 }}
					>
						· {activeFilterCount} filtro{activeFilterCount !== 1 ? "s" : ""}
					</span>
				)}

				{seed.error && <ErrorNote error={seed.error} />}
				{seed.loading && txRows.length === 0 && (
					<div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
						{[1, 2, 3, 4].map((i) => (
							<div key={i} className="skeleton" style={{ height: 80 }} />
						))}
					</div>
				)}

				{filtered.length === 0 && !seed.loading ? (
					<EmptyState message="Nenhuma transação para este filtro." />
				) : (
					<div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
						{filtered.map((tx, i) => {
							const o = overlayById.get(tx.id);
							return (
								<ReviewRow
									key={tx.id}
									selected={i === cursor}
									registerFocus={
										i === cursor ? (fn) => (focusRef.current = fn) : undefined
									}
									onSelect={() => setUi({ cursor: i })}
									postedAt={tx.postedAt}
									amount={tx.amount}
									rawDescription={tx.rawDescription}
									description={o?.description ?? tx.description}
									merchantName={o?.merchantName ?? tx.merchantName}
									purpose={o?.purpose ?? tx.purpose}
									category={o?.categoryId ?? tx.categoryId}
									reviewed={tx.reviewed === 1}
									isSubscription={tx.isSubscription === 1}
									isInstallment={tx.isInstallment === 1}
									categories={categoryIds}
									onSubmit={(patch) => submit(tx.id, patch)}
								/>
							);
						})}
					</div>
				)}
			</div>

			<aside className="review-rail">
				<Card accent={ACCENT} style={{ position: "sticky", top: 16 }}>
					<FilterRail
						ui={ui}
						setUi={setUi}
						owners={owners}
						accounts={accounts}
						onReload={() => seed.reload()}
					/>
					{(ui.unreviewedOnly ||
						ui.subscriptionsOnly ||
						ui.installmentsOnly ||
						ui.ownerFilter ||
						ui.accountFilter ||
						ui.categoryFilter ||
						ui.merchantFilter) && (
						<div style={{ marginTop: 8 }}>
							<Pill
								accent={ACCENT}
								onClick={() =>
									setUi({
										unreviewedOnly: false,
										subscriptionsOnly: false,
										installmentsOnly: false,
										ownerFilter: null,
										accountFilter: null,
										categoryFilter: null,
										merchantFilter: null,
									})
								}
							>
								limpar filtros
							</Pill>
						</div>
					)}
					<SumStrip
						saidas={sums.saidas}
						entradas={sums.entradas}
						count={filtered.length}
					/>
				</Card>
			</aside>

			<datalist id="phai-categories">
				{categoryIds.map((c) => (
					<option key={c} value={c} />
				))}
			</datalist>
		</div>
	);
};

interface RailUi {
	ownerFilter: string | null;
	accountFilter: string | null;
	merchantFilter: string | null;
	categoryFilter: string | null;
	unreviewedOnly: boolean;
	subscriptionsOnly: boolean;
	installmentsOnly: boolean;
}

const FilterRail = ({
	ui,
	setUi,
	owners,
	accounts,
	onReload,
}: {
	ui: RailUi;
	setUi: (patch: Partial<RailUi>) => void;
	owners: string[];
	accounts: ReadonlyArray<{ id: string; label: string; owner: string }>;
	onReload: () => void;
}) => (
	<div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
		<Label>filtros</Label>

		<div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
			<Pill
				active={ui.unreviewedOnly}
				accent={ACCENT}
				onClick={() => setUi({ unreviewedOnly: !ui.unreviewedOnly })}
			>
				{ui.unreviewedOnly ? "não revisadas" : "todas"}
			</Pill>
			<Pill
				active={ui.subscriptionsOnly}
				accent="var(--cyan)"
				onClick={() => setUi({ subscriptionsOnly: !ui.subscriptionsOnly })}
			>
				assinaturas
			</Pill>
			<Pill
				active={ui.installmentsOnly}
				accent="var(--amber)"
				onClick={() => setUi({ installmentsOnly: !ui.installmentsOnly })}
			>
				parcelas
			</Pill>
		</div>

		<TextInput
			list="phai-categories"
			placeholder="categoria…"
			value={ui.categoryFilter ?? ""}
			onChange={(e) => setUi({ categoryFilter: e.target.value || null })}
			style={{ color: "var(--cyan)", width: "100%" }}
			aria-label="categoria"
		/>
		<TextInput
			placeholder="merchant…"
			value={ui.merchantFilter ?? ""}
			onChange={(e) => setUi({ merchantFilter: e.target.value || null })}
			style={{ width: "100%" }}
			aria-label="merchant"
		/>
		<Select
			value={ui.ownerFilter ?? ""}
			onChange={(e) => setUi({ ownerFilter: e.target.value || null })}
			aria-label="responsável"
			style={{ width: "100%" }}
		>
			<option value="">todos · responsável</option>
			{owners.map((o) => (
				<option key={o} value={o}>
					{o}
				</option>
			))}
		</Select>
		<Select
			value={ui.accountFilter ?? ""}
			onChange={(e) => setUi({ accountFilter: e.target.value || null })}
			aria-label="conta"
			style={{ width: "100%" }}
		>
			<option value="">todas · conta</option>
			{accounts.map((a) => (
				<option key={a.id} value={a.id}>
					{a.label || a.id}
				</option>
			))}
		</Select>

		<Pill accent={ACCENT} onClick={onReload}>
			↻ recarregar janela
		</Pill>
	</div>
);

const SumStrip = ({
	saidas,
	entradas,
	count,
}: {
	saidas: number;
	entradas: number;
	count: number;
}) => (
	<div
		style={{
			marginTop: 16,
			paddingTop: 16,
			borderTop: "1px solid var(--border)",
			display: "flex",
			flexDirection: "column",
			gap: 10,
		}}
	>
		<Label>seleção · {count}</Label>
		<SumLine
			label="total saídas"
			value={formatMoneyNumber(-saidas)}
			color="var(--rose)"
		/>
		<SumLine
			label="total entradas"
			value={formatMoneyNumber(entradas)}
			color="var(--green)"
		/>
		<SumLine
			label="líquido"
			value={formatMoneyNumber(entradas - saidas)}
			color="var(--purple)"
		/>
	</div>
);

const SumLine = ({
	label,
	value,
	color,
}: {
	label: string;
	value: string;
	color: string;
}) => (
	<div
		className="mono"
		style={{
			display: "flex",
			justifyContent: "space-between",
			alignItems: "baseline",
			fontSize: 13,
		}}
	>
		<span style={{ color: "var(--muted)", fontSize: 11 }}>{label}</span>
		<span style={{ color, fontWeight: 600 }}>{value}</span>
	</div>
);

const ReviewRow = (props: {
	selected: boolean;
	registerFocus?: (focus: () => void) => void;
	onSelect: () => void;
	postedAt: string;
	amount: string;
	rawDescription: string;
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	category: string | null;
	reviewed: boolean;
	isSubscription: boolean;
	isInstallment: boolean;
	categories: string[];
	onSubmit: (patch: Patch) => void;
}) => {
	const [expanded, setExpanded] = useState(false);
	const [category, setCategory] = useState(props.category ?? "");
	const [description, setDescription] = useState(props.description ?? "");
	const [merchantName, setMerchantName] = useState(props.merchantName ?? "");
	const [purpose, setPurpose] = useState(props.purpose ?? "");
	const [showDropdown, setShowDropdown] = useState(false);
	const categoryRef = useRef<HTMLInputElement>(null);

	// Keep local edit state in sync when the overlay/seed changes underneath.
	useEffect(() => setCategory(props.category ?? ""), [props.category]);
	useEffect(() => setDescription(props.description ?? ""), [props.description]);
	useEffect(
		() => setMerchantName(props.merchantName ?? ""),
		[props.merchantName],
	);
	useEffect(() => setPurpose(props.purpose ?? ""), [props.purpose]);

	// Let the parent focus this row's category input on Enter.
	useEffect(() => {
		props.registerFocus?.(() => {
			setExpanded(true);
			categoryRef.current?.focus();
		});
	}, [props.registerFocus]);

	const display =
		props.description || props.merchantName || props.rawDescription;

	const commitCategory = () => {
		const next = category.trim() || null;
		if (next !== (props.category ?? null)) {
			props.onSubmit({
				description: null,
				merchantName: null,
				purpose: null,
				categoryId: next,
			});
		}
	};

	const commitAnatomy = () => {
		props.onSubmit({
			description: description.trim() || null,
			merchantName: merchantName.trim() || null,
			purpose: purpose.trim() || null,
			categoryId: category.trim() || null,
		});
		setExpanded(false);
	};

	return (
		<Card
			selected={props.selected}
			accent={ACCENT}
			style={{ cursor: "default" }}
		>
			<div
				onClick={props.onSelect}
				style={{
					display: "grid",
					gridTemplateColumns: "1fr auto",
					gap: 12,
					alignItems: "center",
				}}
			>
				<div style={{ minWidth: 0 }}>
					<div
						style={{
							fontWeight: 500,
							overflow: "hidden",
							textOverflow: "ellipsis",
							display: "flex",
							gap: 6,
							alignItems: "center",
						}}
					>
						{display}
						{props.isSubscription && (
							<Tag label="assinatura" color="var(--cyan)" />
						)}
						{props.isInstallment && (
							<Tag label="parcela" color="var(--amber)" />
						)}
						{props.reviewed && <Tag label="revisada" color="var(--green)" />}
					</div>
					<div className="mono" style={{ color: "var(--muted)", fontSize: 12 }}>
						{props.postedAt}
						{props.merchantName ? ` · ${props.merchantName}` : ""}
					</div>
				</div>
				<div style={{ textAlign: "right" }}>
					<div
						className="mono"
						style={{ color: amountColor(props.amount), fontWeight: 500 }}
					>
						{formatMoney(props.amount)}
					</div>
					<div style={{ position: "relative" }}>
						<input
							ref={categoryRef}
							list="phai-categories"
							value={category}
							placeholder="categoria…"
							onChange={(e) => setCategory(e.target.value)}
							onFocus={() => {
								props.onSelect();
								setShowDropdown(true);
							}}
							onBlur={() => {
								commitCategory();
								setTimeout(() => setShowDropdown(false), 150);
							}}
							onKeyDown={(e) => {
								if (e.key === "Enter") {
									(e.target as HTMLInputElement).blur();
								}
							}}
							className="mono"
							style={{
								marginTop: 6,
								background: "var(--bg)",
								color: "var(--cyan)",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
								padding: "4px 8px",
								fontSize: 12,
								width: 180,
							}}
						/>
						{showDropdown && (
							<div
								className="mono"
								style={{
									position: "absolute",
									top: "100%",
									left: 0,
									zIndex: 10,
									background: "var(--surface)",
									border: "1px solid var(--border)",
									borderRadius: "var(--radius-sm)",
									maxHeight: 200,
									overflowY: "auto",
									width: 180,
									marginTop: 2,
								}}
							>
								{props.categories
									.filter((c) =>
										c.toLowerCase().includes(category.toLowerCase()),
									)
									.slice(0, 8)
									.map((c) => (
										<div
											key={c}
											onClick={() => {
												setCategory(c);
												setShowDropdown(false);
											}}
											style={{
												padding: "6px 10px",
												cursor: "pointer",
												fontSize: 12,
												background:
													c === category
														? "rgba(109,74,255,0.08)"
														: "transparent",
											}}
										>
											{c.toLowerCase().includes(category.toLowerCase())
												? c
														.split(
															new RegExp(
																`(${category.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`,
																"i",
															),
														)
														.map((part, i) =>
															part.toLowerCase() === category.toLowerCase() ? (
																<span
																	key={i}
																	style={{
																		color: "var(--purple)",
																		fontWeight: 600,
																	}}
																>
																	{part}
																</span>
															) : (
																part
															),
														)
												: c}
										</div>
									))}
								{props.categories.filter((c) =>
									c.toLowerCase().includes(category.toLowerCase()),
								).length === 0 && (
									<div
										style={{
											padding: "6px 10px",
											color: "var(--muted)",
											fontSize: 12,
										}}
									>
										sem resultados
									</div>
								)}
							</div>
						)}
					</div>
				</div>
			</div>

			<button
				onClick={() => setExpanded((v) => !v)}
				className="mono"
				style={{
					marginTop: 10,
					background: "transparent",
					border: "none",
					color: "var(--muted)",
					fontSize: 11,
					cursor: "pointer",
					padding: 0,
				}}
			>
				{expanded ? "◇ ocultar anatomia" : "◇ editar anatomia"}
			</button>

			{expanded && (
				<div
					style={{
						marginTop: 12,
						display: "grid",
						gridTemplateColumns: "1fr",
						gap: 10,
						borderTop: "1px solid var(--border)",
						paddingTop: 12,
					}}
				>
					<AnatomyField
						label="descrição"
						value={description}
						onChange={setDescription}
					/>
					<AnatomyField
						label="merchant"
						value={merchantName}
						onChange={setMerchantName}
					/>
					<AnatomyField
						label="propósito"
						value={purpose}
						onChange={setPurpose}
					/>
					<div style={{ display: "flex", gap: 8 }}>
						<Pill accent={ACCENT} active onClick={commitAnatomy}>
							salvar →
						</Pill>
					</div>
				</div>
			)}
		</Card>
	);
};

const Tag = ({ label, color }: { label: string; color: string }) => (
	<span
		className="mono"
		style={{
			fontSize: 10,
			color,
			border: `1px solid ${color}`,
			borderRadius: "var(--radius-full)",
			padding: "1px 8px",
			whiteSpace: "nowrap",
		}}
	>
		{label}
	</span>
);

const AnatomyField = ({
	label,
	value,
	onChange,
}: {
	label: string;
	value: string;
	onChange: (v: string) => void;
}) => (
	<label
		style={{
			display: "grid",
			gridTemplateColumns: "110px 1fr",
			gap: 10,
			alignItems: "center",
		}}
	>
		<Label>{label}</Label>
		<TextInput value={value} onChange={(e) => onChange(e.target.value)} />
	</label>
);
