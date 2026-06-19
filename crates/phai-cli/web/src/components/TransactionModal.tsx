import {
	useCallback,
	useEffect,
	useMemo,
	useState,
	type CSSProperties,
} from "react";
import { AnimatePresence, motion } from "framer-motion";
import { useStore } from "@livestore/react";
import { events } from "../livestore/schema";
import { amountColor, formatMoney } from "../lib/format";
import type { ReviewOverlay, TxView } from "../lib/derivations";

/**
 * Transaction edit modal — shared by the categorias view, the planilha view
 * and the card installment panel so "click a transaction → edit description /
 * merchant / purpose / category" behaves identically everywhere. Bulk apply to
 * similar transactions included. Render it inside an <AnimatePresence>.
 */

export interface ReviewPatch {
	description: string | null;
	merchantName: string | null;
	purpose: string | null;
	categoryId: string | null;
	commitmentTier?: string | null;
}

const inputStyle: CSSProperties = {
	background: "var(--bg)",
	color: "var(--white)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "5px 9px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
	outline: "none",
};

const pillStyle: CSSProperties = {
	background: "transparent",
	color: "var(--muted)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-full)",
	padding: "4px 12px",
	cursor: "pointer",
	fontSize: 11,
	fontFamily: "var(--font-mono)",
};

// ── Transaction modal ──────────────────────────────────────────────────────

