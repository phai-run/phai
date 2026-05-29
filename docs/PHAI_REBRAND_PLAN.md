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

## Phase 4 — Brand source files into repo + revise DESIGN.md ✅ DONE (commit ed75400)

**Goal:** brand assets live in the repo; DESIGN.md is sharper.

Source files currently on the `master` branch of the repo (orphan brand branch): `DESIGN.md`, `BRAND_BOOK.md`, `phai-brand.html`, `phai-logo.svg`, `phai-banner.svg`, `README.md`. Pull them with `gh api repos/phai-run/phai/contents/<f>?ref=master`.

- [x] Add `DESIGN.md` (root) and `phai-logo.svg`, `phai-banner.svg` (chose **`assets/brand/`** — consistent home; Phase 6 will reference them for favicon/OG).
- [x] **Do not** import `BRAND_BOOK.md` (deprecated). Extracted still-useful lines into DESIGN.md (pronunciation, `.run` verb, taglines, anti-brand, naming architecture); dropped the superseded gold/JetBrains-display palette.
- [x] **Revise DESIGN.md** to be more elegant/modern/refined/authentic:
  - [x] Tightened prose; φ+fi+ai equation is the spine.
  - [x] Added a **Motion** section (6s ease-in-out breathe on the hero φ only; honors `prefers-reduced-motion`).
  - [x] Added **Accessibility** guardrail: `muted2 #4A4A5E` on void ~2:1 — decorative only, never body text.
  - [x] Resolved the emoji contradiction — **one rule**: monoline glyphs `φ ⊹ ⌨ ◇` for decoration; emoji only inside simulated terminal blocks.
  - [x] Specified favicon/OG and the φ rendering rule (embedded vector path, not a font reference).
  - [x] Added a **token → CSS var** mapping table.
- [x] SVGs: converted the φ `<text>` to an embedded `<path>`. ⚠️ **Finding:** Playfair Display ships **no φ glyph** (only Δ Ω μ π); the source SVGs actually rendered φ via Georgia. Path extracted from **Georgia bold italic** (the faithful match, a high-contrast italic serif) via `fonttools`; DESIGN.md documents this and the `phi-display` token now points at the Georgia serif stack as the live-text fallback. Both SVGs verified rendering via `rsvg-convert`.

Subagent note: revised DESIGN.md in the main session; delegated the φ glyph→path extraction to a `general-purpose` subagent.

Commit: `docs(brand): add DESIGN.md + assets, refine the spec`

---

## Phase 5 — README rewrite + docs brand sweep + repo description ✅ DONE (commits d9e6139 + 314b505)

**Goal:** README sells phai in DESIGN.md voice; stale "Finance OS"/"finance-os" text across docs becomes "phai".

- [x] Rewrite `README.md`: hero (φ, name, tagline), the equation, rules-first/LLM-neutral pitch, terminal screenshot block, install one-liner (`curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash` until `phai.run/install.sh` DNS exists), quickstart (`phai sync`, `phai report`), links. Working URLs only.
- [x] Brand-text sweep (display strings, **not** crate identity): `finance-os` → `phai`, `Finance OS` → `phai`. Files include: `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`, `FINANCE_OS.md` (renamed → `REPORTING_UX.md` — it holds Reporting UX rules, not brand voice), `docs/*.md`, `docs/adr/*.md` (⚠️ **never edit an active ADR's decision** — only fix the product name in prose; if an ADR's identity changes materially, supersede it), `integrations/openclaw/skill/*`, `scripts/*`.
  - ⚠️ Leave `schema/sqlite/026_drop_phantom_account.sql` migration semantics alone — only touch comments if they name the product, never the SQL.
- [x] `gh repo edit phai-run/phai --description "φ Rules-first, LLM-neutral personal finance agent. Terminal-first, built in Rust." --homepage "https://phai.run"`

**This phase is the prime subagent candidate.** The brand-text sweep fans out across ~40 files. Spawn 2–3 parallel `general-purpose` subagents partitioned by directory (e.g. `docs/`, `docs/adr/`, `integrations/`+`scripts/`), each briefed with: the locked decisions, the "display string only — never crate identity, never ADR decisions, never SQL" rule, and its file list. Main session writes the README itself (craft) and reviews subagent diffs before committing.

Acceptance:
- [x] `grep -rn "Finance OS\|finance-os" --exclude-dir=.git .` returns only intentional/historical refs (CHANGELOG history; `finance-os.local.db`/`finance-os.db` filenames + `FINANCE_OS_*` envs from the deferred config.rs data-dir contract; the deferred OpenClaw skill deploy identity — `name: finance-os` + `skills/finance-os/finance.sh` paths; `target/` build artifacts).
- [x] Links in README resolve.

