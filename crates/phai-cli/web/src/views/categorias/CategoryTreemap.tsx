import { useEffect, useMemo, useState } from "react";
import { squarify } from "../../lib/treemap";
import { categoryEmoji } from "../../lib/categoryEmoji";
import {
	commitmentTier,
	COMMITMENT_TIER_LABELS,
	effectiveCategory,
	parseCategory,
	sheetLabel,
	type CommitmentTier,
	type ReviewOverlay,
	type TxView,
} from "../../lib/derivations";
import { formatMoneyNumber, isNegative, toCents } from "../../lib/format";

/**
 * Categorias as a drillable treemap. Level 1 tiles the month's expense
 * magnitude by parent category; clicking a tile drills into a sub-treemap of
 * its subcategories; clicking a subcategory tiles its individual transactions
 * the same way (value always visible; whatever is too small to stay legible
 * falls back to compact rows below the board). Clicking a transaction opens
 * the shared edit modal via `onEditTx`. A breadcrumb walks back up; Escape
 * goes one level up.
 *
 * Pure presentation: the caller owns filtering (the filter bar applies before
 * `txs` arrives) and editing (modal + write path).
 */

type Drill =
	| { level: 1 }
	| { level: 2; parent: string }
	| { level: 3; parent: string; sub: string };

/** Grouping lens for level 1: by category (default) or by controllability. */
type Lens = "category" | "tier";

/** Tier tile colours + glyphs (ADR-0030). */
const TIER_COLOR: Record<CommitmentTier, string> = {
	locked: "#6b6b78",
	cancellable: "#b45309",
	variable: "#15803d",
};
const TIER_GLYPH: Record<CommitmentTier, string> = {
	locked: "🔒",
	cancellable: "✂️",
	variable: "🎚️",
};

const TILE_COLORS = [
	"#6d4aff",
	"#0d9488",
	"#e11d48",
	"#b45309",
	"#0369a1",
	"#7c3aed",
	"#15803d",
	"#be185d",
	"#4d7c0f",
	"#b91c1c",
	"#0e7490",
	"#a21caf",
];

const colorFor = (index: number) => TILE_COLORS[index % TILE_COLORS.length]!;

interface Bucket {
	key: string;
	total: number;
	count: number;
	txs: TxView[];
}

const bucketBy = (
	txs: ReadonlyArray<TxView>,
	keyOf: (tx: TxView) => string,
): Bucket[] => {
	const map = new Map<string, Bucket>();
	for (const tx of txs) {
		const key = keyOf(tx);
		let b = map.get(key);
		if (!b) {
			b = { key, total: 0, count: 0, txs: [] };
			map.set(key, b);
		}
		b.total += Math.abs(toCents(tx.amount)) / 100;
		b.count += 1;
		b.txs.push(tx);
	}
	return Array.from(map.values()).sort((a, b) => b.total - a.total);
};

