import { queryDb } from "@livestore/livestore";
import { useQuery, useStore } from "@livestore/react";
import { useEffect, useMemo, useState } from "react";
import { events, tables } from "../../livestore/schema";
import { categoryEmoji } from "../../lib/categoryEmoji";
import { formatMoneyNumber } from "../../lib/format";
import {
	buildEnvelopeWrites,
	buildOverlayMap,
	buildWarPlan,
	simulateWarPlanGoals,
	type TxView,
	type WarPlanRow,
	type WarPlanSubRow,
} from "../../lib/derivations";
import type { ChartSimulation } from "../chart/model";
import type { ForecastView } from "../types";

const txAll$ = queryDb(tables.transactions);
const overlay$ = queryDb(tables.reviewOverlay);

/**
 * Plano de guerra — the goal-setting workbench for one month. Per parent
 * category it lines up the budget envelope, the realized spend, the 3-month
 * average and the projection (`max(realizado, orçamento)` — the same envelope
 * model the chart uses). Each SUBcategory carries a goal slider that opens at
 * its 3-month average (floored at what is already spent); dragging recomputes
 * the month + rest-of-year projection live, in the panel and in the annual
 * chart above (via `onSimulationChange`). Confirming persists the touched
 * parents' goals as monthly budget envelopes — plain `kind=manual` forecasts
 * upserted through the bridge, so `phai forecast list` and the OpenClaw
 * follow-ups see exactly what was saved here.
 */
export const WarPlanPanel = ({
	month,
	forecasts,
	isPast,
	allForecasts,
	persistMonths,
	onSimulationChange,
	onSaved,
}: {
	month: string;
	forecasts: ForecastView[];
	isPast: boolean;
	/** Every seeded forecast (all months) — needed to find envelopes to update. */
	allForecasts: ForecastView[];
	/** Months ("YYYY-MM") a confirmed goal writes envelopes for. */
	persistMonths: ReadonlyArray<string>;
	/** Live goal simulation for the annual chart (null = no active goals). */
	onSimulationChange: (sim: ChartSimulation | null) => void;
	onSaved: () => void;
}) => {
	const { store } = useStore();
	const txRows = useQuery(txAll$) as ReadonlyArray<TxView>;
	const overlay = useQuery(overlay$);
	const overlayMap = useMemo(() => buildOverlayMap(overlay), [overlay]);

	const plan = useMemo(
		() =>
			buildWarPlan(
				txRows,
				month,
				forecasts.map((f) => ({
					amount: f.amount,
					categoryId: f.categoryId,
					kind: f.kind,
					status: f.status,
					month: f.month,
				})),
				overlayMap,
				isPast ? "past" : "open",
			),
		[txRows, month, forecasts, overlayMap, isPast],
	);

	// Goal sliders, keyed by the sub's categoryId. Reset when the month changes.
	const [goals, setGoals] = useState<Map<string, number>>(new Map());
	const [arming, setArming] = useState(false);
	useEffect(() => {
		setGoals(new Map());
		setArming(false);
	}, [month]);

	const sim = useMemo(() => simulateWarPlanGoals(plan, goals), [plan, goals]);
	const hasGoals = sim.goalByParent.size > 0;

	// Feed the live simulation to the annual chart; clear it on unmount.
	useEffect(() => {
		onSimulationChange(
			!isPast && goals.size > 0
				? { fromMonth: month, monthlySaving: sim.economiaMes }
				: null,
		);
		return () => onSimulationChange(null);
	}, [onSimulationChange, isPast, goals.size, month, sim.economiaMes]);

	const setGoal = (categoryId: string, value: number) => {
		setArming(false);
		setGoals((prev) => {
			const next = new Map(prev);
			next.set(categoryId, value);
			return next;
		});
	};

	const saveGoals = () => {
		const writes = buildEnvelopeWrites(
			sim.goalByParent,
			persistMonths,
			allForecasts.map((f) => ({
				forecastId: f.forecastId,
				amount: f.amount,
				categoryId: f.categoryId,
				kind: f.kind,
				status: f.status,
				month: f.month,
			})),
		);
		if (writes.length === 0) return;
		store.commit(
			...writes.map((w) =>
				events.forecastEnvelopeUpserted({
					writeId: crypto.randomUUID(),
					forecastId: w.forecastId ?? "",
					description: w.description,
					amount: w.amount,
					dueDate: w.dueDate,
					categoryId: w.categoryId,
					upsertedAt: Date.now(),
				}),
			),
		);
		setGoals(new Map());
		setArming(false);
		onSaved();
	};

	const canSave = hasGoals && !isPast && persistMonths.length > 0;
	const maxProjecao = Math.max(1, ...plan.rows.map((r) => r.projecao));

	return (
		<section aria-label={`Planning for ${month}`}>
			<PlanSummary
				plan={plan}
				sim={sim}
				hasGoals={hasGoals}
				monthsCount={Math.max(1, persistMonths.length)}
			/>

			<div
				style={{
					border: "1px solid var(--border)",
					borderRadius: "var(--radius-md)",
					overflow: "auto",
					background: "var(--card)",
				}}
			>
				<table
					style={{
						width: "100%",
						// "separate": collapsed borders make body cells paint through
						// the sticky header while scrolling.
						borderCollapse: "separate",
						borderSpacing: 0,
						fontSize: 14,
					}}
				>
					<thead>
						<tr className="mono">
							<th style={thStyle}>category</th>
							<th style={{ ...thStyle, textAlign: "right" }}>budget</th>
							<th style={{ ...thStyle, textAlign: "right" }}>spent</th>
							<th style={{ ...thStyle, width: "24%" }}>usage · goal</th>
							<th style={{ ...thStyle, textAlign: "right" }}>3-mo avg</th>
							<th style={{ ...thStyle, textAlign: "right" }}>projection</th>
							<th style={{ ...thStyle, textAlign: "right" }}>goal</th>
							<th style={{ ...thStyle, textAlign: "right" }}>Δ month</th>
						</tr>
					</thead>
					<tbody>
						{plan.rows.map((row) => (
							<ParentRows
								key={row.parent}
								row={row}
								maxProjecao={maxProjecao}
								goals={goals}
								goal={sim.goalByParent.get(row.parent)}
								simulated={sim.simulatedByParent.get(row.parent) ?? row.projecao}
								disabled={isPast}
								onGoal={setGoal}
							/>
						))}
					</tbody>
				</table>
			</div>

			<PlanFooter
				parcelas={plan.parcelasComprometidas}
				showClear={goals.size > 0}
				canSave={canSave}
				arming={arming}
				parentCount={sim.goalByParent.size}
				monthCount={persistMonths.length}
				onClear={() => {
					setGoals(new Map());
					setArming(false);
				}}
				onArm={() => setArming(true)}
				onSave={saveGoals}
			/>
		</section>
	);
};

