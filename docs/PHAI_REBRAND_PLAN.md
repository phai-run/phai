# φ phai — Rebrand & Rename Plan

> Living plan. Each phase is sized to run as its **own session**. Check items off as you go.
> Source of truth for the brand is [DESIGN.md](../DESIGN.md) (to be added in Phase 4). `BRAND_BOOK.md` is **deprecated** — do not follow it; useful bits are extracted into "Reference" below.

> ## ⚠️ Resuming an in-progress run — read first
> Work lives on branch **`chore/rename-phai`** (NOT `main`). Before doing anything:
> 1. `git switch chore/rename-phai` (or branch off it) — Phase 1's rename commits are here, not on main. If you start from main you will redo or conflict with completed work.
> 2. Check the **Progress log** at the bottom of this file for the last completed phase + commit hash.
> 3. Read **Locked decisions** + **Working agreements** below before editing.
> Then continue with the next unchecked phase.

---

## Locked decisions

| Topic | Decision |
|-------|----------|
| Binary name | `fin` → **`phai`** (breaking; users reinstall) |
| Crates | `finance-core` → **`phai-core`**, `finance-cli` → **`phai-cli`** |
| Canonical repo | **`github.com/phai-run/phai`** (repo already moved here) |
| GitHub org | `phai-run` (exists) |
| Domain | `phai.run` (not yet registered — use as **display/brand** surface; functional URLs use the GitHub repo until DNS exists) |
| Brand spec | **DESIGN.md is canonical.** BRAND_BOOK ignored. |
| Review TUI | **Being discontinued** — do not invest in rebranding it; migration target is `phai serve` (web). |
| Money type | `rust_decimal::Decimal` everywhere (never f64) — unchanged |

### URL policy
- **Functional** (install script src, self-update API, Cargo `repository`, release asset download): `https://github.com/phai-run/phai` and `https://raw.githubusercontent.com/phai-run/phai/main/...`. These must work today.
- **Display/marketing** (site hero, README title, CLI tagline footer, social): `phai.run`, `@phai`, `github.com/phai-run`. Mark domain-dependent links as "coming soon" until DNS is live.
- ⚠️ The local git remote still reads `feliperun/finance-os.git` (redirects, so it works). Update it once: `git remote set-url origin git@github.com:phai-run/phai.git`.

---

## Working agreements (read every session)

- **Never Read `crates/finance-cli/src/main.rs` in full** — it is ~482 KB / 14.4k lines and will blow the context window. Use `Grep` to locate, then `Read` with `offset`/`limit` on the exact range.
- Follow [AGENTS.md](../AGENTS.md): conventional commits, `cargo fmt`/`clippy -D warnings`/`test --workspace` green before commit, `sentrux gate .` shows no degradation on touched files, migrations idempotent in **both** backends, no `--no-verify`, no personal data in shared source.
- One bounded phase per PR. Small, focused, conventional-commit titles.
- **Subagent rule of thumb:** delegate *read-heavy fan-out* (multi-file sweeps, inventories, doc edits across many files) to parallel `Explore`/`general-purpose` subagents so raw file contents stay out of the main window. Keep *decisions and sequencing* in the main session. Brief each subagent cold: it has none of this context — paste the relevant rows from "Locked decisions" + the exact file list.

---

## Phase 0 — Prep (5 min, do once)

- [ ] `git remote set-url origin git@github.com:phai-run/phai.git`
- [ ] Branch off `main`: `git switch -c chore/rename-phai` (or per-phase branches)
- [ ] `sentrux gate --save .` to snapshot baseline (current Quality ≈ 6995)
- [ ] Confirm baseline builds: `cargo build --workspace`

---

## Phase 1 — Crate + binary rename (foundation; must compile) ✅ DONE (commit 1fa5f8c)

**Goal:** `finance-core`→`phai-core`, `finance-cli`→`phai-cli`, binary `fin`→`phai`. Pure identity rename, no behavior change. This unblocks every later phase.

