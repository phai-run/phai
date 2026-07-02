// @ts-nocheck — store type inference is overly strict with synced events;
//               tests pass at runtime (same caveat as seed.test.ts).
/**
 * Planning-scenario events (ADR-0037): materializers queue the right
 * pendingWrites rows, keep the optimistic tables in sync, and the
 * normalizers translate the bridge's snake_case + orphan list.
 */
import { makeInMemoryAdapter } from "@livestore/adapter-web";
import { createStorePromise } from "@livestore/livestore";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { events, schema, tables } from "../../livestore/schema";
import { normalizeScenarioChanges, normalizeScenarios } from "../sync";

const changeRow = (overrides: Partial<Record<string, unknown>> = {}) => ({
	changeId: (overrides.changeId as string) ?? "chg-1",
	scenarioId: (overrides.scenarioId as string) ?? "scn-1",
	kind: (overrides.kind as string) ?? "add_one_shot",
	targetForecastId: (overrides.targetForecastId as string | null) ?? null,
	targetTemplateId: (overrides.targetTemplateId as string | null) ?? null,
	month: (overrides.month as string | null) ?? "2026-09",
	effectiveFrom: (overrides.effectiveFrom as string | null) ?? null,
	amount: (overrides.amount as string | null) ?? "-2000.00",
	monthsCount: (overrides.monthsCount as number | null) ?? null,
	description: (overrides.description as string | null) ?? "viagem",
	categoryId: (overrides.categoryId as string | null) ?? null,
	accountId: (overrides.accountId as string | null) ?? null,
	status: (overrides.status as string) ?? "ativo",
	orphaned: (overrides.orphaned as number) ?? 0,
});

