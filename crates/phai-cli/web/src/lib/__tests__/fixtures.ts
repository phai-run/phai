/**
 * Synthetic fixture generators for phai web tests.
 *
 * All data is plausible-but-fake. No real counterparty names, account labels,
 * or financial patterns derived from production data. See AGENTS.md §1.
 */

import type { TxView } from "../derivations";

export interface FixtureOptions {
	/** Number of months to generate (default 12). */
	months?: number;
	/** Transactions per month (default 50). */
	txsPerMonth?: number;
	/** Base year (default 2026). */
	startYear?: number;
	/** Base month 1-12 (default 1 = Jan). */
	startMonth?: number;
}

const MERCHANTS = [
	"Supermercado Bom Preço",
	"Padaria Pão Quente",
	"Restaurante Sabor Caseiro",
	"Farmácia Vida",
	"Posto Combustível Rápido",
	"Loja de Roupas Estilo",
	"Streaming Filmes+",
	"Academia Corpo e Mente",
	"Plano de Saúde Total",
	"Aluguel Residencial",
	"Condomínio Edifício Central",
	"Energia Elétrica S.A.",
	"Água e Saneamento",
	"Internet Fibra Rápida",
	"Mercado Hortifruti",
	"Livraria Cultura Digital",
	"Pet Shop Amigo Animal",
	"Transporte Público Municipal",
	"Delivery Comida Expressa",
	"Barbearia Corte Certo",
];

const CATEGORIES = [
	"alimentacao:mercado",
	"alimentacao:restaurante",
	"alimentacao:padaria",
	"saude:farmacia",
	"saude:plano",
	"transporte:combustivel",
	"transporte:publico",
	"moradia:aluguel",
	"moradia:condominio",
	"moradia:energia",
	"moradia:agua",
	"moradia:internet",
	"assinaturas:streaming",
	"assinaturas:academia",
	"compras:roupas",
	"compras:livros",
	"pets",
	"cuidados:barbearia",
	"lazer:delivery",
	null, // uncategorized
];

const ACCOUNTS = [
	{ id: "acc-cc-1", label: "Conta Corrente", owner: "felipe" },
	{ id: "acc-cc-2", label: "Conta Conjunta", owner: "maria" },
	{ id: "acc-card-1", label: "Cartão Master", owner: "felipe" },
	{ id: "acc-card-2", label: "Cartão Visa", owner: "maria" },
];

const INCOME_DESCRIPTIONS = [
	"Salário",
	"Freelance Projeto Web",
	"Reembolso Despesas",
	"Venda Item Usado",
	"Rendimento Investimento",
	"Bônus Trimestral",
];

// ── Generator ──────────────────────────────────────────────────────────────

let _seed = 42;

/** Simple seeded PRNG (mulberry32). */
function random(): number {
	_seed |= 0;
	_seed = (_seed + 0x6d2b79f5) | 0;
	let t = Math.imul(_seed ^ (_seed >>> 15), 1 | _seed);
	t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
	return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

function pick<T>(arr: readonly T[]): T {
	return arr[Math.floor(random() * arr.length)];
}

function amount(isIncome: boolean): string {
	const base = Math.floor(random() * 50000) / 100;
	if (isIncome) return String(Math.max(0.01, base * 100));
	return String(-Math.max(0.01, base));
}

function monthLabel(year: number, month: number): string {
	return `${year}-${String(month).padStart(2, "0")}`;
}

/**
 * Generate a set of synthetic transactions.
 *
 * @param opts Controls volume and window.
 * @returns Array of TxView records — pure data, no LiveStore or React deps.
 */
export function generateTransactions(opts: FixtureOptions = {}): TxView[] {
	const months = opts.months ?? 12;
	const txsPerMonth = opts.txsPerMonth ?? 50;
	const startYear = opts.startYear ?? 2026;
	const startMonth = opts.startMonth ?? 1;

	const result: TxView[] = [];
	let idCounter = 0;

	for (let m = 0; m < months; m++) {
		const year = startYear + Math.floor((startMonth - 1 + m) / 12);
		const mo = ((startMonth - 1 + m) % 12) + 1;
		const monthStr = monthLabel(year, mo);
		const daysInMonth = new Date(year, mo, 0).getDate();

		for (let i = 0; i < txsPerMonth; i++) {
			idCounter++;
			const day = Math.floor(random() * daysInMonth) + 1;
			const postedAt = `${monthStr}-${String(day).padStart(2, "0")}`;

			// 10% chance of income transaction
			const isIncome = random() < 0.1;
			const cat = isIncome ? null : pick(CATEGORIES);
			const merchant = isIncome ? null : pick(MERCHANTS);
			const desc = isIncome ? pick(INCOME_DESCRIPTIONS) : null;
			const account = pick(ACCOUNTS);

			// 5% chance of being an installment
			const isInstallment = !isIncome && random() < 0.05 ? 1 : 0;
			// 3% chance of being a subscription
			const isSubscription =
				!isIncome && !isInstallment && random() < 0.03 ? 1 : 0;
			// 60% chance of being reviewed
			const reviewed = random() < 0.6 ? 1 : 0;

			result.push({
				id: `tx-${String(idCounter).padStart(6, "0")}`,
				accountId: account.id,
				postedAt,
				amount: amount(isIncome),
				rawDescription: merchant ?? desc ?? "Transação",
				description: desc,
				merchantName: merchant,
				purpose: null,
				categoryId: cat,
				month: monthStr,
				paymentStatus: "cleared",
				reviewed,
				isInstallment,
				isSubscription,
			});
		}
	}

	return result;
}

/**
 * Generate accounts for use in tests.
 */
export function generateAccounts(): Array<{
	id: string;
	label: string;
	owner: string;
}> {
	return ACCOUNTS.map((a) => ({ ...a }));
}

/**
 * Generate category IDs for use in tests.
 */
export function generateCategoryIds(): string[] {
	return [...new Set(CATEGORIES.filter(Boolean))] as string[];
}

/**
 * Generate a high-volume fixture: 20k transactions over 12 months.
 */
export function generateLargeFixture(): TxView[] {
	return generateTransactions({ months: 12, txsPerMonth: 1667 });
}