**Do NOT delegate the mechanical sed to a subagent** — it's a handful of deterministic commands; running them in the main session is cheaper than briefing an agent. (A subagent can't `git mv` in your worktree anyway.)

Steps:
- [ ] `git mv crates/finance-core crates/phai-core` and `git mv crates/finance-cli crates/phai-cli`
- [ ] `Cargo.toml` (workspace): members → `crates/phai-core`, `crates/phai-cli`; update `repository = "https://github.com/phai-run/phai"`; `authors = ["phai contributors"]`
- [ ] `crates/phai-core/Cargo.toml`: `name = "phai-core"`
- [ ] `crates/phai-cli/Cargo.toml`: `name = "phai-cli"`; dep `finance-core` → `phai-core = { path = "../phai-core" }`; `[[bin]]` `name = "phai"`
- [ ] Code identifiers (crate names use underscores): in `crates/phai-cli/src/*.rs` replace `finance_core` → `phai_core` (8 files: main, serve, review, cashflow_chart, enrich, pulse, sync_notify, forecast_cmd). No `finance_cli` self-refs exist today (verified).
- [ ] `crates/phai-core/src/**`: replace any internal `finance_core` doc paths if present.
- [ ] `release-please-config.json`: update `extra-files` paths → `crates/phai-cli/Cargo.toml`, `crates/phai-core/Cargo.toml`; consider `package-name: "phai"`.
- [ ] `.github/workflows/release-please.yml`: `--package finance-cli` → `--package phai-cli`; asset filenames/tar `fin` → `phai`; output names `finance-cli-*` → `phai-cli-*` (cosmetic but keep consistent). **Coordinate with Phase 2** (install.sh ASSET_PREFIX must match the new asset name).
- [ ] Regenerate `Cargo.lock`: `cargo build --workspace`
- [ ] `crates/phai-core/CHANGELOG.md`, `crates/phai-cli/CHANGELOG.md`: rename headers if they embed crate names (low priority).

Acceptance:
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `sentrux gate .` — no degradation on touched files
- [ ] Binary builds as `phai`: `cargo run -p phai-cli -- --help`

Commit: `refactor: rename crates to phai-core/phai-cli and binary to phai`

---

## Phase 2 — Infra: install, self-update, release, dashboard, repo description ✅ DONE (commit b512592)

**Goal:** every functional pointer targets `phai-run/phai` and the `phai` binary; nothing 404s.

Files & changes:
- [x] `install.sh`: `REPO="phai-run/phai"`; `ASSET_PREFIX="phai-cli"`; `BINARY_NAME="phai"`; header comment + banner text + error prefixes + usage URL → phai.
- [x] `.github/workflows/release-please.yml` (deferred from Phase 1): `--package phai-cli`; asset/tar `phai-cli-<target>.tar.gz` containing `phai`; output names `finance-cli-*` → `phai-cli-*`.
- [x] `crates/phai-cli/src/update.rs`: `REPO_OWNER="phai-run"`, `REPO_NAME="phai"`, `REPO_URL`, asset names `phai-cli-*`, `BINARY_NAME="phai"`, user-agent `phai-cli/{version}`, + user-facing strings/tmpdir prefix/tests.
- [x] `crates/phai-cli/src/self_cmd.rs`: `finance self update` → `phai self update`.
- [x] `crates/phai-cli/src/serve_dashboard.html`: `<title>`/`<h1>` "Finance OS" → "phai".
- [x] `crates/phai-cli/src/review_template.html`: footer "Finance OS" → "phai".
- [x] `crates/phai-cli/src/main.rs`: stale `target/debug/finance-cli` comment → `phai`.
- [ ] **Deferred:** repo description via `gh repo edit` — shared-state remote mutation; Phase 5 sets the canonical copy, so do it there once.
- [ ] **Out of scope / flagged:** `crates/phai-core/src/config.rs` still uses `finance-os` for the on-disk config/data dir (`~/.config/finance-os`, `finance-os.local.db`). Renaming orphans existing users' data — needs a deliberate migration decision, not covered by this plan. Left untouched.