describe("planning-scenario events", () => {
	let store: Awaited<ReturnType<typeof createStorePromise>>;

	beforeAll(async () => {
		store = await createStorePromise({
			schema,
			storeId: "scenario-test",
			adapter: makeInMemoryAdapter(),
			debug: { instanceId: "scenario-test" },
		});
	});

	beforeEach(() => {
		store.commit(
			events.bridgeIdentityChanged({ oldIdentity: "a", newIdentity: "b" }),
		);
	});

	afterAll(() => {
		store?.sqliteDbWrapper?.close?.();
	});

	it("scenarioCreated queues the write and inserts optimistically", () => {
		store.commit(
			events.scenarioCreated({
				writeId: "w1",
				scenarioId: "scn-1",
				name: "com carro novo",
				description: null,
				createdAt: 1,
			}),
		);
		const scenarios = store.query(tables.scenarios.select());
		expect(scenarios.length).toBe(1);
		expect(scenarios[0].name).toBe("com carro novo");
		const pending = store.query(tables.pendingWrites.select());
		expect(pending.length).toBe(1);
		expect(pending[0].type).toBe("scenarioCreate");
		expect(pending[0].payload.scenarioId).toBe("scn-1");

		// The flusher's ack drains the queue but keeps the optimistic row.
		store.commit(events.writeAcked({ writeId: "w1" }));
		expect(store.query(tables.pendingWrites.select()).length).toBe(0);
		expect(store.query(tables.scenarios.select()).length).toBe(1);
	});

	it("scenarioChangeAdded / Removed round-trip the change table", () => {
		store.commit(
			events.scenarioChangeAdded({ writeId: "w1", row: changeRow(), addedAt: 1 }),
		);
		expect(store.query(tables.scenarioChanges.select()).length).toBe(1);
		const pending = store.query(tables.pendingWrites.select());
		expect(pending[0].type).toBe("scenarioChange");
		expect(pending[0].payload.kind).toBe("add_one_shot");
		expect(pending[0].payload.amount).toBe("-2000.00");

		store.commit(
			events.scenarioChangeRemoved({
				writeId: "w2",
				changeId: "chg-1",
				scenarioId: "scn-1",
				removedAt: 2,
			}),
		);
		expect(store.query(tables.scenarioChanges.select()).length).toBe(0);
		const types = store
			.query(tables.pendingWrites.select())
			.map((r) => r.type)
			.sort();
		expect(types).toEqual(["scenarioChange", "scenarioChangeDelete"]);
	});

	it("scenarioDeleted drops the scenario and its changes", () => {
		store.commit(
			events.scenarioCreated({
				writeId: "w1",
				scenarioId: "scn-1",
				name: "x",
				description: null,
				createdAt: 1,
			}),
		);
		store.commit(
			events.scenarioChangeAdded({ writeId: "w2", row: changeRow(), addedAt: 2 }),
		);
		store.commit(
			events.scenarioDeleted({ writeId: "w3", scenarioId: "scn-1", deletedAt: 3 }),
		);
		expect(store.query(tables.scenarios.select()).length).toBe(0);
		expect(store.query(tables.scenarioChanges.select()).length).toBe(0);
	});

	it("scenarioPromoted / Archived flip the optimistic status", () => {
		store.commit(
			events.scenarioCreated({
				writeId: "w1",
				scenarioId: "scn-1",
				name: "x",
				description: null,
				createdAt: 1,
			}),
		);
		store.commit(
			events.scenarioPromoted({ writeId: "w2", scenarioId: "scn-1", promotedAt: 2 }),
		);
		expect(store.query(tables.scenarios.select())[0].status).toBe("promovido");
		store.commit(
			events.scenarioArchived({ writeId: "w3", scenarioId: "scn-1", archivedAt: 3 }),
		);
		expect(store.query(tables.scenarios.select())[0].status).toBe("arquivado");
	});

	it("scenarioChangesSeeded replaces only that scenario's rows", () => {
		store.commit(
			events.scenarioChangesSeeded({
				scenarioId: "scn-1",
				rows: [changeRow({ changeId: "chg-a" })],
			}),
		);
		store.commit(
			events.scenarioChangesSeeded({
				scenarioId: "scn-2",
				rows: [changeRow({ changeId: "chg-b", scenarioId: "scn-2" })],
			}),
		);
		expect(store.query(tables.scenarioChanges.select()).length).toBe(2);
		// Re-seeding scn-1 with fresh rows must not disturb scn-2.
		store.commit(
			events.scenarioChangesSeeded({
				scenarioId: "scn-1",
				rows: [changeRow({ changeId: "chg-c" })],
			}),
		);
		const ids = store
			.query(tables.scenarioChanges.select())
			.map((r) => r.changeId)
			.sort();
		expect(ids).toEqual(["chg-b", "chg-c"]);
	});

	it("bridgeIdentityChanged clears the scenario tables", () => {
		store.commit(
			events.scenarioCreated({
				writeId: "w1",
				scenarioId: "scn-1",
				name: "x",
				description: null,
				createdAt: 1,
			}),
		);
		store.commit(
			events.scenarioChartSeeded({
				scenarioId: "scn-1",
				months: [
					{
						label: "set/26",
						scenarioId: "scn-1",
						month: "2026-09",
						inflows: "0",
						outflows: "0",
						forecastInflowsRemaining: "0",
						forecastOutflowsRemaining: "0",
						closingBalance: "0",
						projectedClosingBalance: "0",
						isFuture: 1,
						ordinal: 0,
					},
				],
			}),
		);
		store.commit(
			events.bridgeIdentityChanged({ oldIdentity: "b", newIdentity: "c" }),
		);
		expect(store.query(tables.scenarios.select()).length).toBe(0);
		expect(store.query(tables.scenarioChanges.select()).length).toBe(0);
		expect(store.query(tables.scenarioChartMonths.select()).length).toBe(0);
	});
});

describe("scenario normalizers", () => {
	it("normalizeScenarios maps snake_case", () => {
		const rows = normalizeScenarios([
			{ scenario_id: "scn-1", name: "plano", description: null, status: "ativo" },
		]);
		expect(rows[0]).toEqual({
			scenarioId: "scn-1",
			name: "plano",
			description: null,
			status: "ativo",
		});
	});

	it("normalizeScenarioChanges flags orphans from the server list", () => {
		const rows = normalizeScenarioChanges(
			[
				{
					change_id: "chg-1",
					scenario_id: "scn-1",
					kind: "skip_forecast",
					target_forecast_id: "f1",
					target_template_id: null,
					month: null,
					effective_from: null,
					amount: null,
					months_count: null,
					description: null,
					category_id: null,
					account_id: null,
					status: "ativo",
				},
				{
					change_id: "chg-2",
					scenario_id: "scn-1",
					kind: "add_one_shot",
					target_forecast_id: null,
					target_template_id: null,
					month: "2026-09",
					effective_from: null,
					amount: "-100.00",
					months_count: null,
					description: "viagem",
					category_id: null,
					account_id: null,
					status: "ativo",
				},
			],
			["chg-1"],
		);
		expect(rows[0].orphaned).toBe(1);
		expect(rows[1].orphaned).toBe(0);
	});
});
