"use client";

import { useState } from "react";

type NameResult = {
  name: string;
  strategy: string;
  rationale: string;
  domain_hint: string;
};

export default function Home() {
  const [brief, setBrief] = useState("");
  const [loading, setLoading] = useState(false);
  const [results, setResults] = useState<NameResult[] | null>(null);
  const [error, setError] = useState("");
  const [roundsLeft, setRoundsLeft] = useState<number | null>(null);

  const handleGenerate = async () => {
    if (!brief.trim()) return;
    setLoading(true);
    setError("");
    setResults(null);
    try {
      const res = await fetch("/api/generate", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ brief: brief.trim() }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || "Something went wrong");
      setResults(data.names);
      setRoundsLeft(data.remaining);
    } catch (e: any) {
      setError(e.message);
      if (e.message?.includes("No rounds")) {
        setRoundsLeft(0);
      }
    } finally {
      setLoading(false);
    }
  };

  const handlePurchase = async () => {
    setLoading(true);
    try {
      const res = await fetch("/api/checkout", { method: "POST" });
      const data = await res.json();
      if (data.url) window.location.href = data.url;
      else throw new Error(data.error || "Checkout failed");
    } catch (e: any) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  };

  return (
    <>
      {/* Hero */}
      <section className="hero">
        <div className="hero-phi">φ</div>
        <h1>Name your thing</h1>
        <p>AI-powered naming in seconds.<br />DeepSeek-driven, $1 for 10 rounds.</p>
      </section>

      {/* How */}
      <section className="alt" style={{ padding: "3rem 0" }}>
        <div className="container">
          <div className="stats">
            <div className="stat">
              <div className="num">$1</div>
              <div className="lbl">per 10 rounds</div>
            </div>
            <div className="stat">
              <div className="num">10</div>
              <div className="lbl">names per round</div>
            </div>
            <div className="stat">
              <div className="num">DeepSeek</div>
              <div className="lbl">LLM engine</div>
            </div>
            <div className="stat">
              <div className="num">Stripe</div>
              <div className="lbl">payments</div>
            </div>
          </div>
        </div>
      </section>

      {/* Form */}
      <section id="generate">
        <div className="container">
          <h2>What are you <em>naming</em>?</h2>
          <p style={{ color: "var(--muted)", marginBottom: "1.5rem", fontSize: "0.92rem" }}>
            Describe your project, startup, app, or product. The more detail, the better the names.
          </p>
          <div className="form-group">
            <textarea
              placeholder="e.g. A CLI tool for personal finance. Connects to Brazilian banks, syncs transactions, reports via WhatsApp. Audience: devs and families. Max 8 chars, no hyphens, avoid clichés."
              value={brief}
              onChange={(e) => setBrief(e.target.value)}
            />
          </div>
          <div style={{ display: "flex", gap: "1rem", flexWrap: "wrap", alignItems: "center" }}>
            <button className="btn btn-primary" onClick={handleGenerate} disabled={loading || !brief.trim()}>
              {loading ? "Generating…" : "Generate Names →"}
            </button>
            {roundsLeft !== null && roundsLeft > 0 && (
              <span style={{ color: "var(--muted)", fontSize: "0.85rem" }}>
                {roundsLeft} round{roundsLeft !== 1 ? "s" : ""} left
              </span>
            )}
            {(roundsLeft === null || roundsLeft === 0) && (
              <button className="btn btn-primary" style={{ background: "var(--cyan)", color: "var(--bg)" }} onClick={handlePurchase} disabled={loading}>
                Buy 10 Rounds · $1 →
              </button>
            )}
          </div>
          {error && <div className="error">{error}</div>}
        </div>
      </section>

      {/* Results */}
      {results && (
        <section className="alt">
          <div className="container">
            <h2>Your <em>names</em></h2>
            <div className="results-grid">
              {results.map((r, i) => (
                <div key={i} className="result-card">
                  <div className="name">{r.name}</div>
                  <div className="strategy">{r.strategy}</div>
                  <div className="rationale">{r.rationale}</div>
                  <div className="domains">
                    <span className="domain-tag">{r.domain_hint}</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        </section>
      )}

      <footer>
        <div className="container">
          φ phai · naming-as-a-service<br />
          powered by <a href="https://deepseek.com" target="_blank" rel="noopener">DeepSeek</a> · payments by <a href="https://stripe.com" target="_blank" rel="noopener">Stripe</a>
        </div>
      </footer>
    </>
  );
}