export const CategoryTreemap = ({
	txs,
	overlayMap,
	onEditTx,
	fixedCategories,
}: {
	/** The month's transactions, already filtered by the caller. */
	txs: ReadonlyArray<TxView>;
	overlayMap: Map<string, ReviewOverlay>;
	onEditTx: (tx: TxView) => void;
	/** Fixed-category set for the controllability lens (ADR-0030). */
	fixedCategories?: ReadonlySet<string>;
}) => {
	const [drill, setDrill] = useState<Drill>({ level: 1 });
	const [lens, setLens] = useState<Lens>("category");

	const expenses = useMemo(
		() => txs.filter((t) => isNegative(t.amount)),
		[txs],
	);
	const income = useMemo(() => txs.filter((t) => !isNegative(t.amount)), [txs]);
	const cat = (tx: TxView) => effectiveCategory(tx, overlayMap);

	// Level-1 grouping key + the next-level key, both driven by the lens. In the
	// tier lens level 1 is the controllability tier and level 2 is its
	// categories; in the category lens it is parent category → subcategory.
	const level1Key =
		lens === "tier"
			? (tx: TxView) => commitmentTier(tx, fixedCategories, overlayMap)
			: (tx: TxView) => parseCategory(cat(tx)).parent;
	const level2Key =
		lens === "tier"
			? (tx: TxView) => parseCategory(cat(tx)).parent
			: (tx: TxView) => parseCategory(cat(tx)).sub ?? "—";

	const parents = useMemo(
		() => bucketBy(expenses, level1Key),
		// eslint-disable-next-line react-hooks/exhaustive-deps
		[expenses, overlayMap, lens, fixedCategories],
	);
	const parentIndex = useMemo(
		() => new Map(parents.map((p, i) => [p.key, i])),
		[parents],
	);

	// Drill targets can disappear when filters / edits change the data.
	useEffect(() => {
		if (drill.level !== 1 && !parentIndex.has(drill.parent)) {
			setDrill({ level: 1 });
		}
	}, [drill, parentIndex]);

	useEffect(() => {
		const onKey = (e: KeyboardEvent) => {
			if (e.key !== "Escape") return;
			setDrill((d) =>
				d.level === 3
					? { level: 2, parent: d.parent }
					: d.level === 2
						? { level: 1 }
						: d,
			);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, []);

	const subs = useMemo(() => {
		if (drill.level === 1) return [];
		const parent = drill.parent;
		const inParent = expenses.filter((tx) => level1Key(tx) === parent);
		return bucketBy(inParent, level2Key);
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [drill, expenses, overlayMap, lens, fixedCategories]);

	// Switching the lens invalidates any category-keyed drill path.
	useEffect(() => setDrill({ level: 1 }), [lens]);

	const monthTotal = parents.reduce((s, p) => s + p.total, 0);
	const incomeTotal = income.reduce(
		(s, t) => s + Math.abs(toCents(t.amount)) / 100,
		0,
	);

	const crumb = (label: string, target: Drill, active: boolean) => (
		<button
			key={label}
			onClick={() => setDrill(target)}
			className="mono"
			disabled={active}
			style={{
				background: "transparent",
				border: "none",
				padding: 0,
				fontSize: 13,
				cursor: active ? "default" : "pointer",
				color: active ? "var(--text)" : "var(--purple)",
				fontWeight: active ? 600 : 400,
			}}
		>
			{label}
		</button>
	);

	// Lens-aware presentation helpers for level-1 root tiles.
	const rootColor = (key: string): string =>
		lens === "tier"
			? TIER_COLOR[key as CommitmentTier]
			: colorFor(parentIndex.get(key) ?? 0);
	const rootGlyph = (key: string): string =>
		lens === "tier" ? TIER_GLYPH[key as CommitmentTier] : categoryEmoji(key);
	const rootLabel = (key: string): string =>
		lens === "tier" ? COMMITMENT_TIER_LABELS[key as CommitmentTier] : key;

	return (
		<section aria-label="month categories (treemap)">
			{/* Lens toggle — category vs controllability (ADR-0030) */}
			<div style={{ display: "flex", gap: 6, padding: "4px 0" }}>
				{(["category", "tier"] as const).map((l) => (
					<button
						key={l}
						onClick={() => setLens(l)}
						className="mono"
						aria-pressed={lens === l}
						style={{
							background: lens === l ? "var(--purple)" : "transparent",
							color: lens === l ? "#fff" : "var(--muted)",
							border: `1px solid ${lens === l ? "var(--purple)" : "var(--border)"}`,
							borderRadius: "var(--radius-full)",
							padding: "3px 12px",
							fontSize: 11,
							cursor: "pointer",
						}}
					>
						{l === "category" ? "by category" : "by controllability"}
					</button>
				))}
			</div>

			{/* Breadcrumb */}
			<div
				style={{
					display: "flex",
					alignItems: "baseline",
					gap: 8,
					padding: "10px 0",
					flexWrap: "wrap",
				}}
			>
				{crumb(
					`expenses ${formatMoneyNumber(monthTotal)}`,
					{ level: 1 },
					drill.level === 1,
				)}
				{(drill.level === 2 || drill.level === 3) && (
					<>
						<span style={{ color: "var(--muted)" }}>›</span>
						{crumb(
							rootLabel(drill.parent),
							{ level: 2, parent: drill.parent },
							drill.level === 2,
						)}
					</>
				)}
				{drill.level === 3 && (
					<>
						<span style={{ color: "var(--muted)" }}>›</span>
						{crumb(drill.sub, drill, true)}
					</>
				)}
				<span
					className="mono"
					style={{ marginLeft: "auto", fontSize: 12, color: "var(--muted)" }}
				>
					{drill.level === 1
						? "click a category to open it"
						: "Esc goes up one level"}
				</span>
			</div>

			{drill.level === 1 && (
				<TreemapBoard
					buckets={parents}
					total={monthTotal}
					colorOf={(key) => rootColor(key)}
					emojiOf={(key) => rootGlyph(key)}
					labelOf={(key) => rootLabel(key)}
					onOpen={(key) => setDrill({ level: 2, parent: key })}
				/>
			)}

			{drill.level === 2 && (
				<TreemapBoard
					buckets={subs}
					total={subs.reduce((s, b) => s + b.total, 0)}
					colorOf={() => rootColor(drill.parent)}
					emojiOf={(key) =>
						lens === "tier" ? categoryEmoji(key) : categoryEmoji(drill.parent)
					}
					shade
					onOpen={(key) =>
						setDrill({ level: 3, parent: drill.parent, sub: key })
					}
				/>
			)}

			{drill.level === 3 && (
				<TxBoard
					txs={
						subs.find((b) => b.key === drill.sub)?.txs ??
						([] as ReadonlyArray<TxView>)
					}
					color={rootColor(drill.parent)}
					emoji={
						lens === "tier"
							? categoryEmoji(drill.sub)
							: categoryEmoji(drill.parent)
					}
					onEditTx={onEditTx}
				/>
			)}

			{/* Income strip — the treemap is expenses-only by design. */}
			{drill.level === 1 && income.length > 0 && (
				<div
					className="mono"
					style={{
						display: "flex",
						gap: 12,
						padding: "10px 2px",
						fontSize: 12,
						color: "var(--muted)",
					}}
				>
					<span style={{ color: "var(--green)" }}>
						income {formatMoneyNumber(incomeTotal)}
					</span>
					<span>
						{income.length} transaction{income.length > 1 ? "s" : ""} — edit
						in the sheet or click below
					</span>
				</div>
			)}
			{drill.level === 1 &&
				income.slice(0, 6).map((tx) => (
					<button
						key={tx.id}
						onClick={() => onEditTx(tx)}
						className="mono"
						style={{
							display: "flex",
							gap: 10,
							width: "100%",
							textAlign: "left",
							background: "transparent",
							border: "none",
							borderBottom: "1px solid var(--border)",
							padding: "6px 2px",
							fontSize: 12,
							cursor: "pointer",
							alignItems: "baseline",
						}}
					>
						<span style={{ color: "var(--muted)" }}>
							{tx.postedAt.slice(8, 10)}/{tx.postedAt.slice(5, 7)}
						</span>
						<span
							style={{
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
								flex: 1,
							}}
						>
							{sheetLabel(tx)}
						</span>
						<span style={{ color: "var(--green)" }}>
							{formatMoneyNumber(toCents(tx.amount) / 100)}
						</span>
					</button>
				))}
		</section>
	);
};

/** One treemap level rendered as absolutely-positioned tiles (percent space). */
const TreemapBoard = ({
	buckets,
	total,
	colorOf,
	emojiOf,
	labelOf,
	onOpen,
	shade,
}: {
	buckets: ReadonlyArray<Bucket>;
	total: number;
	colorOf: (key: string) => string;
	emojiOf?: (key: string) => string;
	/** Display label for a tile; defaults to the bucket key. */
	labelOf?: (key: string) => string;
	onOpen: (key: string) => void;
	/** Level 2: same hue family, opacity scaled by share. */
	shade?: boolean;
}) => {
	const rects = useMemo(
		() =>
			squarify(
				buckets.map((b) => ({ id: b.key, value: b.total })),
				0,
				0,
				100,
				100,
			),
		[buckets],
	);
	const byId = useMemo(() => new Map(buckets.map((b) => [b.key, b])), [buckets]);

	if (buckets.length === 0) {
		return (
			<div
				className="mono"
				style={{
					padding: 32,
					textAlign: "center",
					color: "var(--muted)",
					fontSize: 13,
					border: "1px dashed var(--border)",
					borderRadius: "var(--radius-md)",
				}}
			>
				No expenses for the current filters.
			</div>
		);
	}

	return (
		<div
			style={{
				position: "relative",
				width: "100%",
				aspectRatio: "16 / 7.5",
				minHeight: 280,
				borderRadius: "var(--radius-md)",
				overflow: "hidden",
			}}
		>
			{rects.map((r) => {
				const b = byId.get(r.id)!;
				const pct = total > 0 ? Math.round((b.total / total) * 100) : 0;
				const big = r.w * r.h >= 6; // enough room for two text lines
				return (
					<button
						key={r.id}
						onClick={() => onOpen(r.id)}
						title={`${labelOf ? labelOf(r.id) : r.id} · ${formatMoneyNumber(b.total)} (${pct}%) · ${b.count} tx`}
						style={{
							position: "absolute",
							left: `${r.x}%`,
							top: `${r.y}%`,
							width: `${r.w}%`,
							height: `${r.h}%`,
							background: colorOf(r.id),
							opacity: shade ? 0.45 + 0.55 * (b.total / (buckets[0]?.total || 1)) : 0.92,
							border: "2px solid var(--bg)",
							borderRadius: 6,
							cursor: "pointer",
							display: "flex",
							flexDirection: "column",
							alignItems: "flex-start",
							justifyContent: "flex-start",
							padding: big ? "8px 10px" : "2px 6px",
							overflow: "hidden",
							color: "#fff",
							transition: "filter 120ms",
						}}
						onMouseEnter={(e) => {
							(e.currentTarget as HTMLElement).style.filter = "brightness(1.12)";
						}}
						onMouseLeave={(e) => {
							(e.currentTarget as HTMLElement).style.filter = "";
						}}
					>
						<span
							style={{
								fontSize: big ? 14 : 11,
								fontWeight: 600,
								maxWidth: "100%",
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
								textShadow: "0 1px 2px rgba(0,0,0,0.25)",
							}}
						>
							{emojiOf ? `${emojiOf(r.id)} ` : ""}
							{labelOf ? labelOf(r.id) : r.id}
						</span>
						{big && (
							<span
								className="mono"
								style={{
									fontSize: 12,
									opacity: 0.95,
									textShadow: "0 1px 2px rgba(0,0,0,0.25)",
								}}
							>
								{formatMoneyNumber(b.total)} · {pct}%
							</span>
						)}
					</button>
				);
			})}
		</div>
	);
};

// Legibility floor for level-3 tiles: below this share of the board a tile
// cannot render a readable value, so the transaction moves to the row list.
const MIN_TILE_SHARE_PCT = 2.5;
const MAX_TILES = 30;

const txMagnitude = (tx: TxView) => Math.abs(toCents(tx.amount)) / 100;

/**
 * Level 3 — the subcategory's transactions as treemap tiles, value always
 * visible (the whole point of the drill: see where the money went at a
 * glance). Transactions too small to stay legible as squares fall back to
 * compact amount-sorted rows below the board; click anything to edit.
 */
const TxBoard = ({
	txs,
	color,
	emoji,
	onEditTx,
}: {
	txs: ReadonlyArray<TxView>;
	color: string;
	emoji: string;
	onEditTx: (tx: TxView) => void;
}) => {
	const sorted = useMemo(
		() =>
			[...txs].sort(
				(a, b) => Math.abs(toCents(b.amount)) - Math.abs(toCents(a.amount)),
			),
		[txs],
	);
	const total = sorted.reduce((s, t) => s + txMagnitude(t), 0);

	// Sorted descending, so the legible prefix is contiguous.
	const tiles = useMemo(
		() =>
			total > 0
				? sorted
						.slice(0, MAX_TILES)
						.filter((t) => (txMagnitude(t) / total) * 100 >= MIN_TILE_SHARE_PCT)
				: [],
		[sorted, total],
	);
	const rest = sorted.slice(tiles.length);
	const maxMag = txMagnitude(sorted[0] ?? ({ amount: "0" } as TxView)) || 1;

	const rects = useMemo(
		() =>
			squarify(
				tiles.map((t) => ({ id: t.id, value: txMagnitude(t) })),
				0,
				0,
				100,
				100,
			),
		[tiles],
	);
	const byId = useMemo(() => new Map(tiles.map((t) => [t.id, t])), [tiles]);

	if (sorted.length === 0) {
		return (
			<div
				className="mono"
				style={{
					padding: 32,
					textAlign: "center",
					color: "var(--muted)",
					fontSize: 13,
					border: "1px dashed var(--border)",
					borderRadius: "var(--radius-md)",
				}}
			>
				No expenses for the current filters.
			</div>
		);
	}

	return (
		<div>
			{tiles.length > 0 && (
				<div
					style={{
						position: "relative",
						width: "100%",
						aspectRatio: "16 / 6.5",
						minHeight: 240,
						borderRadius: "var(--radius-md)",
						overflow: "hidden",
					}}
				>
					{rects.map((r) => (
						<TxTile
							key={r.id}
							tx={byId.get(r.id)!}
							rect={r}
							color={color}
							emoji={emoji}
							maxMag={maxMag}
							onEditTx={onEditTx}
						/>
					))}
				</div>
			)}

			{rest.length > 0 && (
				<div
					style={{
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-md)",
						background: "var(--card)",
						overflow: "hidden",
						marginTop: tiles.length > 0 ? 10 : 0,
					}}
				>
					{tiles.length > 0 && (
						<div
							className="mono"
							style={{
								padding: "8px 14px",
								fontSize: 11,
								color: "var(--muted)",
								borderBottom: "1px solid var(--border)",
								textTransform: "uppercase",
								letterSpacing: "0.06em",
							}}
						>
							menores demais para o quadro ({rest.length})
						</div>
					)}
					{rest.map((tx) => (
						<RestRow
							key={tx.id}
							tx={tx}
							emoji={emoji}
							maxMag={maxMag}
							onEditTx={onEditTx}
						/>
					))}
				</div>
			)}

			<div
				className="mono"
				style={{
					display: "flex",
					justifyContent: "space-between",
					padding: "9px 4px",
					fontSize: 12,
					color: "var(--muted)",
				}}
			>
				<span>
					{sorted.length} transaction{sorted.length === 1 ? "" : "s"} · click
					to edit
				</span>
				<span style={{ fontWeight: 600 }}>{formatMoneyNumber(total)}</span>
			</div>
		</div>
	);
};

/** One level-3 tile: emoji + label when there is room, value always. */
const TxTile = ({
	tx,
	rect,
	color,
	emoji,
	maxMag,
	onEditTx,
}: {
	tx: TxView;
	rect: { x: number; y: number; w: number; h: number };
	color: string;
	emoji: string;
	maxMag: number;
	onEditTx: (tx: TxView) => void;
}) => {
	const mag = txMagnitude(tx);
	const big = rect.w * rect.h >= 6; // room for label + value lines
	const label = sheetLabel(tx);
	const marker = tx.installmentMarker ? ` · ${tx.installmentMarker}` : "";
	return (
		<button
			onClick={() => onEditTx(tx)}
			title={`${label} · ${formatMoneyNumber(mag)}${marker} · edit`}
			style={{
				position: "absolute",
				left: `${rect.x}%`,
				top: `${rect.y}%`,
				width: `${rect.w}%`,
				height: `${rect.h}%`,
				background: color,
				opacity: 0.45 + 0.55 * (mag / maxMag),
				border: "2px solid var(--bg)",
				borderRadius: 6,
				cursor: "pointer",
				display: "flex",
				flexDirection: "column",
				alignItems: "flex-start",
				justifyContent: big ? "flex-start" : "center",
				padding: big ? "8px 10px" : "2px 6px",
				overflow: "hidden",
				color: "#fff",
				transition: "filter 120ms",
			}}
			onMouseEnter={(e) => {
				(e.currentTarget as HTMLElement).style.filter = "brightness(1.12)";
			}}
			onMouseLeave={(e) => {
				(e.currentTarget as HTMLElement).style.filter = "";
			}}
		>
			{big && (
				<span
					style={{
						fontSize: 13,
						fontWeight: 600,
						maxWidth: "100%",
						overflow: "hidden",
						textOverflow: "ellipsis",
						whiteSpace: "nowrap",
						textShadow: "0 1px 2px rgba(0,0,0,0.25)",
					}}
				>
					{emoji} {label}
				</span>
			)}
			<span
				className="mono"
				style={{
					fontSize: big ? 12 : 11,
					fontWeight: big ? 400 : 600,
					maxWidth: "100%",
					overflow: "hidden",
					textOverflow: "ellipsis",
					whiteSpace: "nowrap",
					opacity: 0.95,
					textShadow: "0 1px 2px rgba(0,0,0,0.25)",
				}}
			>
				{formatMoneyNumber(mag)}
				{big ? marker : ""}
			</span>
		</button>
	);
};

/** A below-the-board row for transactions too small to tile legibly. */
const RestRow = ({
	tx,
	emoji,
	maxMag,
	onEditTx,
}: {
	tx: TxView;
	emoji: string;
	maxMag: number;
	onEditTx: (tx: TxView) => void;
}) => {
	const mag = txMagnitude(tx);
	return (
		<button
			onClick={() => onEditTx(tx)}
			title="edit transaction"
			style={{
				position: "relative",
				display: "flex",
				gap: 12,
				alignItems: "baseline",
				width: "100%",
				textAlign: "left",
				background: "transparent",
				border: "none",
				borderBottom: "1px solid var(--border)",
				padding: "9px 14px",
				cursor: "pointer",
				fontSize: 14,
			}}
		>
			{/* Magnitude bar behind the row — instant visual ranking. */}
			<span
				aria-hidden
				style={{
					position: "absolute",
					left: 0,
					top: 0,
					bottom: 0,
					width: `${(mag / maxMag) * 100}%`,
					background: "rgba(109,74,255,0.07)",
				}}
			/>
			<span className="mono" style={{ color: "var(--muted)", fontSize: 12 }}>
				{tx.postedAt.slice(8, 10)}/{tx.postedAt.slice(5, 7)}
			</span>
			<span
				style={{
					flex: 1,
					overflow: "hidden",
					textOverflow: "ellipsis",
					whiteSpace: "nowrap",
				}}
			>
				{emoji} {sheetLabel(tx)}
				{tx.installmentMarker ? (
					<span
						className="mono"
						style={{ color: "var(--muted)", fontSize: 11, marginLeft: 8 }}
					>
						installment {tx.installmentMarker}
					</span>
				) : null}
			</span>
			<span className="mono" style={{ fontWeight: 600, whiteSpace: "nowrap" }}>
				{formatMoneyNumber(mag)}
			</span>
		</button>
	);
};
