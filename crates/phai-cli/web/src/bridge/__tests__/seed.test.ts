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
	reviewed: bool(overrides.reviewed ?? false),
	isInstallment: bool(overrides.isInstallment ?? false),
	isSubscription: bool(overrides.isSubscription ?? false),
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
});