Acceptance:
- [x] `cargo test --workspace` green (400 pass).
- [x] No functional `feliperun/finance-os` or `BINARY_NAME=fin` left (only the config-dir paths above, intentionally deferred).

Commit: `chore: point install/self-update/release at phai-run/phai`

---

## Phase 3 — CLI branding (banner, --version, about) ✅ DONE (commit 075f704)

**Goal:** a tasteful φ touch in the CLI, in DESIGN.md voice. Terminal-first, no infantilizing, data > opinion.

- [x] clap root: `name = "phai"`, `about = "phai — finanças da casa, inteligência de verdade."` (was "Finance OS — `fin` abre a revisão TUI").
- [x] `--version`: disabled clap's auto flag (`disable_version_flag = true`), added a manual `-V`/`--version` bool + a `VERSION_BANNER` const, short-circuited in `main()` before any side effects. Renders φ glyph + version + tagline + `phai.run · github.com/phai-run/phai`. Plain text (no ANSI) by design — it gets piped/screenshotted.
- [x] Skipped the optional φ header in report/pulse output (keeps scope tight; no risk to `--json`).
- [x] Did not touch the review TUI.

Acceptance:
- [x] `phai --version`, `phai -V`, `phai --help` render the brand; no JSON output paths touched.
- [x] tests green (400 pass); sentrux no degradation (6995→6995).

Commit: `feat(cli): add phai branding to version and help`

---

## Phase 4 — Brand source files into repo + revise DESIGN.md

**Goal:** brand assets live in the repo; DESIGN.md is sharper.

Source files currently on the `master` branch of the repo (orphan brand branch): `DESIGN.md`, `BRAND_BOOK.md`, `phai-brand.html`, `phai-logo.svg`, `phai-banner.svg`, `README.md`. Pull them with `gh api repos/phai-run/phai/contents/<f>?ref=master`.

- [ ] Add `DESIGN.md` (root) and `phai-logo.svg`, `phai-banner.svg` (root or `assets/brand/`).
- [ ] **Do not** import `BRAND_BOOK.md` (deprecated). Extract any still-useful lines into DESIGN.md or README (see Reference).
- [ ] **Revise DESIGN.md** to be more elegant/modern/refined/authentic. Concrete upgrades to consider:
  - Tighten the prose; cut repetition. Keep the φ+fi+ai equation as the spine.
  - Add a short **motion** section (the hero φ "breathe" already exists — codify it: 6s ease-in-out brightness, nothing else moves).
  - Add **accessibility** guardrail: `muted2 #4A4A5E` on void is ~2:1 — decorative only, never body text. Body text uses `white`/`muted`.
  - Resolve the emoji contradiction: DESIGN.md says "no emojis except terminal output" but the landing uses 🏠⌨️🔧 in DNA cards. **Decide and state one rule** (recommendation: replace decorative emojis with monoline glyphs `φ ⊹ ⌨ ◇` for a more refined, authentic feel; keep emojis only inside simulated terminal blocks).
  - Specify favicon/OG: φ must render identically everywhere → convert φ to a vector **path** in the SVGs (Playfair-like italic), not a `font-family` reference (Georgia fallback currently breaks the look). See Phase 6.
  - Add a one-line **token → CSS var** mapping table so CLI/web/site stay in sync.
- [ ] SVGs: convert the φ `<text>` to a `<path>` so it doesn't depend on Georgia/Playfair being installed. (Generate the path from Playfair Display italic φ; embed.)

Subagent note: revising DESIGN.md is a single-file craft task — keep it in the main session. Converting the φ glyph to a path can be delegated to a `general-purpose` subagent with a crisp spec (input glyph, target font, output `<path d="...">`).

