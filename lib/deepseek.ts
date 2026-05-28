import OpenAI from "openai";

const deepseek = new OpenAI({
	apiKey: process.env.DEEPSEEK_API_KEY || "",
	baseURL: "https://api.deepseek.com/v1",
});

const SYSTEM_PROMPT = `You are a world-class naming strategist. Generate creative, ownable names for any project, startup, app, or product.

Rules:
- Generate exactly 10 names.
- Each name must be distinct, memorable, and appropriate for the brief.
- Mix strategies: coined words, repurposed real words, compounds, foreign words, portmanteaus.
- Avoid clichés: -ify, -ly, -ai, -io suffixes. No Lumina, Nova, Nexus, Apex, etc.
- Names should be 3-10 characters unless the brief specifies otherwise.
- Cross-lingual safe: pronounceable in English, no awkward meanings in major languages.
- Do NOT repeat names from previous rounds.

Return ONLY valid JSON. No markdown, no prose outside the JSON:

{
  "names": [
    {
      "name": "Stripe",
      "strategy": "Repurposed real word",
      "rationale": "One sentence on the metaphor and why it fits the brief.",
      "domain_hint": "stripe.com (likely taken), stripe.io, stripe.dev"
    }
  ]
}`;

type GeneratedName = {
	name: string;
	strategy: string;
	rationale: string;
	domain_hint: string;
};

export async function generateNames(
	brief: string,
	previousNames?: string[],
): Promise<GeneratedName[]> {
	let userMessage = `Brief: ${brief}`;

	if (previousNames && previousNames.length > 0) {
		userMessage += `\n\nPreviously generated names (DO NOT repeat): ${previousNames.join(", ")}`;
	}

	const response = await deepseek.chat.completions.create({
		model: "deepseek-chat",
		messages: [
			{ role: "system", content: SYSTEM_PROMPT },
			{ role: "user", content: userMessage },
		],
		temperature: 0.9,
		max_tokens: 2048,
	});

	const text = response.choices[0]?.message?.content || "";

	// Extract JSON from response (handle markdown fences)
	let json = text.trim();
	const fenceMatch = json.match(/```(?:json)?\s*([\s\S]*?)```/);
	if (fenceMatch) json = fenceMatch[1].trim();

	const parsed = JSON.parse(json);
	return parsed.names || [];
}
