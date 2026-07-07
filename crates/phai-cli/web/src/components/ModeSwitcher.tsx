import { useClientDocument } from "@livestore/react";
import { motion } from "framer-motion";
import { tables } from "../livestore/schema";
import { springSnap } from "../design/motion";

/**
 * Global view-mode switcher (planilha · categorias · cartões). Lives in the top
 * header so it reads as primary navigation — deliberately separate from the
 * sheet's filter controls. Purple = active navigation context (the app's colour
 * hierarchy: purple for navigation/active state, neutral for refinements).
 */
export const MODES = [
	{ id: "planilha", label: "planilha" },
	{ id: "categorias", label: "categorias" },
	{ id: "cartoes", label: "cartões" },
] as const;

export const ModeSwitcher = () => {
	const [ui, setUi] = useClientDocument(tables.ui);
	const current = ui.detailMode || "planilha";
	return (
		<div
			role="tablist"
			aria-label="modo de visualização"
			style={{
				display: "inline-flex",
				gap: 2,
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-full)",
				padding: 3,
				background: "var(--card)",
			}}
		>
			{MODES.map((m, i) => {
				const active = current === m.id;
				return (
					<button
						key={m.id}
						type="button"
						role="tab"
						aria-selected={active}
						title={`${m.label} (${i + 1})`}
						onClick={() => setUi({ detailMode: m.id })}
						className="mono pressable"
						style={{
							border: "none",
							borderRadius: "var(--radius-full)",
							padding: "5px 14px",
							fontSize: 12,
							cursor: "pointer",
							background: "transparent",
							color: active ? "#fff" : "var(--muted)",
							position: "relative",
							zIndex: 1,
							transition: "color 150ms",
						}}
					>
						{active && (
							<motion.span
								layoutId="mode-switch-indicator"
								transition={springSnap}
								style={{
									position: "absolute",
									inset: 0,
									borderRadius: "var(--radius-full)",
									background: "var(--purple)",
									zIndex: -1,
								}}
							/>
						)}
						{m.label}
					</button>
				);
			})}
		</div>
	);
};
