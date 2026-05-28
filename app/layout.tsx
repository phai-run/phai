import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "φ phai — Name Your Thing",
  description: "AI-powered naming. One brief, ten names, one dollar. DeepSeek-powered, Stripe-billed.",
  openGraph: {
    images: ["/phai-banner.svg"],
  },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
