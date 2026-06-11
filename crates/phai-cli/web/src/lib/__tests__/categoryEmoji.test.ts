/**
 * Mirrors crates/phai-cli/src/human_format.rs `category_emoji` — if these
 * expectations change, change the CLI mapping in the same commit.
 */
import { describe, expect, it } from "vitest";
import { categoryEmoji } from "../categoryEmoji";

describe("categoryEmoji", () => {
	it("maps the family (the part before the colon)", () => {
		expect(categoryEmoji("alimentacao:restaurantes")).toBe("🍽️");
		expect(categoryEmoji("moradia")).toBe("🏠");
		expect(categoryEmoji("casa:manutencao")).toBe("🏠");
		expect(categoryEmoji("saude:farmacia")).toBe("🩺");
		expect(categoryEmoji("transporte")).toBe("🚗");
		expect(categoryEmoji("mobilidade:apps")).toBe("🚗");
		expect(categoryEmoji("educacao")).toBe("📚");
		expect(categoryEmoji("lazer:viagens")).toBe("🎉");
		expect(categoryEmoji("investimentos")).toBe("📈");
		expect(categoryEmoji("financeiro:tarifas")).toBe("🧾");
		expect(categoryEmoji("vestuario")).toBe("👕");
		expect(categoryEmoji("assinaturas:streaming")).toBe("🔂");
	});

	it("treats income and the receitas/salario families as money", () => {
		expect(categoryEmoji("receitas:salario")).toBe("💰");
		expect(categoryEmoji("salario")).toBe("💰");
		expect(categoryEmoji("alimentacao:restaurantes", true)).toBe("💰");
	});

	it("matches any transfer-prefixed family", () => {
		expect(categoryEmoji("transferencias")).toBe("🔁");
		expect(categoryEmoji("transferencia:propria")).toBe("🔁");
	});

	it("falls back to expense for unknown families and question for none", () => {
		expect(categoryEmoji("outros:imprevistos")).toBe("💸");
		expect(categoryEmoji(null)).toBe("❓");
		expect(categoryEmoji(undefined)).toBe("❓");
		expect(categoryEmoji("")).toBe("❓");
	});
});
