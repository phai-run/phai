/**
 * Category-family emoji. Mirrors the CLI mapping in
 * crates/phai-cli/src/human_format.rs (`category_emoji`) — keep both in sync
 * so the web app and terminal reports speak the same visual language.
 */
const FAMILY_EMOJI: Record<string, string> = {
	receitas: "💰",
	salario: "💰",
	assinaturas: "🔂",
	moradia: "🏠",
	casa: "🏠",
	alimentacao: "🍽️",
	saude: "🩺",
	transporte: "🚗",
	mobilidade: "🚗",
	educacao: "📚",
	lazer: "🎉",
	investimentos: "📈",
	financeiro: "🧾",
	vestuario: "👕",
};

export const categoryEmoji = (
	categoryId: string | null | undefined,
	isIncome = false,
): string => {
	if (isIncome) return "💰";
	const family = categoryId?.split(":")[0]?.trim().toLowerCase() || null;
	if (family == null) return "❓";
	if (family.startsWith("transfer")) return "🔁";
	return FAMILY_EMOJI[family] ?? "💸";
};
