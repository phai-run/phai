# namezator

> AI-powered naming tool. Describe your project, get 10 names.

**namezator** generates creative, ownable names for startups, apps, CLI tools, products, and projects. Powered by DeepSeek. $1 per 10 rounds via Stripe.

## Stack

- **Frontend:** Next.js 14, TypeScript, CSS
- **AI:** DeepSeek API
- **Payments:** Stripe Checkout
- **Deploy:** Vercel

## Setup

```bash
npm install
cp .env.example .env.local
# Add DEEPSEEK_API_KEY, STRIPE_SECRET_KEY, NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY
npm run dev
```

## API

### POST /api/generate
Generate 10 names from a brief.

```json
{ "brief": "A CLI tool for personal finance..." }
```

### GET/POST /api/checkout
Create Stripe checkout session ($1/10 rounds).

## Env vars

| Key | Description |
|-----|-------------|
| `DEEPSEEK_API_KEY` | DeepSeek API key |
| `STRIPE_SECRET_KEY` | Stripe secret key |
| `NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY` | Stripe publishable key |
| `INITIAL_ROUNDS` | Free rounds on startup (default: 10) |