const PlanSummary = ({
	plan,
	sim,
	hasGoals,
	monthsCount,
}: {
	plan: ReturnType<typeof buildWarPlan>;
	sim: ReturnType<typeof simulateWarPlanGoals>;
	hasGoals: boolean;
	monthsCount: number;
}) => {
	const deltaAccent = hasGoals
		? sim.economiaMes >= 0
			? "var(--green)"
			: "var(--rose)"
		: undefined;
	return (
		<div
			style={{
				display: "grid",
				gridTemplateColumns: "repeat(auto-fit, minmax(160px, 1fr))",
				gap: 12,
				padding: "12px 0",
			}}
		>
			<SummaryCard label="month projection" value={plan.totalProjecao} />
			<SummaryCard label="already spent" value={plan.totalRealizado} />
			<SummaryCard
				label="with goals"
				value={sim.projecaoSimulada}
				accent={hasGoals ? "var(--purple)" : undefined}
			/>
			<SummaryCard label="Δ / month" value={sim.economiaMes} accent={deltaAccent} />
			<SummaryCard
				label={`Δ through Dec (×${monthsCount})`}
				value={sim.economiaMes * monthsCount}
				accent={deltaAccent}
			/>
		</div>
	);
};

const footerButton = (kind: "muted" | "outline" | "solid"): React.CSSProperties => ({
	background: kind === "solid" ? "var(--purple)" : "transparent",
	color:
		kind === "solid" ? "#fff" : kind === "outline" ? "var(--purple)" : "var(--muted)",
	border: kind === "solid" ? "none" : `1px solid ${kind === "outline" ? "var(--purple)" : "var(--border)"}`,
	borderRadius: "var(--radius-full)",
	padding: "4px 14px",
	cursor: "pointer",
	fontSize: 11,
});

