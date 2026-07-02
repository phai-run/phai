import { queryDb } from "@livestore/livestore";
import { useStore } from "@livestore/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { events, STORE_ID, tables } from "../livestore/schema";
import {
	api,
	type BridgeIdentity,
	type ChartData,
	type EnvelopeUpsert,
	type NewForecast,
	type NewScenarioChange,
	type ForecastRecord,
	type ForecastTemplateRecord,
	type PlanChangeRecord,
	type PlanScenarioRecord,
	type ReviewFlushItem,
	type TxRow,
} from "./api";

const MAX_RETRIES = 10;
const pendingWrites$ = queryDb(tables.pendingWrites.orderBy("createdAt", "asc"));
const BRIDGE_IDENTITY_STORAGE_KEY = "phai.bridgeIdentity";
const UNKNOWN_VERSION = "unknown";

// One identity fetch per page load, shared by the bootstrap and every seed
// hook. A rejection clears the memo so the next caller retries the bridge.
let bridgeIdentityPromise: Promise<BridgeIdentity> | null = null;
const getBridgeIdentity = (): Promise<BridgeIdentity> => {
	if (!bridgeIdentityPromise) {
		bridgeIdentityPromise = api.identity().catch((e: unknown) => {
			bridgeIdentityPromise = null;
			throw e;
		});
	}
	return bridgeIdentityPromise;
};

/** Binary version of the bridge, or "unknown" while it is unreachable. */
export const getBridgeVersion = (): Promise<string> =>
	getBridgeIdentity().then(
		(identity) => identity.version ?? UNKNOWN_VERSION,
		() => UNKNOWN_VERSION,
	);

export interface SyncStatus {
	pending: number;
	error: string | null;
	seeded: boolean;
	retry: () => void;
}

interface PendingRow {
	writeId: string;
	type: string;
	transactionId: string;
	forecastId: string;
	payload: unknown;
	attempts: number;
}

interface ForecastStoreRow {
	forecastId: string;
	description: string;
	amount: string;
	dueDate: string | null;
	categoryId: string | null;
	accountId: string | null;
	status: string;
	kind: string;
	draggable: number;
	templateId: string | null;
	realizedTransactionId: string | null;
	realizedAt: string | null;
	metadataJson: Record<string, unknown>;
}

type StoreApi = ReturnType<typeof useStore>["store"];

const bool = (v: unknown): number => (v ? 1 : 0);

const readStoredBridgeIdentity = (): string | null => {
	try {
		return window.localStorage.getItem(BRIDGE_IDENTITY_STORAGE_KEY);
	} catch {
		return null;
	}
};

const writeStoredBridgeIdentity = (identity: BridgeIdentity) => {
	try {
		window.localStorage.setItem(BRIDGE_IDENTITY_STORAGE_KEY, identity.identity);
	} catch {
		// Storage can be unavailable in hardened/private contexts. In that case
		// we still gate flushing for this mount, but cannot persist the guard.
	}
};

export const shouldClearLocalWrites = (
	previousIdentity: string | null,
	nextIdentity: string,
	_queuedWriteCount: number,
): boolean => previousIdentity !== null && previousIdentity !== nextIdentity;

const clearStaleLocalWrites = (store: StoreApi, identity: BridgeIdentity) => {
	const previous = readStoredBridgeIdentity();
	const queuedWrites = store.query(pendingWrites$) as ReadonlyArray<PendingRow>;
	if (!shouldClearLocalWrites(previous, identity.identity, queuedWrites.length)) {
		return false;
	}
	store.commit(
		events.bridgeIdentityChanged({
			oldIdentity: previous ?? "unknown",
			newIdentity: identity.identity,
		}),
	);
	return true;
};

/**
 * Wires LiveStore to the Rust bridge:
 *  1. On mount, seed reference data (categories, accounts) from the bridge.
 *  2. Continuously drain `pendingWrites`, routing each row to its endpoint by
 *     `type` (review → /api/events, forecastMove → /api/forecast/move). On
 *     success, commit `writeAcked`; on failure, `writeFailed`. Retries on the
 *     next tick.
 *
 * The per-view re-seed of the transaction window, chart, forecasts and templates
 * is handled by the dedicated hooks below, which the views call so a seed only
 * fires when that view is mounted.
 */
