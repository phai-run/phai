import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "namezator — AI Name Generator",
  description: "AI-powered naming tool. Describe your project, get 10 names. DeepSeek-powered, $1 per 10 rounds.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
