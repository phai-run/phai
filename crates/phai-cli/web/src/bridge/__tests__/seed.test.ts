// @ts-nocheck — store type inference is overly strict with synced events;
//               tests pass at runtime. See vitest output.
/**
 * Unit tests for incremental transaction seeding.
 *
 * Verifies that:
 *  - First page (transactionsSeeded) deletes all + inserts.
 *  - Subsequent pages (transactionsPageSeeded) append without deleting.
 *  - After seeding, all rows from all pages are present.
 */
import { makeInMemoryAdapter } from "@livestore/adapter-web";
import { createStorePromise } from "@livestore/livestore";
import { describe, it, expect, beforeAll, afterAll, beforeEach } from "vitest";
import { schema, events, tables } from "../../livestore/schema";
import { normalizeTransactions } from "../sync";

const bool = (v: unknown): number => (v ? 1 : 0);

const makeTx = (
	id: string,
	overrides: Partial<Record<string, unknown>> = {},
) => ({
	id,
	accountId: (overrides.accountId as string) ?? "acc-1",
	postedAt: (overrides.postedAt as string) ?? "2024-01-15",
	amount: (overrides.amount as string) ?? "-50.00",
	rawDescription: (overrides.rawDescription as string) ?? `desc-${id}`,
	description: (overrides.description as string | null) ?? null,
	merchantName: (overrides.merchantName as string | null) ?? null,
	purpose: (overrides.purpose as string | null) ?? null,
	categoryId: (overrides.categoryId as string | null) ?? null,
	month: (overrides.month as string) ?? "2024-01",
	paymentStatus: (overrides.paymentStatus as string) ?? "posted",
	installmentMarker: (overrides.installmentMarker as string | null) ?? null,
	reviewed: bool(overrides.reviewed ?? false),
	isInstallment: bool(overrides.isInstallment ?? false),
	isSubscription: bool(overrides.isSubscription ?? false),
	commitmentTier: (overrides.commitmentTier as string | null) ?? null,
});