export const useBridgeSync = (): SyncStatus => {
	const { store } = useStore();
	const [error, setError] = useState<string | null>(null);
	const [pending, setPending] = useState(0);
	const [seeded, setSeeded] = useState(false);
	const [identityReady, setIdentityReady] = useState(false);
	const flushing = useRef(false);
	const backoffRef = useRef({
		failures: 0,
		timer: null as ReturnType<typeof setTimeout> | null,
	});
	const scheduleFlushRef = useRef<(() => void) | null>(null);

	const clearBackoffTimer = () => {
		if (backoffRef.current.timer) {
			clearTimeout(backoffRef.current.timer);
			backoffRef.current.timer = null;
		}
	};

	// 1. Seed reference data once (re-runs when retry() bumps bootNonce after a
	// failed bootstrap).
	const [bootNonce, setBootNonce] = useState(0);
	useEffect(() => {
		let cancelled = false;
		setIdentityReady(false);
		// Fetch identity + reference data concurrently (independent requests) and
		// await them together, so first paint isn't gated on serial round-trips.
		// Identity still drives the stale-write clear, which must run before the
		// category/account commits (a bridge-identity change nukes all tables) —
		// that ordering is preserved inside the handler.
		Promise.all([getBridgeIdentity(), api.categories(), api.accounts()])
			.then(([identity, cats, accs]) => {
				if (cancelled) return;
				if (clearStaleLocalWrites(store, identity)) {
					setPending(0);
				}
				writeStoredBridgeIdentity(identity);
				sweepStaleSeedStamps(identity.version ?? UNKNOWN_VERSION);
				store.commit(events.categoriesSeeded({ ids: cats.ids }));
				store.commit(events.accountsSeeded({ rows: accs.rows }));
				setSeeded(true);
				setIdentityReady(true);
			})
			.catch((e: unknown) => {
				if (cancelled) return;
				console.error("[phai] bridge bootstrap failed:", e);
				setError(String(e));
			});
		return () => {
			cancelled = true;
		};
	}, [store, bootNonce]);

	// 2. Drain the typed pending-write queue with exponential backoff.
	useEffect(() => {
		if (!identityReady) return;
		const flush = async () => {
			if (flushing.current) return;
			const rows = store.query(pendingWrites$) as ReadonlyArray<PendingRow>;
			setPending(rows.length);
			if (rows.length === 0) return;
			flushing.current = true;
			try {
				const failures = await drainQueue(store, rows);
				if (failures.length > 0) {
					backoffRef.current.failures++;
				} else {
					backoffRef.current.failures = 0;
				}
				setError(failures.length > 0 ? failures[0] : null);
			} catch (e: unknown) {
				backoffRef.current.failures++;
				setError(String(e));
			} finally {
				flushing.current = false;
			}
			scheduleFlush();
		};

		const scheduleFlush = () => {
			clearBackoffTimer();
			const delay = Math.min(1000 * 2 ** backoffRef.current.failures, 30000);
			backoffRef.current.timer = setTimeout(() => {
				backoffRef.current.timer = null;
				void flush();
			}, delay);
		};
		scheduleFlushRef.current = scheduleFlush;

		const sub = store.subscribe(pendingWrites$, {
			onUpdate: () => void flush(),
		});
		void flush();
		scheduleFlush();
		return () => {
			sub();
			clearBackoffTimer();
		};
	}, [store, identityReady]);

	const retry = useCallback(() => {
		setError(null);
		if (!identityReady) {
			// Bootstrap never completed (bridge was down): re-run it — the chip's
			// retry button must never be a no-op.
			setBootNonce((n) => n + 1);
			return;
		}
		backoffRef.current.failures = 0;
		clearBackoffTimer();
		if (flushing.current) {
			scheduleFlushRef.current?.();
			return;
		}
		void (async () => {
			const rows = store.query(pendingWrites$) as ReadonlyArray<PendingRow>;
			if (rows.length > 0) {
				try {
					const failures = await drainQueue(store, rows);
					setError(failures.length > 0 ? failures[0] : null);
				} catch (e: unknown) {
					setError(String(e));
				}
			}
			scheduleFlushRef.current?.();
		})();
	}, [store, identityReady]);

	return { pending, error, seeded, retry };
};