Commit(s): `docs: rewrite README for phai` + `docs: sweep product name to phai`

---

## Phase 6 — Landing page polish + GitHub Pages publish ✅ DONE (commit 1aa1967)

**Goal:** `phai-brand.html` becomes a spectacular, DESIGN.md-perfect site, served via GitHub Pages.

- [x] Pull `phai-brand.html` from `master`. Polish:
  - [x] Replace dead links: `github.com/phai` → `github.com/phai-run/phai`; keep `phai.run` as canonical home; install line uses the github-raw URL (`raw.githubusercontent.com/phai-run/phai/main/install.sh`) with a "phai.run/install.sh em breve" note until DNS exists.
  - [x] Apply Phase 4 emoji/glyph decision consistently — monoline glyphs (`φ ⌨ ◇ ⌂ ⊹ → ✓`) for decoration; emoji confined to the two terminal demos.
  - [x] Accessibility pass: moved all readable text off `muted2` → `muted`; `:focus-visible` purple ring on CTA links (and global); `prefers-reduced-motion: reduce` disables the breathe.
  - [x] Wire favicon → `phai-logo.svg`; OG/twitter meta → `phai-banner.svg`; `<title>`, description, `lang=pt-BR`.
  - [x] φ is now the **embedded vector `<path>`** (hero + footer), gradient cyan→purple→amber — never a font reference; dropped the Playfair Display font link. `h2 em` accent words → Space Grotesk cyan (never the serif).
  - [x] Spectacular polish: hairline section transitions, 80px vertical rhythm, terminal demos rebuilt as **real aligned lines** (`.ln` blocks), mobile pass at 600px. Removed dead `.audience-*` CSS.
- [x] Published via GitHub Pages: `phai-brand.html` → `docs/index.html`, assets (`phai-logo.svg`, `phai-banner.svg`) + `.nojekyll` moved alongside. Enabled Pages via `gh api -X POST repos/phai-run/phai/pages` (JSON body `{"source":{"branch":"main","path":"/docs"}}`). Site URL: `https://phai-run.github.io/phai/`. **No `CNAME`** — DNS not pointed (domain unregistered per Locked decisions); add `phai.run` CNAME when DNS exists.
- [x] Verified render in-browser via local static server (identical serve): hero vector φ + gradient, equation, DNA monoline glyphs, terminal lines, LLM-neutral, palette, CTA, footer φ; mobile 600px pass; favicon + OG SVGs resolve (200); reduced-motion + focus-visible present. ⚠️ **Live `phai-run.github.io/phai/` 404s until this branch merges to `main`** — Pages source is `main /docs` and the content lives on `chore/rename-phai`; it goes live automatically on merge.