Commit: `docs(brand): add DESIGN.md + assets, refine the spec`

---

## Phase 5 — README rewrite + docs brand sweep + repo description

**Goal:** README sells phai in DESIGN.md voice; stale "Finance OS"/"finance-os" text across docs becomes "phai".

- [ ] Rewrite `README.md`: hero (φ, name, tagline), the equation, rules-first/LLM-neutral pitch, terminal screenshot block, install one-liner (`curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash` until `phai.run/install.sh` DNS exists), quickstart (`phai sync`, `phai report`), links. Working URLs only.
- [ ] Brand-text sweep (display strings, **not** crate identity): `finance-os` → `phai`, `Finance OS` → `phai`. Files include: `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, `FINANCE_OS.md` (consider renaming → `BRAND_VOICE.md` or fold into DESIGN.md), `docs/*.md`, `docs/adr/*.md` (⚠️ **never edit an active ADR's decision** — only fix the product name in prose; if an ADR's identity changes materially, supersede it), `integrations/openclaw/skill/*`, `scripts/*`.
  - ⚠️ Leave `schema/sqlite/026_drop_phantom_account.sql` migration semantics alone — only touch comments if they name the product, never the SQL.
- [ ] `gh repo edit phai-run/phai --description "φ Rules-first, LLM-neutral personal finance agent. Terminal-first, built in Rust." --homepage "https://phai.run"`

**This phase is the prime subagent candidate.** The brand-text sweep fans out across ~40 files. Spawn 2–3 parallel `general-purpose` subagents partitioned by directory (e.g. `docs/`, `docs/adr/`, `integrations/`+`scripts/`), each briefed with: the locked decisions, the "display string only — never crate identity, never ADR decisions, never SQL" rule, and its file list. Main session writes the README itself (craft) and reviews subagent diffs before committing.

Acceptance:
- [ ] `grep -rn "Finance OS\|finance-os" --exclude-dir=.git .` returns only intentional/historical refs (CHANGELOG history, superseded ADRs).
- [ ] Links in README resolve.

Commit(s): `docs: rewrite README for phai` + `docs: sweep product name to phai`

---

## Phase 6 — Landing page polish + GitHub Pages publish

**Goal:** `phai-brand.html` becomes a spectacular, DESIGN.md-perfect site, served via GitHub Pages.

- [ ] Pull `phai-brand.html` from `master`. Polish:
  - Replace dead links: `github.com/phai` → `github.com/phai-run/phai`; keep `phai.run` as canonical home (mark install as live only when DNS exists).
  - Apply Phase 4 emoji/glyph decision consistently.
  - Accessibility pass (contrast on `muted2`, focus states on the CTA pills, `prefers-reduced-motion` to disable the breathe animation).
  - Wire favicon → `phai-logo.svg`; OG/twitter meta → `phai-banner.svg`; `<title>`, description, lang.
  - Consider: real `install.sh` curl line, a "coming soon" treatment for WebApp, self-host fonts or `font-display: swap` (already swap).
  - Polish opportunities to make it "spectacular": refined section transitions, a subtle φ watermark, consistent vertical rhythm (80px per DESIGN.md), grid alignment, mobile pass at 600px.
- [ ] Publish via GitHub Pages. Recommended: `docs/` folder on `main` with `index.html` (rename `phai-brand.html` → `docs/index.html`, move assets alongside), then `gh api -X POST repos/phai-run/phai/pages -f source.branch=main -f source.path=/docs` (or enable in repo settings). Site lands at `https://phai-run.github.io/phai/`; add a `CNAME` file with `phai.run` once DNS is pointed.
- [ ] Verify render: open the published URL (or local) in a browser; check mobile, dark contrast, links, OG preview.

Subagent note: keep the visual polish in the main session (it's iterative/judgment-heavy and benefits from screenshots). A subagent could do the mechanical link/meta fixes, but the "make it spectacular" work should stay where you can see it.

Commit: `feat(site): publish phai.run landing via GitHub Pages`

---

## Phase 7 — Verification & release

- [ ] Full suite: `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace`
- [ ] `cargo audit` + `cargo deny check licenses`
- [ ] `sentrux check .` clean; `sentrux gate .` no degradation vs baseline.
- [ ] Dry-run the install path mentally: asset name (workflow) == `ASSET_PREFIX` (install.sh) == download in `update.rs`. They must all agree on `phai-cli-<target>.tar.gz` containing a `phai` binary.
- [ ] Decide release strategy: the binary rename is a **breaking change** for existing installs. Either (a) `feat!:`/`BREAKING CHANGE:` to bump major and document "reinstall: the binary is now `phai`", or (b) note it in release notes. Release Please will parse the commit.
- [ ] Update `docs/adr/0001-single-binary-rust-cli.md` only if the binary-name decision is material enough to supersede; otherwise prose-only name fix.
- [ ] Consider reserving package names: `crates.io` (`phai`), npm, PyPI (per Reference naming architecture) — out of repo scope, track separately.

---

## Reference — salvaged from the deprecated BRAND_BOOK (use only if it agrees with DESIGN.md)

- **Pronunciation:** "fai" (like "fly" without the L).
- **`.run` is the verb.** `phai run` framing; the domain is the manifesto. (Note: current CLI uses `phai sync` / `phai report`, not a `run` subcommand — keep actual commands as-is unless we deliberately add `run`.)
- **`--version` format** (adapt to DESIGN.md palette):
  ```
  φ phai v<version>
  finanças da casa, inteligência de verdade.
  phai.run · github.com/phai-run/phai
  ```
- **Anti-brand:** not a bank, not a brokerage; no gamification, no "congrats you saved R$12!", no 🚀, no "5 tips to…". (This matches DESIGN.md's Do/Don'ts.)
- **Naming architecture (future):** `phai.run` (site) · `app.phai.run` (web app) · `api.phai.run` · `docs.phai.run` · `github.com/phai-run`.
- **Taglines** (DESIGN.md/landing are canonical): primary "seu dinheiro em equilíbrio."; landing "finanças da casa, inteligência de verdade."; geek "φ = 1.618. sua família também."; direct "menos planilha. mais phi."
- ⚠️ BRAND_BOOK's palette (gold `#D4A843`/GitHub-dark), JetBrains-Mono-as-display, and "devs only, not normal people" audience are **superseded** by DESIGN.md (void+neon, Space Grotesk display, "families who think like engineers"). Do not reintroduce them.

---

## Progress log

| Date | Phase | Note |
|------|-------|------|
| 2026-05-28 | 0 | Plan created. Decisions locked. Repo moved to phai-run/phai. Branch `chore/rename-phai`. Sentrux baseline Quality 6995. |
| 2026-05-28 | 1 | ✅ Crate+binary rename done (commit 1fa5f8c). fmt/clippy/test green (400 pass), sentrux 6995→6995. Binary is now `phai`. Release-asset wiring + brand strings deferred to Phases 2–3 (see notes in those phases). git remote NOT yet updated (still feliperun URL, redirects fine). |
| 2026-05-28 | 2 | ✅ Infra pointers (commit b512592). install.sh, release-please.yml asset wiring (deferred from P1), update.rs (repo/asset/binary/user-agent), self_cmd.rs, both HTML files. 400 pass, sentrux 6995→6995. Deferred: `gh repo edit` (→ Phase 5). Flagged: config.rs still uses `finance-os` data-dir path — needs a migration decision, left untouched. |
| 2026-05-28 | 3 | ✅ CLI branding (commit 075f704). Branded `--version`/`-V` banner (φ glyph, plain text) via custom flag; new `about`; `name="phai"`. 400 pass, sentrux 6995→6995. |
