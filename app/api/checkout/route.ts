import { type NextRequest, NextResponse } from "next/server";

const remainingRounds = parseInt(process.env.INITIAL_ROUNDS || "10");
const ROUNDS_PER_PURCHASE = 10;

export async function GET() {
	return NextResponse.json({ remaining: remainingRounds });
}

export async function POST(req: NextRequest) {
	// Create Stripe checkout session
	const Stripe = (await import("stripe")).default;
	const stripe = new Stripe(process.env.STRIPE_SECRET_KEY || "");

	const session = await stripe.checkout.sessions.create({
		payment_method_types: ["card"],
		line_items: [
			{
				price_data: {
					currency: "usd",
					product_data: {
						name: "10 Naming Rounds",
						description: "Generate 10 names per round. DeepSeek-powered.",
					},
					unit_amount: 100, // $1.00
				},
				quantity: 1,
			},
		],
		mode: "payment",
		success_url: `${req.nextUrl.origin}/?success=true`,
		cancel_url: `${req.nextUrl.origin}/?canceled=true`,
		metadata: { rounds: String(ROUNDS_PER_PURCHASE) },
	});

	return NextResponse.json({ url: session.url });
}
