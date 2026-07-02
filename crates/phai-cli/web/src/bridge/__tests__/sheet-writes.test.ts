// @ts-nocheck — store type inference is overly strict with synced events;
//               tests pass at runtime (same caveat as seed.test.ts).
/**
 * Unified-sheet baseline writes: `forecastDiscarded` and
 * `forecastTemplateEnded` queue the right pendingWrites rows and gray the
 * affected forecasts out optimistically (status "descartado").
 */
import { makeInMemoryAdapter } from "@livestore/adapter-web";
import { createStorePromise } from "@livestore/livestore";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { events, schema, tables } from "../../livestore/schema";

const seedForecast = (overrides: Partial<Record<string, unknown>> = {}) => ({
	forecastId: (overrides.forecastId as string) ?? "f1",
	dueDate: (overrides.dueDate as string | null) ?? "2026-08-10",
	description: (overrides.description as string) ?? "assinatura",
	amount: (overrides.amount as string) ?? "-49.90",
	categoryId: null,
	accountId: null,
	status: (overrides.status as string) ?? "ativo",
	kind: (overrides.kind as string) ?? "manual",
	draggable: 1,
	templateId: (overrides.templateId as string | null) ?? null,
	realizedTransactionId: null,
	realizedAt: null,
	metadataJson: {},
});

describe("unified-sheet baseline write events", () => {
	let store: Awaited<ReturnType<typeof createStorePromise>>;

	beforeAll(async () => {
		store = await createStorePromise({
			schema,
			storeId: "sheet-writes-test",
			adapter: makeInMemoryAdapter(),
			debug: { instanceId: "sheet-writes-test" },
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

	it("forecastDiscarded queues a forecastDiscard write and discards optimistically", () => {
		store.commit(
			events.forecastsSeeded({
				rows: [seedForecast({ forecastId: "f1", templateId: "tpl-1", kind: "template" })],
			}),
		);
		store.commit(
			events.forecastDiscarded({ writeId: "w1", forecastId: "f1", discardedAt: 1 }),
		);
		const pending = store.query(tables.pendingWrites.select());
		expect(pending).toHaveLength(1);
		expect(pending[0].type).toBe("forecastDiscard");
		expect(pending[0].forecastId).toBe("f1");
		expect(store.query(tables.forecasts.select())[0].status).toBe("descartado");

		store.commit(events.writeAcked({ writeId: "w1" }));
		expect(store.query(tables.pendingWrites.select())).toHaveLength(0);
	});

	it("forecastTemplateEnded queues the end write and discards the listed forecasts only", () => {
		store.commit(
			events.forecastsSeeded({
				rows: [
					seedForecast({ forecastId: "f-aug", dueDate: "2026-08-05", templateId: "tpl-1" }),
					seedForecast({ forecastId: "f-sep", dueDate: "2026-09-05", templateId: "tpl-1" }),
					seedForecast({ forecastId: "f-jul", dueDate: "2026-07-05", templateId: "tpl-1" }),
				],
			}),
		);
		store.commit(
			events.forecastTemplateEnded({
				writeId: "w1",
				templateId: "tpl-1",
				effectiveFrom: "2026-08",
				forecastIds: ["f-aug", "f-sep"],
				endedAt: 1,
			}),
		);
		const pending = store.query(tables.pendingWrites.select());
		expect(pending).toHaveLength(1);
		expect(pending[0].type).toBe("forecastTemplateEnd");
		expect(pending[0].payload).toEqual({
			templateId: "tpl-1",
			effectiveFrom: "2026-08",
		});
		const statuses = new Map(
			store.query(tables.forecasts.select()).map((f) => [f.forecastId, f.status]),
		);
		expect(statuses.get("f-aug")).toBe("descartado");
		expect(statuses.get("f-sep")).toBe("descartado");
		// The July row precedes the cutoff and stays active.
		expect(statuses.get("f-jul")).toBe("ativo");
	});

	it("forecastEnvelopeUpserted accepts a null categoryId (keep-stored patch)", () => {
		store.commit(
			events.forecastsSeeded({ rows: [seedForecast({ forecastId: "f1" })] }),
		);
		store.commit(
			events.forecastEnvelopeUpserted({
				writeId: "w1",
				forecastId: "f1",
				description: "",
				amount: "-25.00",
				dueDate: "2026-08-10",
				categoryId: null,
				upsertedAt: 1,
			}),
		);
		const pending = store.query(tables.pendingWrites.select());
		expect(pending[0].type).toBe("forecastEnvelope");
		expect(pending[0].payload.category_id).toBeNull();
		expect(pending[0].payload.forecast_id).toBe("f1");
		expect(store.query(tables.forecasts.select())[0].amount).toBe("-25.00");
	});
});
