import type { CSSProperties, ReactNode } from "react";

/**
 * Branded loading skeletons (N7).
 *
 * Cold start seeds the whole window from the bridge (BigQuery → ~10s). Until the
 * first rows land the store is empty; rendering R$ 0,00 everywhere reads as
 * "broken". These placeholders mirror the real layout — hero balance, four stat
 * cards, the cash chart, the transaction sheet — so the wait feels intentional
 * and the page doesn't reflow when data arrives.
 *
 * Visual language matches the app: 1px-border cards on the light base, the
 * `.skeleton` shimmer token (tokens.css), and the ceremonial φ as the brand
 * anchor. Every block honours `prefers-reduced-motion` via the shared token.
 *
 * Accessibility: each region is wrapped in an `aria-busy` container with a
 * single visually-hidden status label, so a screen reader announces "loading …"
 * once instead of reading dozens of empty bars.
 */

/** A single shimmering placeholder bar. */
export const SkeletonBar = ({
	height = 16,
	width,
	radius = "var(--radius-sm)",
	style,
}: {
	height?: number | string;
	width?: number | string;
	radius?: number | string;
	style?: CSSProperties;
}) => (
	<div
		className="skeleton"
		style={{
			height,
			width: width ?? "100%",
			borderRadius: radius,
			minWidth: 0,
			...style,
		}}
	/>
);

/**
 * Region wrapper: marks the subtree `aria-busy` and emits one off-screen status
 * line so assistive tech announces the load without reading every bar.
 */
export const LoadingRegion = ({
	label,
	children,
	style,
}: {
	label: string;
	children: ReactNode;
	style?: CSSProperties;
}) => (
	<div role="status" aria-busy="true" aria-live="polite" style={style}>
		<span
			style={{
				position: "absolute",
				width: 1,
				height: 1,
				padding: 0,
				margin: -1,
				overflow: "hidden",
				clip: "rect(0,0,0,0)",
				whiteSpace: "nowrap",
				border: 0,
			}}
		>
			{label}
		</span>
		{children}
	</div>
);

const statCardStyle: CSSProperties = {
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-md)",
	padding: "10px 12px",
	minWidth: 0,
};

/** Hero placeholder: the dominant balance figure + four supporting stat cards. */
export const HeroSkeleton = () => (
	<LoadingRegion label="Loading cash balance…">
		<div style={{ display: "flex", alignItems: "baseline", gap: 10, marginBottom: 8 }}>
			<SkeletonBar height={11} width={96} />
			<SkeletonBar height={11} width={48} />
		</div>
		{/* Dominant headline figure */}
		<SkeletonBar height={44} width="min(420px, 70%)" radius="var(--radius-md)" />
		{/* Four supporting KPIs — income / expenses / net / projected */}
		<div
			style={{
				display: "grid",
				gridTemplateColumns: "repeat(auto-fit, minmax(120px, 1fr))",
				gap: 10,
				marginTop: 16,
				maxWidth: 640,
			}}
		>
			{["income", "expenses", "net", "projected"].map((k) => (
				<div key={k} style={statCardStyle}>
					<SkeletonBar height={9} width={52} style={{ marginBottom: 8 }} />
					<SkeletonBar height={18} width="70%" />
				</div>
			))}
		</div>
	</LoadingRegion>
);

/**
 * Cash-chart placeholder: a row of bars at varied heights (deterministic, so it
 * doesn't jitter on re-render) with a faint baseline and the φ mark watermarked
 * in the centre — the brand cue that this is phai loading, not a dead chart.
 */
export const ChartSkeleton = () => (
	<LoadingRegion label="Loading cash chart…">
		<div style={{ position: "relative" }}>
			<div
				style={{
					display: "flex",
					alignItems: "flex-end",
					gap: 8,
					height: 150,
					padding: "12px 0 28px",
				}}
			>
				{Array.from({ length: 12 }).map((_, i) => (
					<SkeletonBar
						key={i}
						width="auto"
						height={`${35 + ((i * 41) % 55)}%`}
						radius={4}
						style={{ flex: 1 }}
					/>
				))}
			</div>
			<div
				className="phi"
				aria-hidden
				style={{
					position: "absolute",
					inset: 0,
					display: "flex",
					alignItems: "center",
					justifyContent: "center",
					fontSize: "2.5rem",
					opacity: 0.18,
					pointerEvents: "none",
				}}
			>
				φ
			</div>
		</div>
	</LoadingRegion>
);

/** Transaction / month list placeholder: a header strip + N shimmer rows. */
export const ListSkeleton = ({ rows = 6 }: { rows?: number }) => (
	<LoadingRegion label="Loading transactions…">
		<div
			style={{
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-md)",
				overflow: "hidden",
				background: "var(--card)",
			}}
		>
			{Array.from({ length: rows }).map((_, i) => (
				<div
					key={i}
					style={{
						display: "flex",
						alignItems: "center",
						gap: 16,
						padding: "12px 14px",
						borderBottom:
							i === rows - 1 ? "none" : "1px solid var(--border)",
					}}
				>
					<SkeletonBar height={12} width={48} />
					<SkeletonBar height={12} width={`${40 + ((i * 23) % 40)}%`} />
					<SkeletonBar
						height={20}
						width={96}
						radius="var(--radius-full)"
						style={{ marginLeft: "auto" }}
					/>
					<SkeletonBar height={12} width={72} />
				</div>
			))}
		</div>
	</LoadingRegion>
);

/** Card-grid placeholder: a few tile-shaped shimmer blocks. */
export const CardGridSkeleton = ({ tiles = 2 }: { tiles?: number }) => (
	<LoadingRegion label="Loading cards…">
		<div
			style={{
				display: "grid",
				gridTemplateColumns: "repeat(auto-fill, minmax(340px, 460px))",
				justifyContent: "start",
				gap: 16,
			}}
		>
			{Array.from({ length: tiles }).map((_, i) => (
				<div
					key={i}
					style={{
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-lg)",
						padding: "var(--card-pad)",
					}}
				>
					<div
						style={{
							display: "flex",
							justifyContent: "space-between",
							gap: 8,
						}}
					>
						<SkeletonBar height={13} width={120} />
						<SkeletonBar height={11} width={48} />
					</div>
					<SkeletonBar height={26} width="60%" style={{ marginTop: 10 }} />
					<div
						style={{
							display: "grid",
							gridTemplateColumns: "repeat(3, 1fr)",
							gap: 8,
							marginTop: 14,
						}}
					>
						{[0, 1, 2].map((j) => (
							<SkeletonBar key={j} height={34} radius="var(--radius-sm)" />
						))}
					</div>
				</div>
			))}
		</div>
	</LoadingRegion>
);

/**
 * Full cold-start placeholder for the Dashboard: hero + chart + sheet, stacked
 * exactly where the real components land so first paint of real data slots in
 * without a layout jump. A subtle `fade-in-soft` entrance softens the swap.
 */
export const DashboardSkeleton = () => (
	<div className="fade-in-soft" style={{ display: "grid", gap: 20 }}>
		<HeroSkeleton />
		<ChartSkeleton />
		<ListSkeleton />
	</div>
);