export const TransactionModal = ({
	tx,
	overlay,
	similarTxs,
	overlayById,
	categories,
	onSubmit,
	onClose,
}: {
	tx: TxView;
	overlay?: ReviewOverlay | ReviewPatch;
	similarTxs: ReadonlyArray<TxView>;
	overlayById: Map<string, ReviewOverlay | ReviewPatch>;
	/** Category ids for the edit form's autocomplete datalist. */
	categories: ReadonlyArray<string>;
	onSubmit: (patch: ReviewPatch) => void;
	onClose: () => void;
}) => {
	type Tab = "edit" | "raw" | "similar";
	const [tab, setTab] = useState<Tab>("edit");
	const [description, setDescription] = useState(
		overlay?.description ?? tx.description ?? "",
	);
	const [merchantName, setMerchantName] = useState(
		overlay?.merchantName ?? tx.merchantName ?? "",
	);
	const [purpose, setPurpose] = useState(overlay?.purpose ?? tx.purpose ?? "");
	const [category, setCategory] = useState(
		overlay?.categoryId ?? tx.categoryId ?? "",
	);

	// Keep fields in sync if overlay changes while modal is open
	useEffect(() => {
		setDescription(overlay?.description ?? tx.description ?? "");
		setMerchantName(overlay?.merchantName ?? tx.merchantName ?? "");
		setPurpose(overlay?.purpose ?? tx.purpose ?? "");
		setCategory(overlay?.categoryId ?? tx.categoryId ?? "");
	}, [tx.id]); // reset on tx change only

	// Bulk edit: selected similar txs
	const [selectedSimilar, setSelectedSimilar] = useState<Set<string>>(
		new Set(),
	);
	const { store } = useStore();

	const applySelectedSimilar = useCallback(
		(patch: ReviewPatch) => {
			// Replicate the human anatomy edit only — never force the source's
			// commitment tier onto the similar rows; each keeps its own override.
			const { commitmentTier: _omitTier, ...bulkPatch } = patch;
			for (const id of selectedSimilar) {
				store.commit(
					events.reviewSubmitted({
						writeId: crypto.randomUUID(),
						transactionId: id,
						patch: bulkPatch,
						submittedAt: Date.now(),
					}),
				);
			}
			setSelectedSimilar(new Set());
		},
		[selectedSimilar, store],
	);

	const handleToggle = useCallback((id: string) => {
		setSelectedSimilar((prev) => {
			const next = new Set(prev);
			if (next.has(id)) next.delete(id);
			else next.add(id);
			return next;
		});
	}, []);

	const handleSelectAll = useCallback(() => {
		setSelectedSimilar(new Set(similarTxs.map((t) => t.id)));
	}, [similarTxs]);

	const handleClearAll = useCallback(() => {
		setSelectedSimilar(new Set());
	}, []);

	const currentPatch = useMemo(
		(): ReviewPatch => ({
			description: description.trim() || null,
			merchantName: merchantName.trim() || null,
			purpose: purpose.trim() || null,
			categoryId: category.trim() || null,
			// Carry the existing commitment-tier override so an anatomy edit
			// doesn't drop it: the overlay row is replaced wholesale on submit
			// (schema onConflict "replace"), so any omitted column is lost.
			commitmentTier: overlay?.commitmentTier ?? tx.commitmentTier ?? null,
		}),
		[description, merchantName, purpose, category, overlay, tx.commitmentTier],
	);

	const selectedSimilarCount = selectedSimilar.size;

	const handleSave = useCallback(() => {
		const patch = currentPatch;
		applySelectedSimilar(patch);
		onSubmit(patch);
	}, [applySelectedSimilar, currentPatch, onSubmit]);

	const saveLabel =
		selectedSimilarCount > 0
			? `Salvar (também em ${selectedSimilarCount} selecionadas)`
			: "Salvar";

	return (
		<>
			<datalist id="phai-modal-cats">
				{categories.map((c) => (
					<option key={c} value={c} />
				))}
			</datalist>
			{/* Backdrop */}
			<motion.div
				key="modal-backdrop"
				initial={{ opacity: 0 }}
				animate={{ opacity: 1 }}
				exit={{ opacity: 0 }}
				transition={{ duration: 0.15 }}
				onClick={onClose}
				style={{
					position: "fixed",
					inset: 0,
					background: "rgba(21,19,31,0.35)",
					backdropFilter: "blur(2px)",
					zIndex: 50,
					display: "flex",
					alignItems: "center",
					justifyContent: "center",
					padding: 20,
				}}
			>
				{/* Modal panel */}
				<motion.div
					key="modal-panel"
					onClick={(e) => e.stopPropagation()}
					initial={{ opacity: 0, scale: 0.97, y: 8 }}
					animate={{ opacity: 1, scale: 1, y: 0 }}
					exit={{ opacity: 0, scale: 0.97, y: 8 }}
					transition={{ duration: 0.16, ease: "easeOut" }}
					style={{
						width: "100%",
						maxWidth: tab === "similar" ? 900 : 520,
						maxHeight: "85vh",
						overflowY: "auto",
						background: "var(--bg)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-xl)",
						padding: 24,
						boxShadow: "0 20px 60px rgba(21,19,31,0.14)",
					}}
				>
					{/* Header */}
					<div
						style={{
							display: "flex",
							alignItems: "center",
							gap: 10,
							marginBottom: 16,
						}}
					>
						<span
							className="mono"
							style={{
								color: amountColor(tx.amount),
								fontWeight: 600,
								fontSize: 15,
							}}
						>
							{formatMoney(tx.amount)}
						</span>
						<span
							style={{
								flex: 1,
								overflow: "hidden",
								textOverflow: "ellipsis",
								whiteSpace: "nowrap",
								fontSize: 13,
							}}
						>
							{tx.description ?? tx.merchantName ?? tx.rawDescription}
						</span>
						<button
							onClick={onClose}
							className="mono"
							style={{
								background: "transparent",
								border: "none",
								cursor: "pointer",
								color: "var(--muted)",
								fontSize: 16,
								padding: "0 4px",
							}}
						>
							×
						</button>
					</div>

					{/* Tabs */}
					<div
						style={{
							display: "flex",
							gap: 4,
							marginBottom: 18,
							borderBottom: "1px solid var(--border)",
							paddingBottom: 8,
						}}
					>
						{(["edit", "raw", "similar"] as Tab[]).map((t) => (
							<button
								key={t}
								onClick={() => setTab(t)}
								className="mono"
								style={{
									background:
										tab === t ? "rgba(109,74,255,0.08)" : "transparent",
									color: tab === t ? "var(--purple)" : "var(--muted)",
									border: `1px solid ${tab === t ? "rgba(109,74,255,0.3)" : "transparent"}`,
									borderRadius: "var(--radius-full)",
									padding: "4px 14px",
									cursor: "pointer",
									fontSize: 12,
								}}
							>
								{t === "edit"
									? "Edit"
									: t === "raw"
										? "JSON"
										: `Similar (${similarTxs.length})`}
							</button>
						))}
					</div>

					{/* Tab content */}
					<AnimatePresence mode="wait" initial={false}>
						{tab === "edit" && (
							<motion.div
								key="edit"
								initial={{ opacity: 0, x: -8 }}
								animate={{ opacity: 1, x: 0 }}
								exit={{ opacity: 0, x: 8 }}
								transition={{ duration: 0.12 }}
							>
								<EditForm
									description={description}
									setDescription={setDescription}
									merchantName={merchantName}
									setMerchantName={setMerchantName}
									purpose={purpose}
									setPurpose={setPurpose}
									category={category}
									setCategory={setCategory}
									postedAt={tx.postedAt}
									accountId={tx.accountId}
								/>
							</motion.div>
						)}

						{tab === "raw" && (
							<motion.div
								key="raw"
								initial={{ opacity: 0, x: -8 }}
								animate={{ opacity: 1, x: 0 }}
								exit={{ opacity: 0, x: 8 }}
								transition={{ duration: 0.12 }}
							>
								<pre
									className="mono"
									style={{
										background: "var(--surface)",
										border: "1px solid var(--border)",
										borderRadius: "var(--radius-sm)",
										padding: 14,
										fontSize: 11,
										overflowX: "auto",
										whiteSpace: "pre-wrap",
										wordBreak: "break-all",
										lineHeight: 1.6,
									}}
								>
									{JSON.stringify(
										{
											id: tx.id,
											accountId: tx.accountId,
											postedAt: tx.postedAt,
											amount: tx.amount,
											rawDescription: tx.rawDescription,
											description: tx.description,
											merchantName: tx.merchantName,
											purpose: tx.purpose,
											categoryId: tx.categoryId,
											month: tx.month,
											paymentStatus: tx.paymentStatus,
											reviewed: tx.reviewed,
											isInstallment: tx.isInstallment,
											isSubscription: tx.isSubscription,
											_overlay: overlayById.get(tx.id) ?? null,
										},
										null,
										2,
									)}
								</pre>
							</motion.div>
						)}

						{tab === "similar" && (
							<motion.div
								key="similar"
								initial={{ opacity: 0, x: -8 }}
								animate={{ opacity: 1, x: 0 }}
								exit={{ opacity: 0, x: 8 }}
								transition={{ duration: 0.12 }}
							>
								<SimilarPanel
									similarTxs={similarTxs}
									overlayById={overlayById}
									selected={selectedSimilar}
									onToggle={handleToggle}
									onSelectAll={handleSelectAll}
									onClearAll={handleClearAll}
								/>
							</motion.div>
						)}
					</AnimatePresence>
					<div style={{ display: "flex", gap: 8, marginTop: 16 }}>
						<button
							onClick={handleSave}
							className="mono"
							style={{
								...pillStyle,
								background: "var(--purple)",
								color: "#fff",
								borderColor: "transparent",
							}}
						>
							{saveLabel}
						</button>
						<button onClick={onClose} className="mono" style={pillStyle}>
							cancel
						</button>
					</div>
				</motion.div>
			</motion.div>
		</>
	);
};

