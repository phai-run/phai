import { useCallback, useEffect, useRef, useState } from "react";
import { api, type VersionStatus } from "../bridge/api";

const POLL_INTERVAL = 5 * 60 * 1000; // 5 min
const RESTART_POLL_INTERVAL = 1_500; // 1.5s while waiting for restart
const RESTART_TIMEOUT = 30_000; // give up after 30s

export type UpdateState = "idle" | "updating" | "restarting" | "error";

export interface UpdateCheck {
	updateAvailable: boolean;
	currentVersion: string | null;
	latestVersion: string | null;
	state: UpdateState;
	error: string | null;
	applyUpdate: () => void;
}

export const useUpdateCheck = (): UpdateCheck => {
	const [status, setStatus] = useState<VersionStatus | null>(null);
	const [state, setState] = useState<UpdateState>("idle");
	const [error, setError] = useState<string | null>(null);
	const timerRef = useRef<ReturnType<typeof setInterval>>(undefined);

	const poll = useCallback(() => {
		api.version().then(setStatus, () => {});
	}, []);

	// Periodic poll + window focus
	useEffect(() => {
		poll();
		timerRef.current = setInterval(poll, POLL_INTERVAL);
		const onFocus = () => poll();
		window.addEventListener("focus", onFocus);
		return () => {
			clearInterval(timerRef.current);
			window.removeEventListener("focus", onFocus);
		};
	}, [poll]);

	const applyUpdate = useCallback(() => {
		setState("updating");
		setError(null);
		api
			.triggerUpdate()
			.then((result) => {
				if (result.status === "up_to_date") {
					setState("idle");
					return;
				}
				setState("restarting");
				const start = Date.now();
				const waitForRestart = setInterval(() => {
					if (Date.now() - start > RESTART_TIMEOUT) {
						clearInterval(waitForRestart);
						setState("error");
						setError(
							"Servidor não reiniciou a tempo. Reinicie manualmente.",
						);
						return;
					}
					api
						.version()
						.then((v) => {
							if (v.currentVersion !== status?.currentVersion) {
								clearInterval(waitForRestart);
								window.location.reload();
							}
						})
						.catch(() => {});
				}, RESTART_POLL_INTERVAL);
			})
			.catch((e: Error) => {
				setState("error");
				setError(e.message);
			});
	}, [status?.currentVersion]);

	return {
		updateAvailable: status?.updateAvailable ?? false,
		currentVersion: status?.currentVersion ?? null,
		latestVersion: status?.latestVersion ?? null,
		state,
		error,
		applyUpdate,
	};
};
