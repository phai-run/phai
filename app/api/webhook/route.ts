import { type NextRequest, NextResponse } from "next/server";

let remainingRounds = parseInt(process.env.INITIAL_ROUNDS || "10");

export async function POST(req: NextRequest) {
  const Stripe = (await import("stripe")).default;
  const stripe = new Stripe(process.env.STRIPE_SECRET_KEY || "");

  const sig = req.headers.get("stripe-signature") || "";
  const body = await req.text();

  try {
    const event = stripe.webhooks.constructEvent(
      body, sig, process.env.STRIPE_WEBHOOK_SECRET || ""
    );

    if (event.type === "checkout.session.completed") {
      const session = event.data.object as any;
      const rounds = parseInt(session.metadata?.rounds || "10");
      remainingRounds += rounds;
      console.log(`✅ Payment received. ${rounds} rounds added. Total: ${remainingRounds}`);
    }

    return NextResponse.json({ received: true });
  } catch (err: any) {
    console.error("Webhook error:", err.message);
    return NextResponse.json({ error: err.message }, { status: 400 });
  }
}

export { remainingRounds };