Subagent note: keep the visual polish in the main session (it's iterative/judgment-heavy and benefits from screenshots). A subagent could do the mechanical link/meta fixes, but the "make it spectacular" work should stay where you can see it.

Commit: `feat(site): publish phai.run landing via GitHub Pages`

---

## Phase 7 — Verification & release

- [x] Full suite: `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace` — green (400 pass, 2 ignored).
- [x] `cargo audit` + `cargo deny check licenses` — licenses ok; audit reports 4 pre-existing **advisory warnings** (RUSTSEC-2025-0134 `rustls-pemfile` unmaintained via `yup-oauth2`; RUSTSEC-2026-0002 `lru` unsound via `ratatui`/the discontinued TUI). None are vulnerabilities, none introduced by the rename, none block.
- [x] `sentrux check .` clean; `sentrux gate .` no degradation (6995→6995).
- [x] Dry-run the install path: **agree.** `install.sh` `ASSET_PREFIX="phai-cli"`+`BINARY_NAME="phai"` → asset `phai-cli-<target>.tar.gz`; workflow `tar czf phai-cli-<target>.tar.gz phai`; `update.rs` `ASSET_NAME="phai-cli-…apple-darwin.tar.gz"`+`BINARY_NAME="phai"`, repo `phai-run/phai`. All three match.
- [x] **Review sweep finding (fixed this phase):** stale brand strings missed in Phases 2–3 — user-facing update messages (`update.rs:516,587` "finance-cli:" → "phai:"), the WhatsApp update notice (`sync_notify.rs:263` "finance-cli atualizado" → "phai atualizado"), the `refresh-installments` `--help` long_about (`main.rs` "finance-core::installments" → "phai-core::installments"), a tmpfile comment (`update_state.rs`), and the broken bigquery smoke-test hint (`-p finance-core` → `-p phai-core`). No tests asserted on these. fmt/clippy/test/sentrux re-run green.
- [x] README functional URLs resolve (repo, releases/latest, raw `main/install.sh` all 200). ⚠️ `main/install.sh` still serves the **pre-rename** script until this branch merges to `main` (same "live on merge" caveat as Pages).
- [x] `docs/adr/0001-single-binary-rust-cli.md` — **no change needed.** ADR-0001 decides the binary *shape* (single static Rust CLI), not its *name*; it never named `fin` and already reads "phai" (swept in Phase 5). The rename does not supersede it.
- [ ] **Open decision (user):** release strategy. The binary rename `fin`→`phai` is a **breaking change** for existing installs. Either (a) land a `feat!:`/`BREAKING CHANGE:` commit so Release Please bumps the major (3.x → 4.0) and the CHANGELOG documents "reinstall — the binary is now `phai`", or (b) keep current commits and note it in release notes only. Not yet decided.
- [ ] **Deferred (persisted-identity, same class as config-dir):** `metadata_json: json!({"origin": "finance-cli"})` at `main.rs:7907,12791,12927` writes provenance into DB rows. Changing it to `phai-cli` forks the data contract vs. existing rows — needs the same deliberate migration decision as the `~/.config/finance-os` data-dir, not a verification-phase edit. Left untouched.
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
| 2026-05-28 | 5 | ✅ README rewrite (commit d9e6139) + product-name sweep (commit 314b505). README in DESIGN.md voice (φ hero, equation, rules-first/LLM-neutral, terminal block, github-raw install with phai.run "coming soon", quickstart, links all resolve). Swept "Finance OS"/"finance-os" → "phai" across docs, ADR **prose only** (decisions untouched), OpenClaw SKILL.md/finance.sh, scripts, + one bigquery.rs comment. `FINANCE_OS.md` → `REPORTING_UX.md`. Fixed finance.sh to call renamed `phai` binary (functional). Docs/comment-only → no cargo/sentrux run. **Deferred (intentional, surfaced to user):** config.rs data-dir (`~/.config/finance-os`, `finance-os.local.db`, `FINANCE_OS_*` envs) + OpenClaw skill deploy identity (`name: finance-os`, `skills/finance-os/` paths) — both need a separate migration decision. `gh repo edit` run for description + homepage. |
| 2026-05-28 | 6 | ✅ Landing page polish + GitHub Pages publish (commit 1aa1967). Polished site → `docs/index.html` (+ `phai-{logo,banner}.svg`, `.nojekyll`): φ as embedded vector path (hero+footer), monoline glyphs only (emoji confined to terminal demos), dead links fixed (`github.com/phai`→`phai-run/phai`, github-raw install line, phai.run kept as home), a11y (text off muted2, `:focus-visible`, `prefers-reduced-motion`), favicon/OG/title/desc/lang, terminal demos rebuilt as aligned `.ln` lines, 80px rhythm, 600px mobile pass; dropped Playfair link + dead `.audience-*` CSS. Pages enabled on `main /docs` (`https://phai-run.github.io/phai/`); **no CNAME** (DNS unregistered). Verified via local static serve (live URL 404s until branch merges to main). Docs/assets only → no cargo/sentrux run. |
| 2026-05-28 | 7 | ✅ Verification + review sweep. Full suite green (fmt/clippy/test 400 pass), sentrux 6995→6995, licenses ok, audit = 4 pre-existing advisory warnings (no vulns, none from rename). Install path verified consistent end-to-end (`phai-cli-<target>.tar.gz` + `phai` binary, owner `phai-run`). README functional URLs 200. ADR-0001 needs no change (shape, not name). **Fixed missed brand strings** (user-facing): `update.rs` update messages, `sync_notify.rs` WhatsApp notice, `main.rs` refresh-installments help, + `update_state.rs` comment & bigquery smoke-test hint `-p finance-core`→`-p phai-core`. **Open (user):** release strategy (breaking binary rename → major bump vs release-notes-only). **Deferred:** `origin: "finance-cli"` provenance metadata (persisted-identity class, same as config-dir). **Uncommitted** pending user call on commit + release strategy. |
| 2026-05-28 | 4 | ✅ Brand files + DESIGN.md refine (commit ed75400). Added DESIGN.md (root) + `assets/brand/phai-{logo,banner}.svg`; φ now an embedded vector `<path>`. Refined DESIGN.md: motion, accessibility, emoji rule (monoline glyphs), favicon/OG, token→CSS-var table; folded salvaged BRAND_BOOK lines. **Finding:** Playfair has no φ glyph — path extracted from Georgia bold italic (the font the SVGs actually rendered). Docs/assets only, no `.rs` touched → no cargo/sentrux run. SVGs verified via `rsvg-convert`. |
