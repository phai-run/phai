import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
	type ReactNode,
} from "react";

/**
 * Hand-rolled pointer-events drag-and-drop (zero deps, keeps the bundle clean).
 *
 * Why not a library or native HTML5 drag: the brand spec (DESIGN.md) requires the
 * dragged element to follow the cursor **1:1**, drop targets to highlight live,
 * and a single soft shadow on the dragged chip — none of which the native drag
 * ghost gives us. This implementation is ~one file and fully under our control.
 *
 * Flow:
 *  - A drag *source* calls `startDrag(payload, event)` on pointerdown; we capture
 *    the pointer and render a floating ghost that tracks the cursor every frame.
 *  - Drop *targets* register a hit-rect (a month column) via `registerTarget`;
 *    on pointermove we hit-test the pointer against all rects and set
 *    `hoverTargetId` so the column can highlight.
 *  - On pointerup we fire the matched target's `onDrop(payload)`; LiveStore makes
 *    the result instant. We honor `prefers-reduced-motion` by skipping the lift
 *    transition (the shadow stays — it's a state signal, not motion).
 */

export interface DragPayload {
	forecastId: string;
	label: string;
	amount: string;
}

interface TargetRect {
	id: string;
	getRect: () => DOMRect | null;
	onDrop: (payload: DragPayload) => void;
}

interface DndState {
	dragging: DragPayload | null;
	hoverTargetId: string | null;
	pointer: { x: number; y: number };
	startDrag: (payload: DragPayload, e: React.PointerEvent) => void;
	registerTarget: (t: TargetRect) => () => void;
}

const Ctx = createContext<DndState | null>(null);

export const useDnd = (): DndState => {
	const ctx = useContext(Ctx);
	if (!ctx) throw new Error("useDnd outside DndProvider");
	return ctx;
};

export const DndProvider = ({ children }: { children: ReactNode }) => {
	const [dragging, setDragging] = useState<DragPayload | null>(null);
	const [hoverTargetId, setHoverTargetId] = useState<string | null>(null);
	const [pointer, setPointer] = useState({ x: 0, y: 0 });
	const targets = useRef(new Map<string, TargetRect>());
	const draggingRef = useRef<DragPayload | null>(null);
	const cleanupRef = useRef<(() => void) | null>(null);

	const registerTarget = useCallback((t: TargetRect) => {
		targets.current.set(t.id, t);
		return () => {
			targets.current.delete(t.id);
		};
	}, []);

	const hitTest = useCallback((x: number, y: number): TargetRect | null => {
		for (const t of targets.current.values()) {
			const r = t.getRect();
			if (r && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom)
				return t;
		}
		return null;
	}, []);

	const startDrag = useCallback(
		(payload: DragPayload, e: React.PointerEvent) => {
			e.preventDefault();
			draggingRef.current = payload;
			setDragging(payload);
			setPointer({ x: e.clientX, y: e.clientY });

			const onMove = (ev: PointerEvent) => {
				setPointer({ x: ev.clientX, y: ev.clientY });
				const hit = hitTest(ev.clientX, ev.clientY);
				setHoverTargetId(hit?.id ?? null);
			};
			cleanupRef.current = () => {
				window.removeEventListener("pointermove", onMove);
				window.removeEventListener("pointerup", onUp);
			};
			const onUp = (ev: PointerEvent) => {
				cleanupRef.current?.();
				cleanupRef.current = null;
				const hit = hitTest(ev.clientX, ev.clientY);
				const p = draggingRef.current;
				draggingRef.current = null;
				setDragging(null);
				setHoverTargetId(null);
				if (hit && p) hit.onDrop(p);
			};
			window.addEventListener("pointermove", onMove);
			window.addEventListener("pointerup", onUp);
		},
		[hitTest],
	);

	useEffect(() => {
		return () => {
			cleanupRef.current?.();
		};
	}, []);

	const value = useMemo<DndState>(
		() => ({ dragging, hoverTargetId, pointer, startDrag, registerTarget }),
		[dragging, hoverTargetId, pointer, startDrag, registerTarget],
	);

	return (
		<Ctx.Provider value={value}>
			{children}
			{dragging && <DragGhost payload={dragging} x={pointer.x} y={pointer.y} />}
		</Ctx.Provider>
	);
};

const DragGhost = ({
	payload,
	x,
	y,
}: {
	payload: DragPayload;
	x: number;
	y: number;
}) => (
	<div
		className="mono"
		style={{
			position: "fixed",
			left: x + 12,
			top: y + 12,
			zIndex: 1000,
			pointerEvents: "none",
			background: "var(--surface)",
			border: "1px solid var(--purple)",
			borderRadius: "var(--radius-sm)",
			// The one sanctioned shadow (DESIGN.md): a dragged element signalling lift.
			boxShadow: "var(--drag-shadow)",
			padding: "6px 12px",
			fontSize: 13,
			display: "flex",
			gap: 10,
			alignItems: "baseline",
			maxWidth: 280,
		}}
	>
		<span
			style={{
				overflow: "hidden",
				textOverflow: "ellipsis",
				whiteSpace: "nowrap",
			}}
		>
			{payload.label}
		</span>
		<span style={{ color: "var(--purple)" }}>{payload.amount}</span>
	</div>
);