const EditForm = ({
	description,
	setDescription,
	merchantName,
	setMerchantName,
	purpose,
	setPurpose,
	category,
	setCategory,
	postedAt,
	accountId,
}: {
	description: string;
	setDescription: (v: string) => void;
	merchantName: string;
	setMerchantName: (v: string) => void;
	purpose: string;
	setPurpose: (v: string) => void;
	category: string;
	setCategory: (v: string) => void;
	postedAt: string;
	accountId: string;
}) => (
	<div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
		<div
			className="mono"
			style={{ fontSize: 11, color: "var(--muted)", marginBottom: 4 }}
		>
			{postedAt} · {accountId}
		</div>

		<FieldRow label="category">
			<input
				list="phai-modal-cats"
				value={category}
				onChange={(e) => setCategory(e.target.value)}
				placeholder="category"
				className="mono"
				style={{ ...inputStyle, color: "var(--cyan)", flex: 1 }}
			/>
		</FieldRow>
		<FieldRow label="description">
			<input
				value={description}
				onChange={(e) => setDescription(e.target.value)}
				placeholder="description"
				className="mono"
				style={{ ...inputStyle, flex: 1 }}
			/>
		</FieldRow>
		<FieldRow label="merchant">
			<input
				value={merchantName}
				onChange={(e) => setMerchantName(e.target.value)}
				placeholder="merchant"
				className="mono"
				style={{ ...inputStyle, flex: 1 }}
			/>
		</FieldRow>
		<FieldRow label="purpose">
			<input
				value={purpose}
				onChange={(e) => setPurpose(e.target.value)}
				placeholder="purpose"
				className="mono"
				style={{ ...inputStyle, flex: 1 }}
			/>
		</FieldRow>
	</div>
);