describe("Incremental transaction seeding", () => {
	let store: Awaited<ReturnType<typeof createStorePromise>>;

	beforeAll(async () => {
		store = await createStorePromise({
			schema,
			storeId: "seed-test",
			adapter: makeInMemoryAdapter(),
			debug: { instanceId: "seed-test" },
		});
	});

	beforeEach(() => {
		// Clear previous test data so each test starts fresh.
		store.commit(events.transactionsSeeded({ rows: [] }));
	});

	afterAll(() => {
		store?.sqliteDbWrapper?.close?.();
	});

	it("first page deletes and inserts; second page appends without deleting", () => {
		const page1 = [makeTx("tx-a"), makeTx("tx-b"), makeTx("tx-c")];
		store.commit(events.transactionsSeeded({ rows: page1 }));

		const afterPage1 = store.query(
			tables.transactions.select().orderBy("id", "asc"),
		);
		expect(afterPage1.length).toBe(3);
		expect(afterPage1.map((r: { id: string }) => r.id)).toEqual([
			"tx-a",
			"tx-b",
			"tx-c",
		]);

		const page2 = [makeTx("tx-d"), makeTx("tx-e")];
		store.commit(events.transactionsPageSeeded({ rows: page2 }));

		const afterAll = store.query(
			tables.transactions.select().orderBy("id", "asc"),
		);
		expect(afterAll.length).toBe(5);
		expect(afterAll.map((r: { id: string }) => r.id)).toEqual([
			"tx-a",
			"tx-b",
			"tx-c",
			"tx-d",
			"tx-e",
		]);
	});

	it("re-seed (another first page) replaces all rows", () => {
		store.commit(
			events.transactionsSeeded({
				rows: [makeTx("tx-1"), makeTx("tx-2")],
			}),
		);
		store.commit(
			events.transactionsPageSeeded({
				rows: [makeTx("tx-3")],
			}),
		);
		expect(store.query(tables.transactions.select()).length).toBe(3);

		const newPage = [makeTx("tx-alpha"), makeTx("tx-beta")];
		store.commit(events.transactionsSeeded({ rows: newPage }));

		const after = store.query(
			tables.transactions.select().orderBy("id", "asc"),
		);
		expect(after.length).toBe(2);
		expect(after.map((r: { id: string }) => r.id)).toEqual([
			"tx-alpha",
			"tx-beta",
		]);
	});

	it("empty page (transactionsPageSeeded with no rows) is a no-op", () => {
		store.commit(
			events.transactionsSeeded({
				rows: [makeTx("tx-only")],
			}),
		);
		store.commit(events.transactionsPageSeeded({ rows: [] }));

		const after = store.query(tables.transactions.select());
		expect(after.length).toBe(1);
		expect(after[0].id).toBe("tx-only");
	});

	it("transactionsPageSeeded before first page inserts normally", () => {
		store.commit(
			events.transactionsPageSeeded({
				rows: [makeTx("orphan")],
			}),
		);

		const after = store.query(tables.transactions.select());
		expect(after.length).toBe(1);
		expect(after[0].id).toBe("orphan");
	});

	it("normalization preserves commitment-tier overrides from the bridge", () => {
		const rows = normalizeTransactions([
			makeTx("tx-tier", { commitmentTier: "locked" }),
		]);
		store.commit(events.transactionsSeeded({ rows }));

		const after = store.query(
			tables.transactions.select().where({ id: "tx-tier" }),
		);
		expect(after.length).toBe(1);
		expect(after[0].commitmentTier).toBe("locked");
	});

	it("bridgeIdentityChanged clears stale local write state", () => {
		store.commit(
			events.reviewSubmitted({
				writeId: "review-stale",
				transactionId: "tx-stale",
				patch: {
					description: "ajuste local",
					merchantName: null,
					purpose: null,
					categoryId: "compras",
				},
				submittedAt: 1,
			}),
		);
		store.commit(
			events.forecastMoved({
				writeId: "forecast-stale",
				forecastId: "fc-stale",
				dueDate: "2026-06-10",
				movedAt: 2,
			}),
		);

		expect(store.query(tables.pendingWrites.select()).length).toBe(2);
		expect(store.query(tables.reviewOverlay.select()).length).toBe(1);
		expect(store.query(tables.forecastOverlay.select()).length).toBe(1);

		store.commit(
			events.bridgeIdentityChanged({
				oldIdentity: "unknown",
				newIdentity: "bigquery:test",
			}),
		);

		expect(store.query(tables.pendingWrites.select()).length).toBe(0);
		expect(store.query(tables.reviewOverlay.select()).length).toBe(0);
		expect(store.query(tables.forecastOverlay.select()).length).toBe(0);
	});
});

describe("Review write persistence (web C5)", () => {
	let store: Awaited<ReturnType<typeof createStorePromise>>;

	beforeAll(async () => {
		store = await createStorePromise({
			schema,
			storeId: "c5-test",
			adapter: makeInMemoryAdapter(),
			debug: { instanceId: "c5-test" },
		});
	});

	afterAll(() => {
		store?.sqliteDbWrapper?.close?.();
	});

	it("queues the FULL human patch for flush and clears on ack", () => {
		store.commit(
			events.reviewSubmitted({
				writeId: "w1",
				transactionId: "tx-9",
				patch: {
					description: "Zenilda Faxina",
					merchantName: "Zenilda",
					purpose: "Faxina mensal",
					categoryId: "moradia:servicos",
					commitmentTier: "locked",
				},
				submittedAt: 1,
			}),
		);

		const pending = store.query(tables.pendingWrites.select());
		expect(pending.length).toBe(1);
		expect(pending[0].type).toBe("review");
		expect(pending[0].transactionId).toBe("tx-9");
		// drainQueue() POSTs payload verbatim as the /api/events patch — the
		// human description/merchant/purpose must ride along, not just category
		// (regression: edits silently dropped, only category persisted).
		expect(pending[0].payload.description).toBe("Zenilda Faxina");
		expect(pending[0].payload.merchantName).toBe("Zenilda");
		expect(pending[0].payload.purpose).toBe("Faxina mensal");
		expect(pending[0].payload.categoryId).toBe("moradia:servicos");
		expect(pending[0].payload.commitmentTier).toBe("locked");

		// Optimistic overlay surfaces the edit immediately.
		const overlay = store.query(tables.reviewOverlay.select());
		expect(overlay.length).toBe(1);
		expect(overlay[0].description).toBe("Zenilda Faxina");
		expect(overlay[0].commitmentTier).toBe("locked");

		// On bridge ack the queued write is removed (flush succeeded).
		store.commit(events.writeAcked({ writeId: "w1" }));
		expect(store.query(tables.pendingWrites.select()).length).toBe(0);
	});
});

