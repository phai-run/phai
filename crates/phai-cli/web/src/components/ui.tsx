import { useEffect, type CSSProperties, type ReactNode } from "react";
import { formatMoneyNumber } from "../lib/format";
import { useCountUp } from "../design/motion";

/**
 * Shared, brand-aligned primitives (DESIGN.md): cards with 1px borders and no
 * shadows, fully-rounded pills, mono inputs, section headers. One accent per
 * view — components take their accent from CSS vars chosen by the caller.
 */

export const ViewHeader = ({
	title,
	count,
	accent = "var(--purple)",
}: {
	title: string;
	count?: number;
	accent?: string;
}) => (
	<h2
		style={{
			fontFamily: "var(--font-display)",
			fontSize: "1.6rem",
			letterSpacing: "-0.02em",
			margin: "0 0 20px",
			display: "flex",
			alignItems: "baseline",
			gap: 10,
		}}
	>
		{title}
		{count != null && (
			<span
				className="mono"
				style={{ color: accent, fontSize: "0.55em", fontWeight: 400 }}
			>
				{count}
			</span>
		)}
	</h2>
);

export const Card = ({
	children,
	style,
	selected,
	accent = "var(--purple)",
}: {
	children: ReactNode;
	style?: CSSProperties;
	selected?: boolean;
	accent?: string;
}) => (
	<div
		style={{
			background: "var(--surface)",
			border: `1px solid ${selected ? accent : "var(--border)"}`,
			borderRadius: "var(--radius-lg)",
			padding: "var(--card-pad)",
			transition: "border-color 150ms",
			...style,
		}}
	>
		{children}
	</div>
);

export const Pill = ({
	active,
	onClick,
	children,
	accent = "var(--purple)",
}: {
	active?: boolean;
	onClick?: () => void;
	children: ReactNode;
	accent?: string;
}) => (
	<button
		onClick={onClick}
		className="mono"
		style={{
			background: active ? "rgba(0,0,0,0.04)" : "transparent",
			color: active ? accent : "var(--muted)",
			border: `1px solid ${active ? accent + "55" : "var(--border)"}`,
			borderRadius: "var(--radius-full)",
			padding: "6px 16px",
			cursor: "pointer",
			fontSize: 12,
			transition: "border-color 150ms, color 150ms",
		}}
	>
		{children}
	</button>
);

const fieldStyle: CSSProperties = {
	background: "var(--bg)",
	color: "var(--white)",
	border: "1px solid var(--border)",
	borderRadius: "var(--radius-sm)",
	padding: "6px 10px",
	fontSize: 12,
	fontFamily: "var(--font-mono)",
};

export const TextInput = (
	props: React.InputHTMLAttributes<HTMLInputElement> & { accent?: string },
) => {
	const { accent, style, ...rest } = props;
	return (
		<input
			{...rest}
			className="mono"
			style={{ ...fieldStyle, ...(accent ? { color: accent } : {}), ...style }}
		/>
	);
};

export const Select = (
	props: React.SelectHTMLAttributes<HTMLSelectElement>,
) => {
	const { style, ...rest } = props;
	return (
		<select {...rest} className="mono" style={{ ...fieldStyle, ...style }} />
	);
};

export const Label = ({ children }: { children: ReactNode }) => (
	<span
		style={{
			fontFamily: "var(--font-body)",
			fontWeight: 600,
			fontSize: 11,
			letterSpacing: "0.12em",
			textTransform: "uppercase",
			color: "var(--muted)",
		}}
	>
		{children}
	</span>
);

export const FilterBar = ({ children }: { children: ReactNode }) => (
	<div
		style={{
			display: "flex",
			flexWrap: "wrap",
			gap: 10,
			alignItems: "center",
			marginBottom: 20,
		}}
	>
		{children}
	</div>
);

export const EmptyState = ({ message }: { message: string }) => (
	<div
		style={{ textAlign: "center", padding: "80px 0", color: "var(--muted)" }}
	>
		<div className="phi" style={{ fontSize: "3rem" }}>
			φ
		</div>
		<p className="mono" style={{ marginTop: 8 }}>
			{message}
		</p>
	</div>
);

export const LoadingNote = ({
	message = "loading…",
}: {
	message?: string;
}) => (
	<p className="mono" style={{ color: "var(--muted)", fontSize: 12 }}>
		{message}
	</p>
);

export const ErrorNote = ({ error }: { error: string }) => (
	<p className="mono" style={{ color: "var(--rose)", fontSize: 12 }}>
		{error}
	</p>
);

export const Skeleton = ({
	height = 80,
	width,
}: {
	height?: number;
	width?: number | string;
}) => (
	<div
		className="skeleton"
		style={{ height, ...(width != null ? { width } : {}), minWidth: 0 }}
	/>
);

/** Money figure that "rolls" from its previous value when it changes (N8).
 *  Honours prefers-reduced-motion (jumps straight to the value). */
export const CountMoney = ({
	value,
	style,
}: {
	value: number;
	style?: CSSProperties;
}) => {
	const v = useCountUp(value);
	return (
		<span className="mono" style={style}>
			{formatMoneyNumber(v)}
		</span>
	);
};

/**
 * Branded loading skeletons live in `./Skeleton`. Re-exported here so existing
 * `import … from "../components/ui"` call sites keep working.
 */
export {
	ChartSkeleton,
	HeroSkeleton,
	ListSkeleton,
	CardGridSkeleton,
	DashboardSkeleton,
} from "./Skeleton";

export const Toast = ({
	message,
	type = "info",
	onDismiss,
}: {
	message: string;
	type?: "success" | "error" | "info";
	onDismiss: () => void;
}) => {
	useEffect(() => {
		const timer = setTimeout(onDismiss, 3000);
		return () => clearTimeout(timer);
	}, [onDismiss]);

	return (
		<div
			className="mono"
			style={{
				position: "fixed",
				bottom: 24,
				right: 24,
				zIndex: 100,
				background: "var(--surface)",
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-md)",
				padding: "12px 16px",
				fontSize: 13,
				display: "flex",
				alignItems: "center",
				gap: 10,
				boxShadow: "var(--drag-shadow)",
				maxWidth: 360,
				color:
					type === "error"
						? "var(--rose)"
						: type === "success"
							? "var(--green)"
							: "var(--white)",
			}}
		>
			<span>{type === "success" ? "✓" : type === "error" ? "✗" : "ℹ"}</span>
			<span style={{ flex: 1 }}>{message}</span>
			<button
				onClick={onDismiss}
				style={{
					background: "none",
					border: "none",
					color: "var(--muted)",
					cursor: "pointer",
					fontSize: 14,
					padding: 0,
					lineHeight: 1,
				}}
			>
				×
			</button>
		</div>
	);
};