/**
 * Routes each pending write to the right endpoint by `type`. Reviews flush as a
 * single batch (the bridge accepts `{ writes }`); forecast moves flush one at a
 * time. Returns the error strings of any failures (for the status chip).
 */
const drainQueue = async (
	store: StoreApi,
	rows: ReadonlyArray<PendingRow>,
): Promise<string[]> => {
	const errors: string[] = [];

	const reviews = rows.filter((r) => r.type === "review");
	if (reviews.length > 0) {
		const items: ReviewFlushItem[] = reviews.map((r) => ({
			writeId: r.writeId,
			transactionId: r.transactionId,
			patch: r.payload as ReviewFlushItem["patch"],
		}));
		try {
			const res = await api.flushReviews(items);
			if (res.acked.length > 0) {
				store.commit(
					...res.acked.map((writeId) => events.writeAcked({ writeId })),
				);
			}
			const failedEvents = res.failed.map((f) => {
				const row = reviews.find((r) => r.writeId === f.writeId);
				return retryOrAbandonEvent(row, f.writeId, f.error);
			});
			if (failedEvents.length > 0) {
				store.commit(...failedEvents);
			}
			for (const f of res.failed) errors.push(f.error);
		} catch (e: unknown) {
			// Whole batch failed (network) — mark each so the chip surfaces it.
			const msg = String(e);
			store.commit(
				...reviews.map((r) => retryOrAbandonEvent(r, r.writeId, msg)),
			);
			errors.push(msg);
		}
	}

	for (const r of rows) {
		const row = currentPendingRow(store, r.writeId);
		if (!row) continue;
		if (row.type === "forecastMove") {
			await flushOne(store, row, errors, async () =>
				api.moveForecast(
					await resolveForecastId(store, row.forecastId),
					(row.payload as { dueDate: string }).dueDate,
				),
			);
		} else if (row.type === "forecastCreate") {
			await flushForecastCreate(store, row, errors);
		} else if (row.type === "forecastEnvelope") {
			// The materializer queued the snake_case /api/forecast body verbatim.
			await flushOne(store, row, errors, () =>
				api.upsertForecast(row.payload as EnvelopeUpsert),
			);
		} else if (row.type === "forecastDelete") {
			await flushOne(store, row, errors, async () =>
				api.deleteForecast(await resolveForecastId(store, row.forecastId)),
			);
		} else if (row.type === "forecastSettle") {
			await flushOne(store, row, errors, async () =>
				api.settleForecast(
					await resolveForecastId(store, row.forecastId),
					(row.payload as { transactionId: string }).transactionId,
				),
			);
		} else if (row.type.startsWith("scenario")) {
			const send = scenarioFlushCall(row);
			if (send) await flushOne(store, row, errors, send);
		}
	}

	return errors;
};

/**
 * Maps a scenario pending-write row (ADR-0037) to its bridge call. Returns
 * null for unknown scenario types (skipped; nothing to flush).
 */
const scenarioFlushCall = (row: PendingRow): (() => Promise<unknown>) | null => {
	const payload = row.payload as {
		scenarioId: string;
		name?: string;
		description?: string | null;
		changeId?: string;
	};
	switch (row.type) {
		case "scenarioCreate":
			return () =>
				api.createScenario({
					scenarioId: payload.scenarioId,
					name: payload.name ?? "",
					description: payload.description ?? null,
				});
		case "scenarioChange":
			return () => api.addScenarioChange(row.payload as NewScenarioChange);
		case "scenarioChangeDelete":
			return () =>
				api.deleteScenarioChange(payload.changeId ?? "", payload.scenarioId);
		case "scenarioArchive":
			return () => api.archiveScenario(payload.scenarioId);
		case "scenarioDelete":
			return () => api.deleteScenario(payload.scenarioId);
		case "scenarioPromote":
			return () => api.promoteScenario(payload.scenarioId);
		default:
			return null;
	}
};