const PlanFooter = ({
	parcelas,
	showClear,
	canSave,
	arming,
	parentCount,
	monthCount,
	onClear,
	onArm,
	onSave,
}: {
	parcelas: number;
	showClear: boolean;
	canSave: boolean;
	arming: boolean;
	parentCount: number;
	monthCount: number;
	onClear: () => void;
	onArm: () => void;
	onSave: () => void;
}) => (
	<div
		className="mono"
		style={{
			display: "flex",
			gap: 16,
			alignItems: "center",
			padding: "10px 4px",
			fontSize: 12,
			color: "var(--muted)",
			flexWrap: "wrap",
		}}
	>
		{parcelas > 0 && (
			<span>
				installments already committed this month: {formatMoneyNumber(parcelas)}
				(inside their categories once paid)
			</span>
		)}
		<span>
			card bills are not counted here — their purchases already live in the categories.
		</span>
		{showClear && (
			<button
				onClick={onClear}
				className="mono"
				style={{ marginLeft: "auto", ...footerButton("muted") }}
			>
				reset goals
			</button>
		)}
		{canSave &&
			(arming ? (
				<button onClick={onSave} className="mono" style={footerButton("solid")}>
					confirm {parentCount} {parentCount === 1 ? "category" : "categories"}{" "}
					× {monthCount} {monthCount === 1 ? "month" : "months"}
				</button>
			) : (
				<button onClick={onArm} className="mono" style={footerButton("outline")}>
					save goals through Dec
				</button>
			))}
	</div>
);

const SummaryCard = ({
	label,
	value,
	accent,
}: {
	label: string;
	value: number;
	accent?: string;
}) => (
	<div
		style={{
			border: "1px solid var(--border)",
			borderRadius: "var(--radius-md)",
			padding: "10px 14px",
			background: "var(--card)",
		}}
	>
		<div
			className="mono"
			style={{
				fontSize: 11,
				textTransform: "uppercase",
				letterSpacing: "0.08em",
				color: "var(--muted)",
			}}
		>
			{label}
		</div>
		<div
			style={{
				fontFamily: "var(--font-display)",
				fontSize: "1.25rem",
				color: accent ?? "var(--text)",
				fontVariantNumeric: "tabular-nums",
			}}
		>
			{formatMoneyNumber(value)}
		</div>
	</div>
);

/** Right-aligned Δ: "−X" = saving (green), "+X" = increase (rose), "—" idle. */
const DeltaCell = ({
	active,
	delta,
	small,
}: {
	active: boolean;
	delta: number;
	small?: boolean;
}) => (
	<td
		className="mono"
		style={{
			...tdStyle,
			textAlign: "right",
			fontSize: small ? 12 : undefined,
			color:
				!active || delta === 0
					? "var(--muted)"
					: delta > 0
						? "var(--green)"
						: "var(--rose)",
		}}
	>
		{active && delta !== 0
			? `${delta > 0 ? "−" : "+"}${formatMoneyNumber(Math.abs(delta))}`
			: "—"}
	</td>
);

/** One parent category: the summary row plus a goal-slider row per sub. */
const ParentRows = ({
	row,
	maxProjecao,
	goals,
	goal,
	simulated,
	disabled,
	onGoal,
}: {
	row: WarPlanRow;
	maxProjecao: number;
	goals: ReadonlyMap<string, number>;
	/** Confirmed envelope goal for this parent (set once any sub is touched). */
	goal: number | undefined;
	simulated: number;
	disabled: boolean;
	onGoal: (categoryId: string, value: number) => void;
}) => {
	const overBudget = row.orcamento != null && row.realizado > row.orcamento;
	const delta = row.projecao - simulated;
	// Uncategorized spend can't carry a budget — no sliders for "—".
	const slidable = !disabled && row.parent !== "—";

	return (
		<>
			<tr>
				<td style={tdStyle}>
					<span style={{ fontWeight: 500 }}>
						{categoryEmoji(row.parent)} {row.parent}
					</span>
				</td>
				<td className="mono" style={{ ...tdStyle, textAlign: "right" }}>
					{row.orcamento != null ? formatMoneyNumber(row.orcamento) : "—"}
				</td>
				<td
					className="mono"
					style={{
						...tdStyle,
						textAlign: "right",
						color: overBudget ? "var(--rose)" : "var(--text)",
					}}
				>
					{formatMoneyNumber(row.realizado)}
				</td>
				<td style={tdStyle}>
					<div
						aria-hidden
						style={{
							position: "relative",
							height: 8,
							borderRadius: 4,
							background: "var(--border)",
							overflow: "hidden",
						}}
					>
						{/* Scale bars against the biggest projection so categories compare visually. */}
						<div
							style={{
								position: "absolute",
								inset: 0,
								width: `${(row.projecao / maxProjecao) * 100}%`,
								background: "var(--chip, rgba(124,93,250,0.18))",
								borderRadius: 4,
							}}
						/>
						<div
							style={{
								position: "absolute",
								inset: 0,
								width: `${(row.realizado / maxProjecao) * 100}%`,
								background: overBudget ? "var(--rose)" : "var(--purple)",
								borderRadius: 4,
								opacity: 0.85,
							}}
						/>
					</div>
					{row.orcamento != null && row.orcamento > 0 && (
						<div className="mono" style={{ fontSize: 11, color: "var(--muted)" }}>
							{Math.round((row.realizado / row.orcamento) * 100)}% of budget
						</div>
					)}
				</td>
				<td
					className="mono"
					style={{ ...tdStyle, textAlign: "right", color: "var(--muted)" }}
				>
					{row.media3m > 0 ? formatMoneyNumber(row.media3m) : "—"}
				</td>
				<td className="mono" style={{ ...tdStyle, textAlign: "right", fontWeight: 600 }}>
					{formatMoneyNumber(row.projecao)}
				</td>
				<td
					className="mono"
					style={{
						...tdStyle,
						textAlign: "right",
						color: goal != null ? "var(--purple)" : "var(--muted)",
						fontWeight: goal != null ? 600 : 400,
					}}
				>
					{goal != null ? formatMoneyNumber(goal) : "—"}
				</td>
				<DeltaCell active={goal != null} delta={delta} />
			</tr>
			{slidable &&
				row.subs.map((sub) => (
					<SubGoalRow
						key={sub.categoryId}
						sub={sub}
						goal={goals.get(sub.categoryId)}
						onGoal={onGoal}
					/>
				))}
		</>
	);
};

