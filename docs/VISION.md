# phai — Product Vision

*A living document. Update it as the vision evolves.*

---

## Why this, why now

Most personal finance tools are dashboards. You log in, you look at a chart, you close the tab, you go back to making the same decision blind. The data is hostage to a UI, a vendor, and a country.

phai rejects that shape. It is a **runtime**, not a dashboard:

- One binary on your machine.
- A real relational database underneath, queryable in SQL.
- Reports designed for the place you actually read them — your phone, on WhatsApp, in 10 seconds.
- A first-class agent surface, so an AI can answer "did my card close higher than last month?" without screen-scraping anything.

The bet: if your finance data lives in a queryable, scriptable, replayable system, you make different decisions. Not better dashboards — better *behavior*.

---

## The problem

People who care about their money face a recurring shape of frustration:

1. **The data is stuck.** Notion, Mobills, Organizze, the bank app — each owns a slice, none expose it as a queryable whole. CSV exports lose categorization. Manual reconciliation is the tax for portability.
2. **The interface dictates the question.** Dashboards answer the questions they were designed for. The one you actually have ("how did my groceries change since I moved?") requires a feature request.
3. **AI can't help.** Even with ChatGPT, an LLM cannot answer a finance question without structured data and a stable API. Screenshots of a banking app don't count.
4. **Privacy is binary.** Either you trust a SaaS with your full transaction history, or you accept zero automation. There is no middle ground that is also low-effort.

phai addresses all four with the same architectural choice: **a local-or-personal-cloud database the user controls, fed by open finance, queried in SQL, surfaced through human-readable and machine-readable reports.**

---

## The insight: a runtime, not an app

The shape that breaks the cycle is not another UI. It is a system that:

- **Owns the data.** Pluggy syncs into a database the user controls (SQLite on disk, or BigQuery in the user's GCP project).
- **Exposes everything.** SQL is the API. The CLI is convenience. Reports are presets.
- **Is replayable.** Every write is an event. You can rebuild every report from scratch, audit every change, undo any correction.
- **Speaks both audiences.** Human reports are formatted for phone reading. `--raw` JSON exists for the same query.

A runtime, not an app. The author's daily WhatsApp pipeline is one consumer of that runtime; an AI agent answering finance questions is another; the `phai serve` web app is a third. None of them is *the* product. The runtime is.

---

## The method: how to organize personal finance

phai is opinionated — these are the conventions that make the system useful out of the box.

### Categorization is a function, not a label

A transaction's category is not stored on the row. It is **derived** from rules, overrides, and effective views. This means:

- Re-running classification is safe.
- Bulk corrections are a rule edit, not a database update.
- Different users (and different agents) see the same raw data with different category lenses.

Rules are runtime data — never code, never migrations. Personal classification logic lives in the user's database, not in the open-source repository.

### Splits respect reality

A single transaction at the supermarket is groceries + cleaning supplies + a treat for the dog. The bank sees one charge; the user lives three categories. `tx split` makes the split first-class. Reports use split children when present, parent otherwise.

### Internal transfers don't count

Moving money between your own accounts is not income or expense. Internal-category infrastructure (migration `010`, `011`) ensures these never pollute reports. Adding a new account is a one-time setup; the runtime takes care of exclusion forever.

### Installments are chains, not events

`12x R$ 200` is not 12 unrelated R$ 200 charges. It is one decision, twelve payments. `installments.rs` recognizes parcela markers (Pluggy structured and free-text), groups them into chains, and projects the end. This is what makes "I'm still paying off the trip" a tractable question.

### Every write is an event

The append-only audit log is the foundation of trust. When something looks wrong in a report, you don't guess — you replay. When you make a correction, you don't lose the prior state. Over years, this is the difference between a database you trust and one you doubt.

### Human format is the default, JSON is a peer

Every report works on a phone first. Emoji prefixes, category grouping, a single number that summarizes the period. `--raw` is the same data shaped for an agent. Both are first-class — neither is a fallback.

---

## The foundation: an architecture that earns trust

The method only works if the system underneath is honest. phai leans on five architectural commitments:

### Decimal precision, end-to-end

Floats never touch an amount. `rust_decimal::Decimal` from the API to the database row. The user never sees a cent drift; reconciliation never disagrees with the bank by R$ 0,01.

### One binary, atomic update

Install: a `curl | bash` and you have `phai`. Update: the running binary fetches the next release, verifies the SHA-256, atomically swaps itself, and re-execs. No package manager. No upgrade ceremony. No version skew between machines.

### Dual backend behind one trait

Local-first via SQLite for the user who lives on one machine. BigQuery for the user with multiple devices. Same CLI, same reports — the `FinanceStore` trait makes the backend invisible to everything above storage.

### Open source, exit-friendly

The runtime is MIT-licensed. The schemas are SQL files in the repo. The audit log is queryable. If something better comes along, you take your database and leave. The exit door is always open by design.

### Privacy is a code rule

No personal counterparty names, account labels, or institution-specific fingerprints in shared source. Classification belongs in the runtime database. Shared fixtures use synthetic data. This is enforced in [AGENTS.md §1](../AGENTS.md#1-privacy--data-hygiene-hard-rules) — not as a guideline but as the first hard rule.

---

## Who it's for, and where it's going

### Three stages of adoption

**Stage 1: One person, one runtime** *(current)*

A single user runs `phai` on a laptop. Pluggy syncs nightly. Reports run on demand or on a schedule. The user reads them on WhatsApp, an AI agent answers ad-hoc questions. The runtime is invisible — it just keeps the answers accurate.

**Stage 2: Couples and households**

The same runtime, deployed to BigQuery, becomes shared across two people. Category overrides land in a shared Sheet. Both users sync their accounts; the audit log distinguishes who did what. Reports gain a "by actor" lens.

**Stage 3: Indie operators**

Freelancers and indie founders use the same runtime to separate personal and business finance, run cashflow projections, and answer "should I take this contract?" with real data. Multi-account, multi-currency, multi-tax-regime — all expressible as views over the same event log.

What stays constant across stages: the runtime, the SQL surface, the audit log, the decimal precision, the conventional commits → release pipeline. The trajectory extends — it does not pivot.

### Early adopters

The first people who'll get the most from phai are:

- Brazilians using Pluggy or curious about open finance.
- Engineers comfortable with a CLI and SQL.
- People who already track finances manually and resent the friction.
- AI-tooling enthusiasts who want their agent to answer finance questions with real data.

Broader audiences follow as the install surface widens and the convention library grows.

---

## Design principles

1. **The database is the source of truth.** Reports, agents, UIs are derivations. The database wins every disagreement.
2. **Decimal precision, end-to-end.** `rust_decimal::Decimal` everywhere. No `f64` on money.
3. **Every write is an event.** Append-only audit log. Replayable by construction.
4. **One binary, atomic update.** Single-binary install; in-place upgrade with SHA-256 verification.
5. **Dual backend behind one trait.** SQLite local-first, BigQuery production. Same CLI, same reports.
6. **Categorization is a function.** Rules + views resolve effective category. Never stored on the row.
7. **Human-friendly default, machine-friendly peer.** Every report has both a human and a `--raw` JSON shape.
8. **Privacy is a code rule.** No personal data in shared source. Classification lives in runtime data.
9. **Conventional Commits drive release.** Release Please owns the version and the changelog. No manual ceremony.
10. **Conventions over configuration.** Standard category infrastructure, standard internal-transfer handling, standard installment detection — all work out of the box; users can extend via rules.
