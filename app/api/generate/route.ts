import { type NextRequest, NextResponse } from "next/server";
import { generateNames } from "@/lib/deepseek";

let remainingRounds = parseInt(process.env.INITIAL_ROUNDS || "10");

export async function POST(req: NextRequest) {
  try {
    const { brief } = await req.json();

    if (!brief || typeof brief !== "string" || brief.trim().length < 10) {
      return NextResponse.json(
        { error: "Please provide a brief with at least 10 characters." },
        { status: 400 }
      );
    }

    if (remainingRounds <= 0) {
      return NextResponse.json(
        { error: "No rounds remaining. Purchase more to continue.", remaining: 0 },
        { status: 402 }
      );
    }

    const names = await generateNames(brief.trim());
    remainingRounds--;

    return NextResponse.json({
      names,
      remaining: remainingRounds,
    });
  } catch (error: any) {
    console.error("Generate error:", error);
    return NextResponse.json(
      { error: error.message || "Failed to generate names" },
      { status: 500 }
    );
  }
}
