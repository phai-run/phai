import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
 * Compact scenario selector for the sheet's filter row (ADR-0037 / ADR-0038).
 * A single pill — `🧪 baseline ▾`, or the active scenario's name with its Δ —
 * opens a popover that lists baseline + active scenarios, starts a new one, and
 * (while a scenario is active) promotes or discards it. Living inline with the
 * filters keeps the planning surface on one line instead of its own strip.
 *
 * Colour hierarchy: the trigger goes teal (cyan) while a scenario is active to
 * signal "you're editing a projection, not the real plan"; baseline is neutral.
 */
export const SheetScenarioBar = ({
	activeScenarioId,
	scenarioDelta,
	canCreate,
	onActivate,
	onMutated,
}: {
	activeScenarioId: string | null;
	/** Selected-month projected-saldo delta vs. baseline (null = not seeded). */
	scenarioDelta: number | null;
	/** Whether a new scenario can be started from the viewed month (current+future only). */
	canCreate: boolean;
	onActivate: (scenarioId: string | null) => void;
	/** Fired after any scenario write so the caller re-seeds the projection. */
	onMutated: () => void;
}) => {
	const { store } = useStore();
	const scenarios = useQuery(scenarios$) as ReadonlyArray<ScenarioView>;
	const allChanges = useQuery(scenarioChanges$) as ReadonlyArray<{
		scenarioId: string;
	}>;
	const [open, setOpen] = useState(false);
	const [creating, setCreating] = useState(false);
	const [newName, setNewName] = useState("");
	const [confirmPromote, setConfirmPromote] = useState(false);
	const ref = useRef<HTMLDivElement>(null);

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

	const setClosed = useCallback(() => {
		setOpen(false);
		setCreating(false);
		setConfirmPromote(false);
	}, []);

	// Close on outside click / Escape.
	useEffect(() => {
		if (!open) return;
		const onDown = (e: MouseEvent) => {
			if (ref.current && !ref.current.contains(e.target as Node)) setClosed();
		};
		const onKey = (e: KeyboardEvent) => {
			if (e.key === "Escape") setClosed();
		};
		document.addEventListener("mousedown", onDown);
		document.addEventListener("keydown", onKey);
		return () => {
			document.removeEventListener("mousedown", onDown);
			document.removeEventListener("keydown", onKey);
		};
	}, [open, setClosed]);

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
		setOpen(false);
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
			setClosed();
			onActivate(null);
			onMutated();
		},
		[store, activeScenarioId, onActivate, onMutated, setClosed],
	);

	const isActive = active != null;
	const accent = isActive ? "var(--cyan)" : "var(--border)";

	return (
		<div ref={ref} style={{ position: "relative", display: "inline-flex" }}>
			<button
				type="button"
				className="mono"
				onClick={() => setOpen((v) => !v)}
				title="cenários de projeção (what-if)"
				style={{
					display: "inline-flex",
					alignItems: "center",
					gap: 6,
					border: `1px solid ${accent}`,
					borderRadius: "var(--radius-full)",
					padding: "6px 12px",
					fontSize: 12,
					cursor: "pointer",
					background: isActive ? "rgba(8,145,178,0.08)" : "var(--card)",
					color: isActive ? "var(--cyan)" : "var(--muted)",
				}}
			>
				<span aria-hidden>🧪</span>
				<span style={{ fontWeight: isActive ? 600 : 400 }}>
					{active ? active.name : "baseline"}
				</span>
				{isActive && scenarioDelta != null && (
					<span style={{ opacity: 0.85 }}>
						· Δ {formatMoneyNumber(scenarioDelta)}
					</span>
				)}
				<span aria-hidden style={{ opacity: 0.6 }}>
					▾
				</span>
			</button>

			{open && (
				<div
					role="menu"
					aria-label="cenários de projeção"
					style={{
						position: "absolute",
						top: "calc(100% + 6px)",
						left: 0,
						zIndex: 50,
						minWidth: 240,
						background: "var(--bg)",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-md)",
						boxShadow: "0 8px 28px rgba(0,0,0,0.28)",
						padding: 6,
					}}
				>
					<MenuItem
						label="baseline"
						hint="plano real"
						selected={activeScenarioId == null}
						onClick={() => {
							onActivate(null);
							setClosed();
						}}
					/>
					{pickable.map((s) => (
						<MenuItem
							key={s.scenarioId}
							label={s.name}
							hint={s.description ?? "cenário"}
							selected={s.scenarioId === activeScenarioId}
							onClick={() => {
								onActivate(s.scenarioId);
								setClosed();
							}}
						/>
					))}

					<div
						style={{
							height: 1,
							background: "var(--border)",
							margin: "6px 2px",
						}}
					/>

					{/* New scenario */}
					{canCreate ? (
						creating ? (
							<div style={{ display: "flex", gap: 4, padding: "2px 4px" }}>
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
								<button
									onClick={createScenario}
									className="mono"
									style={miniBtn("var(--purple)")}
								>
									criar
								</button>
							</div>
						) : (
							<button
								onClick={() => setCreating(true)}
								className="mono"
								style={menuActionStyle}
							>
								+ novo cenário
							</button>
						)
					) : (
						<div
							className="mono"
							style={{
								fontSize: 11,
								color: "var(--muted)",
								padding: "6px 8px",
							}}
						>
							cenários só a partir do mês atual
						</div>
					)}

					{/* Active-scenario lifecycle */}
					{isActive && (
						<>
							<div
								style={{
									height: 1,
									background: "var(--border)",
									margin: "6px 2px",
								}}
							/>
							<div
								className="mono"
								style={{
									fontSize: 11,
									color: "var(--cyan)",
									padding: "2px 8px 6px",
								}}
							>
								{changeCount} mudança{changeCount === 1 ? "" : "s"} · edições
								nesta planilha viram mudanças do cenário
							</div>
							{confirmPromote ? (
								<div style={{ display: "flex", gap: 4, padding: "2px 4px" }}>
									<button
										onClick={() => lifecycle("promote")}
										className="mono"
										style={miniBtn("var(--cyan)")}
									>
										confirmar aplicar {changeCount}
									</button>
									<button
										onClick={() => setConfirmPromote(false)}
										className="mono"
										style={miniBtn("var(--border)")}
									>
										cancelar
									</button>
								</div>
							) : (
								<div style={{ display: "flex", gap: 4, padding: "2px 4px" }}>
									<button
										onClick={() => setConfirmPromote(true)}
										disabled={changeCount === 0}
										className="mono"
										style={{
											...menuActionStyle,
											color: "var(--cyan)",
											opacity: changeCount === 0 ? 0.4 : 1,
											flex: 1,
										}}
									>
										▶ promover ao plano
									</button>
									<button
										onClick={() => lifecycle("delete")}
										className="mono"
										style={{ ...menuActionStyle, color: "var(--rose)", flex: 1 }}
									>
										descartar
									</button>
								</div>
							)}
						</>
					)}
				</div>
			)}
		</div>
	);
};

