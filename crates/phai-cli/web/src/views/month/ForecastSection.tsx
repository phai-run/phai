import { useCallback, useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useClientDocument, useStore } from "@livestore/react";
import { events, tables } from "../../livestore/schema";
import { amountColor, formatMoney } from "../../lib/format";
import { useDnd } from "../../lib/dnd";
import { ToggleBtn } from "./MonthFilters";
import type { ChartMonthView, ForecastView } from "../types";

// ── Forecast section ───────────────────────────────────────────────────────

export const ForecastSection = ({
	month,
	forecasts,
	onAdded,
	months,
	onMoveForecast,
}: {
	month: string;
	forecasts: ForecastView[];
	onAdded: () => void;
	months: ReadonlyArray<ChartMonthView>;
	onMoveForecast: (forecastId: string, targetMonth: string) => void;
}) => {
	const { store } = useStore();
	const [ui] = useClientDocument(tables.ui);
	// Active planning scenario (ADR-0037): while set, edits on future months
	// become scenario deltas instead of real forecast writes.
	const activeScenarioId = ui.activeScenarioId ?? null;
	const [open, setOpen] = useState(false);
	const [addOpen, setAddOpen] = useState(false);
	const [description, setDescription] = useState("");
	const [amount, setAmount] = useState("");
	const [outflow, setOutflow] = useState(true);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [movingId, setMovingId] = useState<string | null>(null);
	const [pickerOpen, setPickerOpen] = useState(false);
	const [adjustValue, setAdjustValue] = useState("");
	const { startDrag, dragging } = useDnd();

	// Allowed target months: current month + any future months (no past).
	const currentMonth = useMemo(() => {
		const d = new Date();
		return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
	}, []);
	const allowedMonths = useMemo(
		() => months.filter((m) => m.month >= currentMonth),
		[months, currentMonth],
	);

	const toggleOpen = useCallback(() => setOpen((v) => !v), []);
	const openAdd = useCallback(() => setAddOpen(true), []);
	const closeAdd = useCallback(() => setAddOpen(false), []);
	const setOut = useCallback(() => setOutflow(true), []);
	const setIn = useCallback(() => setOutflow(false), []);

	// Move selected forecast to target month. Animates briefly.
	const doMove = useCallback(
		(forecastId: string, targetMonth: string) => {
			setMovingId(forecastId);
			onMoveForecast(forecastId, targetMonth);
			setSelectedId(null);
			setPickerOpen(false);
			setTimeout(() => setMovingId(null), 400);
		},
		[onMoveForecast],
	);

	// Shift selected forecast by one allowed month.
	const shiftMonth = useCallback(
		(direction: -1 | 1) => {
			if (!selectedId) return;
			const f = forecasts.find((x) => x.forecastId === selectedId);
			if (!f || f.draggable !== 1) return;
			const current = f.month ?? month;
			const curIdx = allowedMonths.findIndex((m) => m.month >= current);
			if (curIdx === -1) return;
			const targetIdx = curIdx + direction;
			if (targetIdx < 0 || targetIdx >= allowedMonths.length) return;
			doMove(selectedId, allowedMonths[targetIdx].month);
		},
		[selectedId, forecasts, month, allowedMonths, doMove],
	);

	// Keyboard handler for forecast rows.
	const handleForecastKeyDown = useCallback(
		(e: React.KeyboardEvent, forecastId: string) => {
			const f = forecasts.find((x) => x.forecastId === forecastId);
			if (!f) return;
			const mod = e.ctrlKey || e.metaKey;
			if (mod && e.key === "ArrowLeft") {
				e.preventDefault();
				shiftMonth(-1);
			} else if (mod && e.key === "ArrowRight") {
				e.preventDefault();
				shiftMonth(1);
			} else if (mod && (e.key === "m" || e.key === "M")) {
				e.preventDefault();
				if (f.draggable === 1) setPickerOpen(true);
			} else if (e.key === "Enter" || e.key === " ") {
				e.preventDefault();
				setSelectedId((prev) => (prev === forecastId ? null : forecastId));
			}
		},
		[forecasts, shiftMonth],
	);

	const submitForecast = useCallback(() => {
		const desc = description.trim();
		const mag = amount.replace(/^-/, "").trim();
		if (!desc || !mag) return;
		const signed = outflow ? `-${mag}` : mag;
		if (activeScenarioId) {
			// Scenario mode: a new entry is an add_one_shot delta, not a real
			// forecast — it only lands when the scenario is promoted.
			store.commit(
				events.scenarioChangeAdded({
					writeId: crypto.randomUUID(),
					row: {
						changeId: `chg-${crypto.randomUUID()}`,
						scenarioId: activeScenarioId,
						kind: "add_one_shot",
						targetForecastId: null,
						targetTemplateId: null,
						month,
						effectiveFrom: null,
						amount: signed,
						monthsCount: null,
						description: desc,
						categoryId: null,
						accountId: null,
						status: "ativo",
						orphaned: 0,
					},
					addedAt: Date.now(),
				}),
			);
		} else {
			store.commit(
				events.forecastCreated({
					writeId: crypto.randomUUID(),
					description: desc,
					amount: signed,
					dueDate: `${month}-01`,
					categoryId: null,
					accountId: null,
					uiRole: null,
					createdAt: Date.now(),
				}),
			);
		}
		setDescription("");
		setAmount("");
		setAddOpen(false);
		onAdded();
	}, [description, amount, outflow, month, store, onAdded, activeScenarioId]);

	// Scenario deltas targeting the selected forecast (ADR-0037).
	const commitScenarioChange = useCallback(
		(
			kind: "adjust_amount" | "skip_forecast" | "end_template",
			forecast: ForecastView,
			newAmount?: string,
		) => {
			if (!activeScenarioId) return;
			store.commit(
				events.scenarioChangeAdded({
					writeId: crypto.randomUUID(),
					row: {
						changeId: `chg-${crypto.randomUUID()}`,
						scenarioId: activeScenarioId,
						kind,
						targetForecastId: kind === "end_template" ? null : forecast.forecastId,
						targetTemplateId: kind === "end_template" ? forecast.templateId : null,
						month: null,
						effectiveFrom: kind === "end_template" ? month : null,
						amount: kind === "adjust_amount" ? (newAmount ?? null) : null,
						monthsCount: null,
						description: forecast.description,
						categoryId: forecast.categoryId ?? null,
						accountId: forecast.accountId ?? null,
						status: "ativo",
						orphaned: 0,
					},
					addedAt: Date.now(),
				}),
			);
			setSelectedId(null);
			setAdjustValue("");
			onAdded();
		},
		[activeScenarioId, month, store, onAdded],
	);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Enter") submitForecast();
		},
		[submitForecast],
	);

	if (forecasts.length === 0 && !addOpen) {
		return (
			<div style={{ padding: "10px 0" }}>
				<button onClick={openAdd} className="mono" style={addBtnStyle}>
					+ new forecast
				</button>
			</div>
		);
	}

	return (
		<div
			style={{
				borderBottom: "1px solid var(--border)",
				padding: "10px 0 12px",
			}}
		>
			<button
				onClick={toggleOpen}
				className="mono"
				style={{
					background: "transparent",
					border: "none",
					cursor: "pointer",
					fontSize: 11,
					color: "var(--muted)",
					padding: 0,
					display: "flex",
					alignItems: "center",
					gap: 6,
				}}
			>
				<span>{open ? "▾" : "▸"}</span>
				<span style={{ color: "var(--cyan)" }}>
					{forecasts.length} forecast{forecasts.length !== 1 ? "s" : ""}
				</span>
				<span>for {month}</span>
			</button>

			<AnimatePresence>
				{open && (
					<motion.div
						initial={{ opacity: 0, height: 0 }}
						animate={{ opacity: 1, height: "auto" }}
						exit={{ opacity: 0, height: 0 }}
						style={{ overflow: "hidden" }}
					>
						<div
							style={{
								display: "flex",
								flexDirection: "column",
								gap: 6,
								marginTop: 10,
							}}
						>
							{forecasts.map((f) => {
								const locked = f.draggable !== 1;
								const isDragging = dragging?.forecastId === f.forecastId;
								const isSelected = selectedId === f.forecastId;
								const isMoving = movingId === f.forecastId;
								const lockReason =
									f.kind === "installment"
										? "installment — locked"
										: f.kind === "subscription"
											? "subscription — locked"
											: null;
								return (
									<div
										key={f.forecastId}
										tabIndex={0}
										role="option"
										aria-selected={isSelected}
										aria-label={`forecast ${f.description}${locked ? " — " + lockReason : ""}`}
										onClick={() => {
											setSelectedId((prev) =>
												prev === f.forecastId ? null : f.forecastId,
											);
										}}
										onKeyDown={(e) => handleForecastKeyDown(e, f.forecastId)}
										onPointerDown={(e) => {
											if (locked || e.button !== 0) return;
											startDrag(
												{
													kind: "forecast",
													forecastId: f.forecastId,
													label: f.description,
													amount: formatMoney(f.amount),
												},
												e,
											);
										}}
										title={
											locked
												? (lockReason ?? "locked")
												: !isSelected
													? "click to select; drag to another month"
													: "Ctrl+←/→ moves month; Ctrl+M opens picker"
										}
										style={{
											display: "flex",
											justifyContent: "space-between",
											alignItems: "center",
											gap: 10,
											padding: "6px 10px",
											borderRadius: "var(--radius-sm)",
											border: isSelected
												? "1px solid var(--purple)"
												: "1px dashed var(--border)",
											background:
												f.kind === "manual" ? "transparent" : "var(--surface)",
											cursor: locked ? "default" : "grab",
											opacity: isDragging || isMoving ? 0.35 : 1,
											touchAction: "none",
											userSelect: "none",
											transition: "opacity 150ms, border-color 120ms",
										}}
									>
										<span
											style={{
												display: "flex",
												gap: 6,
												alignItems: "center",
												minWidth: 0,
											}}
										>
											<span
												className="mono"
												style={{ color: "var(--muted)", fontSize: 11 }}
											>
												{locked ? "⊘" : "⠿"}
											</span>
											<span
												style={{
													fontSize: 13,
													overflow: "hidden",
													textOverflow: "ellipsis",
													whiteSpace: "nowrap",
												}}
											>
												{f.description}
											</span>
										</span>
										<span
											className="mono"
											style={{
												color: amountColor(f.amount),
												fontSize: 13,
												whiteSpace: "nowrap",
											}}
										>
											{formatMoney(f.amount)}
										</span>
									</div>
								);
							})}
						</div>
					</motion.div>
				)}
			</AnimatePresence>

			{/* Scenario actions on the selected forecast (ADR-0037) */}
			<AnimatePresence>
				{activeScenarioId &&
					selectedId &&
					(() => {
						const f = forecasts.find((x) => x.forecastId === selectedId);
						if (!f) return null;
						return (
							<motion.div
								initial={{ opacity: 0, height: 0 }}
								animate={{ opacity: 1, height: "auto" }}
								exit={{ opacity: 0, height: 0 }}
								style={{ overflow: "hidden", marginTop: 8 }}
							>
								<div
									style={{
										display: "flex",
										gap: 6,
										flexWrap: "wrap",
										alignItems: "center",
										padding: 8,
										border: "1px dashed var(--cyan)",
										borderRadius: "var(--radius-sm)",
									}}
								>
									<span
										className="mono"
										style={{ fontSize: 11, color: "var(--cyan)" }}
									>
										🧪 no cenário:
									</span>
									<button
										onClick={() => commitScenarioChange("skip_forecast", f)}
										className="mono"
										style={pillStyle}
									>
										pular este
									</button>
									<span style={{ display: "inline-flex", gap: 4 }}>
										<input
											inputMode="decimal"
											placeholder="novo valor"
											value={adjustValue}
											onChange={(e) => setAdjustValue(e.target.value)}
											onKeyDown={(e) => {
												if (e.key !== "Enter") return;
												const mag = adjustValue.replace(/^-/, "").trim();
												if (mag)
													commitScenarioChange("adjust_amount", f, `-${mag}`);
											}}
											className="mono"
											style={{ ...inputStyle, width: 90 }}
										/>
										<button
											onClick={() => {
												const mag = adjustValue.replace(/^-/, "").trim();
												if (mag)
													commitScenarioChange("adjust_amount", f, `-${mag}`);
											}}
											disabled={!adjustValue.trim()}
											className="mono"
											style={{
												...pillStyle,
												opacity: adjustValue.trim() ? 1 : 0.4,
											}}
										>
											ajustar
										</button>
									</span>
									{f.templateId && (
										<button
											onClick={() => commitScenarioChange("end_template", f)}
											className="mono"
											style={pillStyle}
											title="a recorrência para de gerar previsões a partir deste mês"
										>
											encerrar desde {month}
										</button>
									)}
								</div>
							</motion.div>
						);
					})()}
			</AnimatePresence>

			{/* Add forecast inline form */}
			<AnimatePresence>
				{addOpen ? (
					<motion.div
						initial={{ opacity: 0, height: 0 }}
						animate={{ opacity: 1, height: "auto" }}
						exit={{ opacity: 0, height: 0 }}
						style={{ overflow: "hidden", marginTop: 8 }}
					>
						<div
							style={{
								display: "flex",
								flexDirection: "column",
								gap: 8,
								padding: 10,
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
							}}
						>
							<input
								autoFocus
								placeholder="forecast description"
								value={description}
								onChange={(e) => setDescription(e.target.value)}
								className="mono"
								style={inputStyle}
							/>
							<div
								style={{
									display: "flex",
									gap: 8,
									alignItems: "center",
								}}
							>
								<ToggleBtn
									active={outflow}
									color="var(--rose)"
									onClick={setOut}
								>
									expense
								</ToggleBtn>
								<ToggleBtn
									active={!outflow}
									color="var(--green)"
									onClick={setIn}
								>
									income
								</ToggleBtn>
								<input
									inputMode="decimal"
									placeholder="0,00"
									value={amount}
									onChange={(e) => setAmount(e.target.value)}
									onKeyDown={handleKeyDown}
									className="mono"
									style={{ ...inputStyle, width: 100 }}
								/>

							</div>
							<div style={{ display: "flex", gap: 8 }}>
								<button
									onClick={submitForecast}
									disabled={!description.trim() || !amount.trim()}
									className="mono"
									style={{
										...pillStyle,
										background: "var(--cyan)",
										color: "#fff",
										opacity: !description.trim() || !amount.trim() ? 0.4 : 1,
									}}
								>
									add →
								</button>
								<button onClick={closeAdd} className="mono" style={pillStyle}>
									cancel
								</button>
							</div>
						</div>
					</motion.div>
				) : (
					<motion.div
						initial={{ opacity: 0 }}
						animate={{ opacity: 1 }}
						style={{ marginTop: 8 }}
					>
						<button onClick={openAdd} className="mono" style={addBtnStyle}>
							+ new forecast
						</button>
					</motion.div>
				)}
			</AnimatePresence>

			{/* Month picker popover for keyboard move (Ctrl+M) */}
			<AnimatePresence>
				{pickerOpen && selectedId != null && (
					<motion.div
						initial={{ opacity: 0, scale: 0.96 }}
						animate={{ opacity: 1, scale: 1 }}
						exit={{ opacity: 0, scale: 0.96 }}
						transition={{ duration: 0.12 }}
						style={{
							marginTop: 8,
							padding: 10,
							border: "1px solid var(--border)",
							borderRadius: "var(--radius-sm)",
							background: "var(--surface)",
						}}
					>
						<div
							className="mono"
							style={{
								fontSize: 11,
								color: "var(--muted)",
								marginBottom: 6,
							}}
						>
							move forecast to:
						</div>
						<div
							style={{
								display: "flex",
								flexWrap: "wrap",
								gap: 4,
								marginBottom: 6,
							}}
						>
							{allowedMonths.map((m) => {
								const isCurrent = m.month === month;
								return (
									<button
										key={m.month}
										onClick={() => doMove(selectedId, m.month)}
										className="mono"
										style={{
											...pillStyle,
											color: isCurrent ? "var(--cyan)" : "var(--white)",
											borderColor: isCurrent ? "var(--cyan)" : "var(--border)",
										}}
									>
										{m.label}
									</button>
								);
							})}
						</div>
						<button
							onClick={() => setPickerOpen(false)}
							className="mono"
							style={pillStyle}
						>
							cancel
						</button>
					</motion.div>
				)}
			</AnimatePresence>
		</div>
	);
};


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

const addBtnStyle: React.CSSProperties = {
	...pillStyle,
	borderStyle: "dashed",
	color: "var(--muted)",
};
