import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { useCallback, useMemo, useState } from "react";
import { events, tables } from "../../livestore/schema";
import { amountColor, formatMoney } from "../../lib/format";

const scenarios$ = queryDb(tables.scenarios.orderBy("name", "asc"));
const scenarioChanges$ = queryDb(tables.scenarioChanges);

interface ScenarioView {
	scenarioId: string;
	name: string;
	description: string | null;
	status: string;
}

interface ScenarioChangeView {
	changeId: string;
	scenarioId: string;
	kind: string;
	targetForecastId: string | null;
	targetTemplateId: string | null;
	month: string | null;
	effectiveFrom: string | null;
	amount: string | null;
	monthsCount: number | null;
	description: string | null;
	status: string;
	orphaned: number;
}

const describeChange = (c: ScenarioChangeView): string => {
	const amount = c.amount ? formatMoney(c.amount) : "";
	switch (c.kind) {
		case "add_one_shot":
			return `+ ${c.description ?? "entrada"} em ${c.month ?? "?"} (${amount})`;
		case "adjust_amount":
			return `~ ${c.description ?? c.targetForecastId ?? "?"} → ${amount}`;
		case "skip_forecast":
			return `− pular ${c.description ?? c.targetForecastId ?? "?"}`;
		case "end_template":
			return `✂ encerrar ${c.description ?? c.targetTemplateId ?? "?"} desde ${c.effectiveFrom ?? "?"}`;
		case "hypothetical_installment":
			return `≡ ${c.description ?? "parcelamento"} — ${c.monthsCount ?? 0}x de ${amount} desde ${c.effectiveFrom ?? "?"}`;
		default:
			return c.kind;
	}
};

/**
 * Scenario workbench (ADR-0037): pick/create a named what-if scenario, list
 * its deltas (orphans flagged), add one-shots and hypothetical installments,
 * and promote/archive/delete it. Per-forecast deltas (adjust/skip/end) are
 * added from the forecast list while a scenario is active.
 */