const FieldRow = ({
	label,
	children,
}: {
	label: string;
	children: React.ReactNode;
}) => (
	<div
		style={{
			display: "grid",
			gridTemplateColumns: "90px 1fr",
			gap: 10,
			alignItems: "center",
		}}
	>
		<span
			className="mono"
			style={{
				fontSize: 10,
				color: "var(--muted)",
				textTransform: "uppercase",
				letterSpacing: "0.06em",
			}}
		>
			{label}
		</span>
		{children}
	</div>
);

const SimilarPanel = ({
	similarTxs,
	overlayById,
	selected,
	onToggle,
	onSelectAll,
	onClearAll,
}: {
	similarTxs: ReadonlyArray<TxView>;
	overlayById: Map<
		string,
		{
			categoryId: string | null;
			description: string | null;
			merchantName: string | null;
			purpose: string | null;
		}
	>;
	selected: Set<string>;
	onToggle: (id: string) => void;
	onSelectAll: () => void;
	onClearAll: () => void;
}) => {
	if (similarTxs.length === 0) {
		return (
			<p className="mono" style={{ color: "var(--muted)", fontSize: 13 }}>
				No similar transactions in this window.
			</p>
		);
	}

	return (
		<div>
			<div
				style={{
					display: "flex",
					gap: 8,
					alignItems: "center",
					marginBottom: 12,
					flexWrap: "wrap",
				}}
			>
				<button onClick={onSelectAll} className="mono" style={pillStyle}>
					select all ({similarTxs.length})
				</button>
				{selected.size > 0 && (
					<button onClick={onClearAll} className="mono" style={pillStyle}>
						clear ({selected.size})
					</button>
				)}
			</div>

			<div
				style={{
					display: "flex",
					flexDirection: "column",
					gap: 0,
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-sm)",
					overflow: "hidden",
				}}
			>
				{similarTxs.map((tx, idx) => {
					const o = overlayById.get(tx.id);
					const cat = o?.categoryId ?? tx.categoryId;
					const display =
						o?.description ??
						tx.description ??
						tx.merchantName ??
						tx.rawDescription;
					const isSelected = selected.has(tx.id);

					return (
						<div
							key={tx.id}
							onClick={() => onToggle(tx.id)}
							style={{
								display: "flex",
								alignItems: "center",
								gap: 10,
								padding: "8px 12px",
								borderTop: idx > 0 ? "1px solid var(--border)" : "none",
								background: isSelected
									? "rgba(109,74,255,0.06)"
									: "transparent",
								cursor: "pointer",
								transition: "background 80ms",
							}}
						>
							<span
								style={{
									width: 14,
									height: 14,
									borderRadius: 3,
									border: `2px solid ${isSelected ? "var(--purple)" : "var(--border)"}`,
									background: isSelected ? "var(--purple)" : "transparent",
									flexShrink: 0,
									display: "flex",
									alignItems: "center",
									justifyContent: "center",
								}}
							>
								{isSelected && (
									<span style={{ color: "#fff", fontSize: 9, fontWeight: 700 }}>
										✓
									</span>
								)}
							</span>
							<span
								className="mono"
								style={{ fontSize: 10, color: "var(--muted2)", minWidth: 50 }}
							>
								{tx.postedAt.slice(0, 7)}
							</span>
							<span
								style={{
									flex: 1,
									fontSize: 12,
									overflow: "hidden",
									textOverflow: "ellipsis",
									whiteSpace: "nowrap",
								}}
							>
								{display}
							</span>
							{cat && (
								<span
									className="mono"
									style={{ fontSize: 10, color: "var(--cyan)" }}
								>
									{cat}
								</span>
							)}
							<span
								className="mono"
								style={{
									color: amountColor(tx.amount),
									fontSize: 12,
									whiteSpace: "nowrap",
								}}
							>
								{formatMoney(tx.amount)}
							</span>
						</div>
					);
				})}
			</div>
		</div>
	);
};