/**
 * Flush a single forecast write: POST it, ack on success, mark failed (and
 * collect the error) otherwise. Rows past the retry budget fail terminally.
 */
const flushOne = async (
	store: StoreApi,
	row: PendingRow,
	errors: string[],
	send: () => Promise<unknown>,
): Promise<void> => {
	if (row.attempts >= MAX_RETRIES) {
		store.commit(
			events.writeAbandoned({
				writeId: row.writeId,
				type: row.type,
				transactionId: row.transactionId,
				forecastId: row.forecastId,
				error: "max retries exceeded",
			}),
		);
		errors.push("max retries exceeded");
		return;
	}
	try {
		await send();
		store.commit(events.writeAcked({ writeId: row.writeId }));
	} catch (e: unknown) {
		const msg = String(e);
		store.commit(retryOrAbandonEvent(row, row.writeId, msg));
		errors.push(msg);
	}
};

const flushForecastCreate = async (
	store: StoreApi,
	row: PendingRow,
	errors: string[],
): Promise<void> => {
	if (row.attempts >= MAX_RETRIES) {
		store.commit(
			events.writeAbandoned({
				writeId: row.writeId,
				type: row.type,
				transactionId: row.transactionId,
				forecastId: row.forecastId,
				error: "max retries exceeded",
			}),
		);
		errors.push("max retries exceeded");
		return;
	}
	try {
		const response = await api.createForecast(row.payload as NewForecast);
		const forecast = currentForecastRow(store, row.writeId);
		if (!forecast) {
			store.commit(events.writeAcked({ writeId: row.writeId }));
			return;
		}
		store.commit(
			events.forecastCreateAcked({
				writeId: row.writeId,
				localForecastId: row.writeId,
				serverForecastId: response.forecast_id,
				description: forecast.description,
				amount: forecast.amount,
				dueDate: forecast.dueDate,
				categoryId: forecast.categoryId,
				accountId: forecast.accountId,
				status: forecast.status,
				kind: forecast.kind,
				draggable: forecast.draggable,
				templateId: forecast.templateId,
				realizedTransactionId: forecast.realizedTransactionId,
				realizedAt: forecast.realizedAt,
				metadataJson: forecast.metadataJson,
			}),
		);
	} catch (e: unknown) {
		const msg = String(e);
		store.commit(retryOrAbandonEvent(row, row.writeId, msg));
		errors.push(msg);
	}
};

const retryOrAbandonEvent = (
	row: PendingRow | undefined,
	writeId: string,
	error: string,
) => {
	const attempts = (row?.attempts ?? 0) + 1;
	if (attempts >= MAX_RETRIES) {
		return events.writeAbandoned({
			writeId,
			type: row?.type ?? "",
			transactionId: row?.transactionId ?? "",
			forecastId: row?.forecastId ?? "",
			error,
		});
	}
	return events.writeFailed({ writeId, error, attempts });
};

const currentPendingRow = (
	store: StoreApi,
	writeId: string,
): PendingRow | undefined =>
	(store.query(pendingWrites$) as ReadonlyArray<PendingRow>).find(
		(row) => row.writeId === writeId,
	);

const currentForecastRow = (
	store: StoreApi,
	forecastId: string,
): ForecastStoreRow | undefined =>
	(store.query(queryDb(tables.forecasts)) as ReadonlyArray<ForecastStoreRow>).find(
		(row) => row.forecastId === forecastId,
	);