export const ScenarioPanel = ({
	selectedMonth,
	activeScenarioId,
	onActivate,
	onMutated,
}: {
	selectedMonth: string;
	activeScenarioId: string | null;
	onActivate: (scenarioId: string | null) => void;
	/** Fired after any scenario write so the caller can re-seed the projection. */
	onMutated: () => void;
}) => {
	const { store } = useStore();
	const scenarios = useQuery(scenarios$) as ReadonlyArray<ScenarioView>;
	const allChanges = useQuery(
		scenarioChanges$,
	) as ReadonlyArray<ScenarioChangeView>;
	const [creating, setCreating] = useState(false);
	const [newName, setNewName] = useState("");
	const [confirmPromote, setConfirmPromote] = useState(false);
	const [addKind, setAddKind] = useState<"one-shot" | "installment" | null>(
		null,
	);
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [months, setMonths] = useState("10");

	const active = useMemo(
		() => scenarios.find((s) => s.scenarioId === activeScenarioId) ?? null,
		[scenarios, activeScenarioId],
	);
	const activeChanges = useMemo(
		() => allChanges.filter((c) => c.scenarioId === activeScenarioId),
		[allChanges, activeScenarioId],
	);
	const pickable = useMemo(
		() => scenarios.filter((s) => s.status === "ativo"),
		[scenarios],
	);

	const createScenario = useCallback(() => {
		const name = newName.trim();
		if (!name) return;
		const scenarioId = `scn-${crypto.randomUUID()}`;
		store.commit(
			events.scenarioCreated({
				writeId: crypto.randomUUID(),
				scenarioId,
				name,
				description: null,
				createdAt: Date.now(),
			}),
		);
		setNewName("");
		setCreating(false);
		onActivate(scenarioId);
		onMutated();
	}, [newName, store, onActivate, onMutated]);

	const removeChange = useCallback(
		(changeId: string) => {
			if (!activeScenarioId) return;
			store.commit(
				events.scenarioChangeRemoved({
					writeId: crypto.randomUUID(),
					changeId,
					scenarioId: activeScenarioId,
					removedAt: Date.now(),
				}),
			);
			onMutated();
		},
		[store, activeScenarioId, onMutated],
	);

	const submitAdd = useCallback(() => {
		if (!activeScenarioId || !addKind) return;
		const desc = description.trim();
		const mag = amount.replace(/^-/, "").trim();
		const count = Number(months);
		if (!desc || !mag) return;
		if (addKind === "installment" && (!Number.isFinite(count) || count < 1))
			return;
		const changeId = `chg-${crypto.randomUUID()}`;
		store.commit(
			events.scenarioChangeAdded({
				writeId: crypto.randomUUID(),
				row: {
					changeId,
					scenarioId: activeScenarioId,
					kind:
						addKind === "one-shot" ? "add_one_shot" : "hypothetical_installment",
					targetForecastId: null,
					targetTemplateId: null,
					month: addKind === "one-shot" ? selectedMonth : null,
					effectiveFrom: addKind === "installment" ? selectedMonth : null,
					amount: `-${mag}`,
					monthsCount: addKind === "installment" ? count : null,
					description: desc,
					categoryId: null,
					accountId: null,
					status: "ativo",
					orphaned: 0,
				},
				addedAt: Date.now(),
			}),
		);
		setDescription("");
		setAmount("");
		setAddKind(null);
		onMutated();
	}, [
		activeScenarioId,
		addKind,
		description,
		amount,
		months,
		selectedMonth,
		store,
		onMutated,
	]);

	const lifecycle = useCallback(
		(action: "promote" | "archive" | "delete") => {
			if (!activeScenarioId) return;
			const writeId = crypto.randomUUID();
			const now = Date.now();
			if (action === "promote") {
				store.commit(
					events.scenarioPromoted({
						writeId,
						scenarioId: activeScenarioId,
						promotedAt: now,
					}),
				);
			} else if (action === "archive") {
				store.commit(
					events.scenarioArchived({
						writeId,
						scenarioId: activeScenarioId,
						archivedAt: now,
					}),
				);
			} else {
				store.commit(
					events.scenarioDeleted({
						writeId,
						scenarioId: activeScenarioId,
						deletedAt: now,
					}),
				);
			}
			setConfirmPromote(false);
			onActivate(null);
			onMutated();
		},
		[store, activeScenarioId, onActivate, onMutated],
	);

	return (
		<div
			style={{
				border: active ? "1px solid var(--cyan)" : "1px dashed var(--border)",
				borderRadius: "var(--radius-sm)",
				padding: "10px 12px",
				margin: "12px 0 0",
				background: active ? "rgba(8,145,178,0.05)" : "transparent",
			}}
		>
			{/* Picker row */}
			<div style={{ display: "flex", gap: 6, flexWrap: "wrap", alignItems: "center" }}>
				<span className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
					🧪 cenário:
				</span>
				<button
					onClick={() => onActivate(null)}
					className="mono"
					style={pillStyle(activeScenarioId == null)}
				>
					baseline
				</button>
				{pickable.map((s) => (
					<button
						key={s.scenarioId}
						onClick={() => onActivate(s.scenarioId)}
						className="mono"
						style={pillStyle(s.scenarioId === activeScenarioId)}
						title={s.description ?? s.name}
					>
						{s.name}
					</button>
				))}
				{creating ? (
					<span style={{ display: "inline-flex", gap: 4 }}>
						<input
							autoFocus
							placeholder="nome do cenário"
							value={newName}
							onChange={(e) => setNewName(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter") createScenario();
								if (e.key === "Escape") setCreating(false);
							}}
							className="mono"
							style={inputStyle}
						/>
						<button onClick={createScenario} className="mono" style={pillStyle(false)}>
							criar
						</button>
					</span>
				) : (
					<button
						onClick={() => setCreating(true)}
						className="mono"
						style={{ ...pillStyle(false), borderStyle: "dashed" }}
					>
						+ novo
					</button>
				)}
			</div>

			{/* Active-scenario body */}
			{active && (
				<div style={{ marginTop: 10 }}>
					<div
						className="mono"
						style={{ fontSize: 11, color: "var(--muted)", marginBottom: 6 }}
					>
						mudanças sobre o baseline (edite valores na lista de forecasts do
						mês, ou adicione abaixo):
					</div>
					{activeChanges.length === 0 ? (
						<div className="mono" style={{ fontSize: 12, color: "var(--muted)" }}>
							(nenhuma mudança ainda)
						</div>
					) : (
						<div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
							{activeChanges.map((c) => (
								<div
									key={c.changeId}
									style={{
										display: "flex",
										justifyContent: "space-between",
										alignItems: "center",
										gap: 8,
										fontSize: 12,
									}}
								>
									<span
										style={{
											color: c.orphaned ? "var(--amber)" : "var(--white)",
											overflow: "hidden",
											textOverflow: "ellipsis",
											whiteSpace: "nowrap",
										}}
										title={c.orphaned ? "órfã — alvo realizado/removido" : undefined}
									>
										{c.orphaned ? "⚠️ " : ""}
										{describeChange(c)}
									</span>
									<button
										onClick={() => removeChange(c.changeId)}
										className="mono"
										aria-label={`remover ${c.changeId}`}
										style={{ ...pillStyle(false), padding: "1px 8px" }}
									>
										×
									</button>
								</div>
							))}
						</div>
					)}

					{/* Add forms */}
					<div style={{ display: "flex", gap: 6, marginTop: 8, flexWrap: "wrap" }}>
						{addKind == null ? (
							<>
								<button
									onClick={() => setAddKind("one-shot")}
									className="mono"
									style={{ ...pillStyle(false), borderStyle: "dashed" }}
								>
									+ gasto em {selectedMonth}
								</button>
								<button
									onClick={() => setAddKind("installment")}
									className="mono"
									style={{ ...pillStyle(false), borderStyle: "dashed" }}
								>
									+ parcelamento desde {selectedMonth}
								</button>
							</>
						) : (
							<span style={{ display: "inline-flex", gap: 4, flexWrap: "wrap" }}>
								<input
									autoFocus
									placeholder="descrição"
									value={description}
									onChange={(e) => setDescription(e.target.value)}
									className="mono"
									style={inputStyle}
								/>
								<input
									inputMode="decimal"
									placeholder={addKind === "installment" ? "parcela" : "0,00"}
									value={amount}
									onChange={(e) => setAmount(e.target.value)}
									onKeyDown={(e) => e.key === "Enter" && submitAdd()}
									className="mono"
									style={{ ...inputStyle, width: 80 }}
								/>
								{addKind === "installment" && (
									<input
										inputMode="numeric"
										placeholder="x"
										value={months}
										onChange={(e) => setMonths(e.target.value)}
										className="mono"
										style={{ ...inputStyle, width: 40 }}
										title="número de parcelas"
									/>
								)}
								<button onClick={submitAdd} className="mono" style={pillStyle(false)}>
									adicionar
								</button>
								<button
									onClick={() => setAddKind(null)}
									className="mono"
									style={pillStyle(false)}
								>
									cancelar
								</button>
							</span>
						)}
					</div>

					{/* Lifecycle */}
					<div style={{ display: "flex", gap: 6, marginTop: 10 }}>
						{confirmPromote ? (
							<>
								<span className="mono" style={{ fontSize: 11, color: "var(--amber)" }}>
									aplicar {activeChanges.length} mudança(s) ao plano real?
								</span>
								<button
									onClick={() => lifecycle("promote")}
									className="mono"
									style={{ ...pillStyle(true), background: "var(--cyan)" }}
								>
									confirmar
								</button>
								<button
									onClick={() => setConfirmPromote(false)}
									className="mono"
									style={pillStyle(false)}
								>
									cancelar
								</button>
							</>
						) : (
							<>
								<button
									onClick={() => setConfirmPromote(true)}
									disabled={activeChanges.length === 0}
									className="mono"
									style={{
										...pillStyle(false),
										color: "var(--cyan)",
										opacity: activeChanges.length === 0 ? 0.4 : 1,
									}}
								>
									▶ promover ao plano real
								</button>
								<button
									onClick={() => lifecycle("archive")}
									className="mono"
									style={pillStyle(false)}
								>
									arquivar
								</button>
								<button
									onClick={() => lifecycle("delete")}
									className="mono"
									style={{ ...pillStyle(false), color: "var(--rose)" }}
								>
									excluir
								</button>
							</>
						)}
					</div>
				</div>
			)}
		</div>
	);
};

/** Amount colour helper re-exported for the forecast-row scenario actions. */
export const scenarioAmountColor = amountColor;

const pillStyle = (active: boolean): React.CSSProperties => ({
	background: active ? "var(--purple)" : "transparent",
	color: active ? "#fff" : "var(--muted)",
	border: `1px solid ${active ? "var(--purple)" : "var(--border)"}`,
	borderRadius: "var(--radius-full)",
	padding: "3px 12px",
	cursor: "pointer",
	fontSize: 11,
	fontFamily: "var(--font-mono)",
});

const inputStyle: React.CSSProperties = {
	background: "var(--bg)",
	color: "var(--white)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "4px 8px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
	outline: "none",
};