const MenuItem = ({
	label,
	hint,
	selected,
	onClick,
}: {
	label: string;
	hint: string;
	selected: boolean;
	onClick: () => void;
}) => (
	<button
		role="menuitemradio"
		aria-checked={selected}
		onClick={onClick}
		className="mono"
		style={{
			display: "flex",
			width: "100%",
			alignItems: "center",
			justifyContent: "space-between",
			gap: 10,
			border: "none",
			borderRadius: "var(--radius-sm)",
			padding: "6px 8px",
			fontSize: 12,
			cursor: "pointer",
			background: selected ? "var(--purple)" : "transparent",
			color: selected ? "#fff" : "var(--text)",
		}}
	>
		<span>{label}</span>
		<span style={{ fontSize: 10, opacity: 0.6 }}>{selected ? "●" : hint}</span>
	</button>
);

const menuActionStyle: React.CSSProperties = {
	display: "block",
	width: "100%",
	border: "none",
	borderRadius: "var(--radius-sm)",
	padding: "6px 8px",
	fontSize: 12,
	textAlign: "left",
	cursor: "pointer",
	background: "transparent",
	color: "var(--muted)",
	fontFamily: "var(--font-mono)",
};

const miniBtn = (color: string): React.CSSProperties => ({
	background: color === "var(--border)" ? "transparent" : color,
	color: color === "var(--border)" ? "var(--muted)" : "#fff",
	border: `1px solid ${color}`,
	borderRadius: "var(--radius-sm)",
	padding: "5px 10px",
	cursor: "pointer",
	fontSize: 11,
	fontFamily: "var(--font-mono)",
	whiteSpace: "nowrap",
});

const inputStyle: React.CSSProperties = {
	background: "var(--bg)",
	color: "var(--text)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "5px 8px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
	outline: "none",
	flex: 1,
	minWidth: 0,
};