describe("Pending write failures", () => {
	let store: Awaited<ReturnType<typeof createStorePromise>>;

	beforeAll(async () => {
		store = await createStorePromise({
			schema,
			storeId: "pending-failures-test",
			adapter: makeInMemoryAdapter(),
			debug: { instanceId: "pending-failures-test" },
		});
	});

	afterAll(() => {
		store?.sqliteDbWrapper?.close?.();
	});

	it("records failed attempts without removing the queued write", () => {
		store.commit(
			events.reviewSubmitted({
				writeId: "wf1",
				transactionId: "tx-fail",
				patch: {
					description: "edit",
					merchantName: null,
					purpose: null,
					categoryId: "food",
				},
				submittedAt: 1,
			}),
		);
		store.commit(
			events.writeFailed({
				writeId: "wf1",
				error: "503 Service Unavailable",
				attempts: 1,
			}),
		);

		const pending = store.query(
			tables.pendingWrites.select().where({ writeId: "wf1" }),
		);
		expect(pending.length).toBe(1);
		expect(pending[0].attempts).toBe(1);
		expect(pending[0].lastError).toBe("503 Service Unavailable");
		expect(
			store.query(tables.reviewOverlay.select().where({ transactionId: "tx-fail" }))
				.length,
		).toBe(1);
	});

	it("abandons terminal review failures and rolls back their overlay", () => {
		store.commit(
			events.reviewSubmitted({
				writeId: "wa-review",
				transactionId: "tx-abandon",
				patch: {
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: "old",
				},
				submittedAt: 2,
			}),
		);
		store.commit(
			events.writeAbandoned({
				writeId: "wa-review",
				type: "review",
				transactionId: "tx-abandon",
				forecastId: "",
				error: "max retries exceeded",
			}),
		);

		expect(
			store.query(tables.pendingWrites.select().where({ writeId: "wa-review" }))
				.length,
		).toBe(0);
		expect(
			store.query(
				tables.reviewOverlay.select().where({ transactionId: "tx-abandon" }),
			).length,
		).toBe(0);
	});

	it("does not roll back a newer review overlay for the same transaction", () => {
		store.commit(
			events.reviewSubmitted({
				writeId: "wa-old",
				transactionId: "tx-race",
				patch: {
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: "old",
				},
				submittedAt: 5,
			}),
		);
		store.commit(
			events.reviewSubmitted({
				writeId: "wa-new",
				transactionId: "tx-race",
				patch: {
					description: null,
					merchantName: null,
					purpose: null,
					categoryId: "new",
				},
				submittedAt: 6,
			}),
		);
		store.commit(
			events.writeAbandoned({
				writeId: "wa-old",
				type: "review",
				transactionId: "tx-race",
				forecastId: "",
				error: "max retries exceeded",
			}),
		);

		const overlay = store.query(
			tables.reviewOverlay.select().where({ transactionId: "tx-race" }),
		);
		expect(overlay.length).toBe(1);
		expect(overlay[0].writeId).toBe("wa-new");
		expect(overlay[0].categoryId).toBe("new");
	});

	it("abandons terminal forecast failures and rolls back optimistic state", () => {
		store.commit(
			events.forecastMoved({
				writeId: "wa-move",
				forecastId: "fc-move",
				dueDate: "2026-08-10",
				movedAt: 3,
			}),
		);
		store.commit(
			events.writeAbandoned({
				writeId: "wa-move",
				type: "forecastMove",
				transactionId: "",
				forecastId: "fc-move",
				error: "max retries exceeded",
			}),
		);
		expect(
			store.query(tables.forecastOverlay.select().where({ forecastId: "fc-move" }))
				.length,
		).toBe(0);

		store.commit(
			events.forecastCreated({
				writeId: "wa-create",
				description: "manual forecast",
				amount: "-10.00",
				dueDate: "2026-08-01",
				createdAt: 4,
			}),
		);
		store.commit(
			events.writeAbandoned({
				writeId: "wa-create",
				type: "forecastCreate",
				transactionId: "",
				forecastId: "",
				error: "max retries exceeded",
			}),
		);
		expect(
			store.query(tables.forecasts.select().where({ forecastId: "wa-create" }))
				.length,
		).toBe(0);
	});

	it("does not roll back a newer forecast move overlay for the same forecast", () => {
		store.commit(
			events.forecastMoved({
				writeId: "move-old",
				forecastId: "fc-race",
				dueDate: "2026-08-10",
				movedAt: 7,
			}),
		);
		store.commit(
			events.forecastMoved({
				writeId: "move-new",
				forecastId: "fc-race",
				dueDate: "2026-09-10",
				movedAt: 8,
			}),
		);
		store.commit(
			events.writeAbandoned({
				writeId: "move-old",
				type: "forecastMove",
				transactionId: "",
				forecastId: "fc-race",
				error: "max retries exceeded",
			}),
		);

		const overlay = store.query(
			tables.forecastOverlay.select().where({ forecastId: "fc-race" }),
		);
		expect(overlay.length).toBe(1);
		expect(overlay[0].writeId).toBe("move-new");
		expect(overlay[0].dueDate).toBe("2026-09-10");
	});
});

