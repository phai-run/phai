import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { useCallback, useMemo, useState } from "react";
import { events, tables } from "../../livestore/schema";
import { formatMoneyNumber } from "../../lib/format";

const scenarios$ = queryDb(tables.scenarios.orderBy("name", "asc"));
const scenarioChanges$ = queryDb(tables.scenarioChanges);

interface ScenarioView {
	scenarioId: string;
	name: string;
	description: string | null;
	status: string;
}

/**
 * Compact scenario pills for the unified sheet header (design H): baseline |
 * active scenarios | "+ novo" (inline input). While a scenario is active, a
 * thin teal strip shows "N mudanças · Δ {soma}" plus promote (one extra
 * confirmation click) and discard. Same events as the ScenarioPanel — this is
 * just its sheet-sized rendering.
 */
export const SheetScenarioBar = ({
	activeScenarioId,
	scenarioDelta,
	onActivate,
	onMutated,
}: {
	activeScenarioId: string | null;
	/** Selected-month projected-saldo delta vs. baseline (null = not seeded). */
	scenarioDelta: number | null;
	onActivate: (scenarioId: string | null) => void;
	/** Fired after any scenario write so the caller re-seeds the projection. */
	onMutated: () => void;
}) => {
	const { store } = useStore();
	const scenarios = useQuery(scenarios$) as ReadonlyArray<ScenarioView>;
	const allChanges = useQuery(scenarioChanges$) as ReadonlyArray<{
		scenarioId: string;
	}>;
	const [creating, setCreating] = useState(false);
	const [newName, setNewName] = useState("");
	const [confirmPromote, setConfirmPromote] = useState(false);

	const active = useMemo(
		() => scenarios.find((s) => s.scenarioId === activeScenarioId) ?? null,
		[scenarios, activeScenarioId],
	);
	const pickable = useMemo(
		() => scenarios.filter((s) => s.status === "ativo"),
		[scenarios],
	);
	const changeCount = useMemo(
		() => allChanges.filter((c) => c.scenarioId === activeScenarioId).length,
		[allChanges, activeScenarioId],
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

	const lifecycle = useCallback(
		(action: "promote" | "delete") => {
			if (!activeScenarioId) return;
			const writeId = crypto.randomUUID();
			const now = Date.now();
			store.commit(
				action === "promote"
					? events.scenarioPromoted({
							writeId,
							scenarioId: activeScenarioId,
							promotedAt: now,
						})
					: events.scenarioDeleted({
							writeId,
							scenarioId: activeScenarioId,
							deletedAt: now,
						}),
			);
			setConfirmPromote(false);
			onActivate(null);
			onMutated();
		},
		[store, activeScenarioId, onActivate, onMutated],
	);

	return (
		<div style={{ margin: "4px 0 8px" }}>
			<div
				style={{
					display: "flex",
					gap: 6,
					flexWrap: "wrap",
					alignItems: "center",
				}}
			>
				<span className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
					🧪
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

			{active && (
				<div
					className="mono"
					style={{
						display: "flex",
						gap: 10,
						alignItems: "center",
						flexWrap: "wrap",
						marginTop: 6,
						padding: "5px 10px",
						fontSize: 11,
						border: "1px solid var(--cyan)",
						borderRadius: "var(--radius-sm)",
						background: "rgba(8,145,178,0.06)",
						color: "var(--cyan)",
					}}
				>
					<span>
						{changeCount} mudança{changeCount === 1 ? "" : "s"}
						{scenarioDelta != null
							? ` · Δ ${formatMoneyNumber(scenarioDelta)}`
							: ""}
					</span>
					<span style={{ opacity: 0.8 }}>
						edições nesta planilha viram mudanças do cenário
					</span>
					<span style={{ marginLeft: "auto", display: "inline-flex", gap: 6 }}>
						{confirmPromote ? (
							<>
								<span style={{ color: "var(--amber)" }}>
									aplicar {changeCount} mudança(s) ao plano real?
								</span>
								<button
									onClick={() => lifecycle("promote")}
									className="mono"
									style={{ ...pillStyle(true), background: "var(--cyan)", borderColor: "var(--cyan)" }}
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
									disabled={changeCount === 0}
									className="mono"
									style={{
										...pillStyle(false),
										color: "var(--cyan)",
										opacity: changeCount === 0 ? 0.4 : 1,
									}}
								>
									▶ promover ao plano
								</button>
								<button
									onClick={() => lifecycle("delete")}
									className="mono"
									style={{ ...pillStyle(false), color: "var(--rose)" }}
								>
									descartar
								</button>
							</>
						)}
					</span>
				</div>
			)}
		</div>
	);
};

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
	color: "var(--text)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "4px 8px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
	outline: "none",
};
