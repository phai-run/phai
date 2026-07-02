/**
 * Schema-drift sentinel.
 *
 * LiveStore only hashes table SHAPES into its on-disk state filename — event
 * payload schemas and clientDocument value Structs are invisible to it. So a
 * change like adding a field to the `ui` document (v5.6.0) reuses the old
 * OPFS state DB and old rows fail to decode at query time, blanking the app.
 *
 * This test fingerprints everything that matters (sqlite table hash + every
 * event payload schema, including the derived `uiSet` which embeds the ui
 * value Struct) and pins it to STORE_VERSION. If you change the schema:
 *   1. bump STORE_VERSION in src/livestore/schema.ts, and
 *   2. re-record EXPECTED.fingerprint below (the failure output shows the
 *      new value).
 * Never update the fingerprint without bumping the version — that re-creates
 * the v5.6.0 bug. (A @livestore/effect dependency bump may also move the
 * fingerprint; re-record AND bump, it only costs one reseed.)
 */
import { Schema } from "@livestore/livestore";
import { describe, expect, it } from "vitest";
import { STORE_ID, STORE_VERSION, schema } from "../schema";

const EXPECTED = {
	storeVersion: 11,
	fingerprint: 1703898827,
};

const djb2 = (s: string): number => {
	let h = 5381;
	for (let i = 0; i < s.length; i++) {
		h = ((h << 5) + h + s.charCodeAt(i)) >>> 0;
	}
	return h;
};

const schemaFingerprint = (): number => {
	const eventPairs = [...schema.eventsDefsMap.entries()]
		.map(([name, def]) => [name, Schema.hash(def.schema)] as const)
		.sort((a, b) => a[0].localeCompare(b[0]));
	return djb2(
		JSON.stringify({ sqlite: schema.state.sqlite.hash, events: eventPairs }),
	);
};

describe("store version sentinel", () => {
	it("storeId derives from STORE_VERSION", () => {
		expect(STORE_ID).toBe(`phai-s${STORE_VERSION}`);
	});

	it("fingerprint covers the ui clientDocument set event", () => {
		// Guards the fingerprint itself: if a livestore upgrade renames
		// eventsDefsMap or stops deriving uiSet, the fingerprint would silently
		// stop covering the exact class of drift this sentinel exists for.
		expect([...schema.eventsDefsMap.keys()]).toContain("uiSet");
	});

	it("schema changes require a STORE_VERSION bump (re-record both together)", () => {
		expect(STORE_VERSION).toBe(EXPECTED.storeVersion);
		expect(schemaFingerprint()).toBe(EXPECTED.fingerprint);
	});
});