const resolveForecastId = async (
	store: StoreApi,
	forecastId: string,
): Promise<string> => {
	const local = currentForecastRow(store, forecastId);
	if (!local) return forecastId;
	// Once the create has acked, the create's pending row is gone and any
	// dependent delete/settle has been remapped to the real server id. In that
	// (common) case `forecastId` is already the backend id — use it directly
	// instead of a content fuzzy-match, which could otherwise pick a different
	// forecast that happens to share the same description/amount/date.
	const pending = store.query(pendingWrites$) as ReadonlyArray<PendingRow>;
	const stillCreating = pending.some(
		(p) => p.type === "forecastCreate" && p.writeId === forecastId,
	);
	if (!stillCreating) return forecastId;
	const { forecasts } = await api.forecasts({});
	const match = forecasts.find((candidate) => {
		const candidateMeta =
			candidate.metadata_json && typeof candidate.metadata_json === "object"
				? (candidate.metadata_json as Record<string, unknown>)
				: {};
		return (
			candidate.description === local.description &&
			(candidate.due_date ?? null) === local.dueDate &&
			(candidate.amount ?? "0") === local.amount &&
			(candidate.category_id ?? null) === local.categoryId &&
			(candidate.account_id ?? null) === local.accountId &&
			String(candidateMeta.ui_role ?? "") ===
				String(local.metadataJson.ui_role ?? "")
		);
	});
	return match?.forecast_id ?? forecastId;
};

export interface SeedState {
	loading: boolean;
	error: string | null;
	reload: () => void;
}

// Stale-while-revalidate freshness window. The seeded datasets (transactions,
// chart, forecasts) live in the OPFS-persisted LiveStore cache and are rendered
// instantly on load; re-fetching them from the bridge is slow (BigQuery). Within
// this window we skip the re-fetch entirely so reloads/remounts are instant.
// A manual reload() or an expired window forces a refresh. Stored in
// localStorage (NOT LiveStore) so it never triggers an OPFS schema migration.
// The bridge also serves these reads from an in-memory TTL cache that busts on
// any write, and the upstream BigQuery is only refreshed by cron a few times a
// day — so a wide window here is safe: stale-while-revalidate still revalidates
// in the background.
const SEED_FRESH_MS = 60 * 60 * 1000;

const SEED_STAMP_PREFIX = "phai:lastSync:";

/**
 * Stamps are keyed by store version AND bridge binary version: upgrading
 * either one changes the key, so every stamp reads as stale and the fresh
 * store/new binary always reseeds. (The v5.6.0 regression: un-versioned
 * stamps stayed "fresh" across an upgrade and blocked the reseed of an
 * empty store.)
 */
export const seedStampKey = (cacheKey: string, version: string): string =>
	`${SEED_STAMP_PREFIX}${STORE_ID}:${version}:${cacheKey}`;

const seedTs = (cacheKey: string, version: string): number => {
	try {
		return Number(localStorage.getItem(seedStampKey(cacheKey, version))) || 0;
	} catch {
		return 0;
	}
};
const markSeed = (cacheKey: string, version: string): void => {
	try {
		localStorage.setItem(seedStampKey(cacheKey, version), String(Date.now()));
	} catch {
		/* private mode / quota — fall back to always-fetch */
	}
};
// Removes the stamp for a cacheKey under every store/binary version, so a
// manual reload() stays synchronous (no need to resolve the version first).
const clearSeed = (cacheKey: string): void => {
	try {
		const suffix = `:${cacheKey}`;
		for (let i = localStorage.length - 1; i >= 0; i--) {
			const k = localStorage.key(i);
			if (k?.startsWith(SEED_STAMP_PREFIX) && k.endsWith(suffix)) {
				localStorage.removeItem(k);
			}
		}
	} catch {
		/* ignore */
	}
};
/** The localStorage subset the stamp sweep touches (injectable for tests). */
export type StampStorage = Pick<Storage, "key" | "removeItem"> & {
	readonly length: number;
};