describe("Envelope goal writes (war plan)", () => {
	let store: Awaited<ReturnType<typeof createStorePromise>>;

	beforeAll(async () => {
		store = await createStorePromise({
			schema,
			storeId: "envelope-test",
			adapter: makeInMemoryAdapter(),
			debug: { instanceId: "envelope-test" },
		});
	});

	afterAll(() => {
		store?.sqliteDbWrapper?.close?.();
	});

	it("create-mode queues a snake_case payload and inserts an optimistic envelope", () => {
		store.commit(
			events.forecastEnvelopeUpserted({
				writeId: "we1",
				forecastId: "",
				description: "meta alimentacao",
				amount: "-450.00",
				dueDate: "2026-07-31",
				categoryId: "alimentacao",
				upsertedAt: 1,
			}),
		);

		const pending = store.query(
			tables.pendingWrites.select().where({ writeId: "we1" }),
		);
		expect(pending.length).toBe(1);
		expect(pending[0].type).toBe("forecastEnvelope");
		// drainQueue POSTs this verbatim to /api/forecast.
		expect(pending[0].payload).toEqual({
			forecast_id: null,
			description: "meta alimentacao",
			amount: "-450.00",
			due_date: "2026-07-31",
			category_id: "alimentacao",
		});

		// The optimistic forecast row makes the envelope visible immediately.
		const optimistic = store.query(
			tables.forecasts.select().where({ forecastId: "we1" }),
		);
		expect(optimistic.length).toBe(1);
		expect(optimistic[0]).toMatchObject({
			amount: "-450.00",
			categoryId: "alimentacao",
			dueDate: "2026-07-31",
			status: "ativo",
			kind: "manual",
		});

		store.commit(events.writeAcked({ writeId: "we1" }));
		expect(
			store.query(tables.pendingWrites.select().where({ writeId: "we1" }))
				.length,
		).toBe(0);
	});

	it("update-mode re-amounts the existing forecast row in place", () => {
		store.commit(
			events.forecastsSeeded({
				rows: [
					{
						forecastId: "f-env-1",
						dueDate: "2026-06-05",
						description: "envelope casa",
						amount: "-700.00",
						categoryId: "moradia",
						accountId: null,
						status: "ativo",
						kind: "manual",
						draggable: 1,
					},
				],
			}),
		);
		store.commit(
			events.forecastEnvelopeUpserted({
				writeId: "we2",
				forecastId: "f-env-1",
				description: "",
				amount: "-350.00",
				dueDate: "2026-06-30",
				categoryId: "moradia",
				upsertedAt: 2,
			}),
		);

		const pending = store.query(
			tables.pendingWrites.select().where({ writeId: "we2" }),
		);
		expect(pending[0].payload).toEqual({
			forecast_id: "f-env-1",
			description: "",
			amount: "-350.00",
			due_date: "2026-06-30",
			category_id: "moradia",
		});

		const rows = store.query(
			tables.forecasts.select().where({ forecastId: "f-env-1" }),
		);
		expect(rows.length).toBe(1);
		expect(rows[0].amount).toBe("-350.00");
		expect(rows[0].dueDate).toBe("2026-06-30");
		// No phantom extra row keyed by the writeId.
		expect(
			store.query(tables.forecasts.select().where({ forecastId: "we2" }))
				.length,
		).toBe(0);
	});
});
