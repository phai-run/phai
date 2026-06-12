import type { ChartMonthView, ChartMode } from "../types";

// ── Legend ─────────────────────────────────────────────────────────────────

export const ChartLegend = ({
	mode,
	months,
}: {
	mode: ChartMode;
	months: ReadonlyArray<ChartMonthView>;
}) => {
	const hasFc = months.some((m) => m.isFuture === 1);

	const items: Array<{
		color: string;
		label: string;
		dashed?: boolean;
	}> = [];

	if (mode === "caixa") {
		items.push({ color: "var(--cyan)", label: "income" });
		items.push({ color: "var(--rose)", label: "expenses" });
		if (hasFc) {
			items.push({ color: "#99f6e4", label: "forecast income" });
			items.push({ color: "#fda4af", label: "forecast expenses" });
			items.push({ color: "var(--amber)", label: "forecast installment" });
		}
		items.push({
			color: "var(--purple)",
			label: "balance",
			dashed: true,
		});
	} else if (mode === "despesas-barras") {
		items.push({ color: "var(--rose)", label: "actual" });
		if (hasFc)
			items.push({
				color: "#fda4af",
				label: "forecast",
			});
	}

	return (
		<div
			className="mono"
			style={{
				display: "flex",
				flexWrap: "wrap",
				gap: 14,
				fontSize: 10,
				color: "var(--muted)",
				marginTop: 6,
			}}
		>
			{items.map((it) => (
				<LegendSwatch
					key={it.label}
					color={it.color}
					label={it.label}
					dashed={it.dashed}
				/>
			))}
		</div>
	);
};


// ── Legend swatch ──────────────────────────────────────────────────────────

const LegendSwatch = ({
	color,
	label,
	dashed,
}: {
	color: string;
	label: string;
	dashed?: boolean;
}) => (
	<span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
		<span
			style={{
				width: dashed ? 14 : 10,
				height: dashed ? 0 : 10,
				borderRadius: dashed ? 0 : 2,
				background: color,
				border: dashed ? `1.5px dashed ${color}` : "none",
			}}
		/>
		{label}
	</span>
);