/** Drops stamps from other store/binary versions (old formats included). */
export const sweepStaleSeedStamps = (
	version: string,
	storage: StampStorage = localStorage,
): void => {
	try {
		const currentPrefix = `${SEED_STAMP_PREFIX}${STORE_ID}:${version}:`;
		for (let i = storage.length - 1; i >= 0; i--) {
			const k = storage.key(i);
			if (k?.startsWith(SEED_STAMP_PREFIX) && !k.startsWith(currentPrefix)) {
				storage.removeItem(k);
			}
		}
	} catch {
		/* ignore */
	}
};

/**
 * Freshness alone is not enough to skip a seed: after a store-version bump
 * the OPFS store is brand new and EMPTY while the stamps may still look
 * fresh. Skip only when the data the stamp vouches for is actually there.
 */
export const shouldSkipSeed = (args: {
	now: number;
	stampedAt: number;
	maxAgeMs: number;
	isMissingData: boolean;
}): boolean =>
	args.stampedAt > 0 &&
	args.now - args.stampedAt < args.maxAgeMs &&
	!args.isMissingData;

/**
 * Generic "fetch from bridge → commit a seed event" hook. Re-runs whenever
 * `deps` change (e.g. window controls) and exposes a manual `reload`. When a
 * `cacheKey` is given, the fetch is skipped while the last sync for that key is
 * still fresh (stale-while-revalidate) AND the seeded table still has data —
 * `isMissingData` is the caller's "my table is empty" probe, which overrides
 * freshness after a store-version bump wipes the OPFS cache.
 */