/** Round a slider ceiling up to a friendly increment. */
const sliderMax = (sub: WarPlanSubRow): number =>
	Math.max(100, Math.ceil((Math.max(sub.goalBase, sub.realizado) * 1.5) / 50) * 50);

const SubGoalRow = ({
	sub,
	goal,
	onGoal,
}: {
	sub: WarPlanSubRow;
	goal: number | undefined;
	onGoal: (categoryId: string, value: number) => void;
}) => {
	const value = goal ?? sub.goalBase;
	const floored = value < sub.realizado;
	const saving = goal != null ? sub.goalBase - goal : 0;

	return (
		<tr style={{ background: "var(--bg)" }}>
			<td style={{ ...tdStyle, paddingLeft: 28, fontSize: 12, color: "var(--muted)" }}>
				↳ {sub.sub === "—" ? "(geral)" : sub.sub}
			</td>
			<td style={tdStyle} />
			<td
				className="mono"
				style={{ ...tdStyle, textAlign: "right", fontSize: 12, color: "var(--muted)" }}
			>
				{sub.realizado > 0 ? formatMoneyNumber(sub.realizado) : "—"}
			</td>
			<td style={tdStyle}>
				<input
					type="range"
					min={0}
					max={sliderMax(sub)}
					step={10}
					value={value}
					aria-label={`monthly goal for ${sub.categoryId}`}
					onChange={(e) => onGoal(sub.categoryId, Number(e.target.value))}
					style={{ width: "100%", accentColor: floored ? "var(--amber)" : "var(--purple)" }}
					title={
						floored
							? "below what is already spent — spent is the floor"
							: "drag to set the monthly goal"
					}
				/>
			</td>
			<td
				className="mono"
				style={{ ...tdStyle, textAlign: "right", fontSize: 12, color: "var(--muted)" }}
			>
				{sub.media3m > 0 ? formatMoneyNumber(sub.media3m) : "—"}
			</td>
			<td style={tdStyle} />
			<td
				className="mono"
				style={{
					...tdStyle,
					textAlign: "right",
					fontSize: 12,
					color: floored
						? "var(--amber)"
						: goal != null
							? "var(--purple)"
							: "var(--muted)",
				}}
			>
				{formatMoneyNumber(value)}
			</td>
			<DeltaCell active={goal != null} delta={saving} small />
		</tr>
	);
};

const thStyle: React.CSSProperties = {
	padding: "8px 12px",
	textAlign: "left",
	fontWeight: 500,
	fontSize: 12,
	textTransform: "uppercase",
	letterSpacing: "0.06em",
	color: "var(--muted)",
	// Sticky on the th (not the tr): collapsed-border sticky rows don't paint
	// their background reliably, so each header cell carries its own.
	position: "sticky",
	top: 0,
	zIndex: 2,
	background: "var(--card)",
	boxShadow: "0 1px 0 var(--border)",
};

const tdStyle: React.CSSProperties = {
	padding: "8px 12px",
	verticalAlign: "middle",
	// Row separator on the td: tr borders don't render with
	// border-collapse: separate (which the sticky header requires).
	borderBottom: "1px solid var(--border)",
};