const useSeed = (
	fetcher: (isCurrent: () => boolean) => Promise<void>,
	deps: ReadonlyArray<unknown>,
	cacheKey?: string,
	isMissingData?: () => boolean,
	maxAgeMs: number = SEED_FRESH_MS,
): SeedState => {
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [nonce, setNonce] = useState(0);
	const reload = useCallback(() => {
		if (cacheKey) clearSeed(cacheKey);
		setNonce((n) => n + 1);
	}, [cacheKey]);

	useEffect(() => {
		let cancelled = false;
		const isCurrent = () => !cancelled;
		const run = async () => {
			if (cacheKey) {
				const version = await getBridgeVersion();
				if (cancelled) return;
				// Fresh cache + data on screen → skip the slow re-fetch.
				const skip = shouldSkipSeed({
					now: Date.now(),
					stampedAt: seedTs(cacheKey, version),
					maxAgeMs,
					isMissingData: isMissingData ? isMissingData() : false,
				});
				if (skip) {
					setLoading(false);
					return;
				}
			}
			setLoading(true);
			setError(null);
			try {
				await fetcher(isCurrent);
				if (!cancelled) {
					setError(null);
					if (cacheKey) markSeed(cacheKey, await getBridgeVersion());
				}
			} catch (e: unknown) {
				if (!cancelled) {
					console.error(`[phai] seed failed (${cacheKey ?? "anon"}):`, e);
					setError(String(e));
				}
			} finally {
				if (!cancelled) setLoading(false);
			}
		};
		void run();
		return () => {
			cancelled = true;
		};
		// fetcher is recreated by the caller when deps change.
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [...deps, nonce]);

	return { loading, error, reload };
};

export const normalizeTransactions = (rows: TxRow[]) =>
	rows.map((r) => ({
		id: r.id,
		accountId: r.accountId ?? "",
		postedAt: r.postedAt ?? "",
		amount: r.amount ?? "0",
		rawDescription: r.rawDescription ?? "",
		description: r.description ?? null,
		merchantName: r.merchantName ?? null,
		purpose: r.purpose ?? null,
		categoryId: r.categoryId ?? null,
		month: r.month ?? "",
		paymentStatus: r.paymentStatus ?? "",
		installmentMarker: r.installmentMarker ?? null,
		reviewed: bool(r.reviewed),
		isInstallment: bool(r.isInstallment),
		isSubscription: bool(r.isSubscription),
		commitmentTier: r.commitmentTier ?? null,
	}));

const TRANSACTIONS_PAGE_SIZE = 1000;

/**
 * Seed the full transaction window from the bridge, paginating automatically.
 * The first page replaces everything; subsequent pages are appended so the UI
 * stays responsive while a large window loads.
 */
export const useTransactionsSeed = (
	monthsBack: number,
	monthsAhead: number,
): SeedState => {
	const { store } = useStore();
	const fetcher = useCallback(async (isCurrent: () => boolean) => {
		let offset = 0;
		let isFirstPage = true;
		// eslint-disable-next-line no-constant-condition
		while (true) {
			const page = await api.transactions({
				monthsBack,
				monthsAhead,
				includeReviewed: true,
				limit: TRANSACTIONS_PAGE_SIZE,
				offset,
			});
			if (!isCurrent()) return;
			const normalized = normalizeTransactions(page.rows);
			if (isFirstPage) {
				store.commit(events.transactionsSeeded({ rows: normalized }));
				isFirstPage = false;
			} else if (normalized.length > 0) {
				store.commit(events.transactionsPageSeeded({ rows: normalized }));
			}
			offset += page.rows.length;
			if (!page.hasMore) break;
		}
	}, [store, monthsBack, monthsAhead]);
	const isMissingData = useCallback(
		() => store.query(tables.transactions.count()) === 0,
		[store],
	);
	return useSeed(
		fetcher,
		[fetcher],
		`tx:${monthsBack}:${monthsAhead}`,
		isMissingData,
	);
};

const normalizeChart = (data: ChartData) =>
	data.months.map((m, i) => ({
		label: m.label,
		month: m.month ?? m.label,
		inflows: m.inflows ?? "0",
		outflows: m.outflows ?? "0",
		forecastInflowsRemaining: m.forecast_inflows_remaining ?? "0",
		forecastOutflowsRemaining: m.forecast_outflows_remaining ?? "0",
		closingBalance: m.closing_balance ?? m.projected_closing_balance ?? "0",
		projectedClosingBalance:
			m.projected_closing_balance ?? m.closing_balance ?? "0",
		isFuture: m.is_future ? 1 : 0,
		ordinal: i,
	}));

/** Re-seed the cash-evolution chart from the bridge. */
export const useChartSeed = (
	monthsBack: number,
	monthsAhead: number,
): SeedState => {
	const { store } = useStore();
	const fetcher = useCallback(async (isCurrent: () => boolean) => {
		const data = await api.chart(monthsBack, monthsAhead);
		if (!isCurrent()) return;
		store.commit(events.chartSeeded({ months: normalizeChart(data) }));
	}, [store, monthsBack, monthsAhead]);
	const isMissingData = useCallback(
		() => store.query(tables.chartMonths.count()) === 0,
		[store],
	);
	return useSeed(
		fetcher,
		[fetcher],
		`chart:${monthsBack}:${monthsAhead}`,
		isMissingData,
	);
};

const normalizeForecasts = (forecasts: ForecastRecord[]) =>
	forecasts.map((f) => ({
		forecastId: f.forecast_id,
		dueDate: f.due_date ?? null,
		description: f.description ?? "",
		amount: f.amount ?? "0",
		categoryId: f.category_id ?? null,
		accountId: f.account_id ?? null,
		status: f.status ?? "",
		kind: f.kind ?? "manual",
		draggable: bool(f.draggable),
		templateId: f.template_id ?? null,
		realizedTransactionId: f.realized_transaction_id ?? null,
		realizedAt: f.realized_at ?? null,
		metadataJson:
			f.metadata_json && typeof f.metadata_json === "object"
				? (f.metadata_json as Record<string, unknown>)
				: {},
	}));

const normalizeTemplates = (templates: ForecastTemplateRecord[]) =>
	templates.map((t) => ({
		templateId: t.template_id,
		description: t.description ?? "",
		kind: t.kind ?? null,
		cadence: t.cadence ?? null,
		amount: t.amount ?? "0",
		status: t.status ?? "",
		confidence: t.confidence == null ? null : String(t.confidence),
	}));

export const normalizeScenarios = (rows: PlanScenarioRecord[]) =>
	rows.map((s) => ({
		scenarioId: s.scenario_id,
		name: s.name ?? "",
		description: s.description ?? null,
		status: s.status ?? "ativo",
	}));

export const normalizeScenarioChanges = (
	changes: PlanChangeRecord[],
	orphaned: string[],
) =>
	changes.map((c) => ({
		changeId: c.change_id,
		scenarioId: c.scenario_id,
		kind: c.kind ?? "",
		targetForecastId: c.target_forecast_id ?? null,
		targetTemplateId: c.target_template_id ?? null,
		month: c.month ?? null,
		effectiveFrom: c.effective_from ?? null,
		amount: c.amount ?? null,
		monthsCount: c.months_count ?? null,
		description: c.description ?? null,
		categoryId: c.category_id ?? null,
		accountId: c.account_id ?? null,
		status: c.status ?? "ativo",
		orphaned: orphaned.includes(c.change_id) ? 1 : 0,
	}));

/**
 * Re-seed the scenario list. No freshness stamp: the list is tiny and must
 * reflect this session's own writes as soon as the flusher lands them.
 */
export const useScenariosSeed = (): SeedState => {
	const { store } = useStore();
	const fetcher = useCallback(
		async (isCurrent: () => boolean) => {
			const { scenarios } = await api.scenarios();
			if (!isCurrent()) return;
			store.commit(
				events.scenariosSeeded({ rows: normalizeScenarios(scenarios) }),
			);
		},
		[store],
	);
	return useSeed(fetcher, [fetcher]);
};

/**
 * Re-seed the active scenario's changes (with server-recomputed orphan flags)
 * and its chart projection. Pass `null` to clear nothing — the hook is a
 * no-op until a scenario is active. `nonce` forces a refetch after local
 * writes flush.
 */
export const useScenarioDetailSeed = (
	scenarioId: string | null,
	monthsBack: number,
	monthsAhead: number,
	nonce = 0,
): SeedState => {
	const { store } = useStore();
	const fetcher = useCallback(
		async (isCurrent: () => boolean) => {
			if (!scenarioId) return;
			const [{ changes, orphaned }, projection] = await Promise.all([
				api.scenarioChanges(scenarioId),
				api.scenarioProjection(scenarioId, monthsBack, monthsAhead),
			]);
			if (!isCurrent()) return;
			store.commit(
				events.scenarioChangesSeeded({
					scenarioId,
					rows: normalizeScenarioChanges(changes, orphaned),
				}),
			);
			store.commit(
				events.scenarioChartSeeded({
					scenarioId,
					months: normalizeChart(projection).map((m) => ({
						...m,
						scenarioId,
					})),
				}),
			);
		},
		[store, scenarioId, monthsBack, monthsAhead],
	);
	return useSeed(fetcher, [fetcher, nonce]);
};

/** Re-seed forecasts + templates from the bridge; reload after mutations. */
export const useForecastsSeed = (status: string | null): SeedState => {
	const { store } = useStore();
	const fetcher = useCallback(async (isCurrent: () => boolean) => {
		const [{ forecasts }, { templates }] = await Promise.all([
			api.forecasts({ status }),
			api.forecastTemplates({}),
		]);
		if (!isCurrent()) return;
		store.commit(
			events.forecastsSeeded({ rows: normalizeForecasts(forecasts) }),
		);
		store.commit(
			events.forecastTemplatesSeeded({ rows: normalizeTemplates(templates) }),
		);
	}, [store, status]);
	const isMissingData = useCallback(
		// One fetcher seeds both tables; either one empty means data is missing.
		// (A user with legitimately zero forecasts refetches every mount — the
		// correctness of "never show an empty store as fresh" wins over that
		// micro-optimization.)
		() =>
			store.query(tables.forecasts.count()) === 0 ||
			store.query(tables.forecastTemplates.count()) === 0,
		[store],
	);
	return useSeed(
		fetcher,
		[fetcher],
		`forecasts:${status ?? "all"}`,
		isMissingData,
	);
};
