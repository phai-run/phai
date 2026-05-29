# Getting Started

How to install phai, set it up, navigate the codebase, and find what you need.

> See [README.md](../README.md) for the marketing surface; this document is for someone about to *use* or *contribute to* phai.

## Prerequisites

- **Rust 1.90+** (`rustup update stable`) — only if building from source.
- **SQLite 3.x** — bundled in the binary; no external install needed for the local backend.
- **`git`** — required for development and Conventional Commit hygiene.
- **Pluggy account + credentials** — needed to sync from Brazilian banks. Free tier covers personal use.
- *(Optional)* **GCP project + service account** — only for the BigQuery backend.

## Install (end user)

```bash
curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh | bash
```

The installer detects your platform, downloads the matching binary into `~/.local/bin/phai`, verifies its SHA-256 against the published `.sha256` asset, and warns if `~/.local/bin` isn't in your `$PATH`.

Pin a version or change the install dir:

```bash
curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh \
  | bash -s -- --version=v0.5.1 --prefix=/usr/local
```

After install the binary self-updates: it checks GitHub Releases at most **once every 24h**, downloads, verifies SHA-256, atomically replaces itself, and re-execs your command. Set `FINANCE_OS_NO_AUTO_UPDATE=1` to opt out.

## First run (local SQLite backend — recommended start)

```bash
# 1. Initialize
phai auth setup --backend local --actor-id "$USER"
phai admin migrate

# 2. Configure Pluggy credentials
export PLUGGY_CLIENT_ID=...
export PLUGGY_CLIENT_SECRET=...
# (put these in your shell profile or a private .env you source)

# 3. Sync
phai sync pluggy --pluggy-config pluggy-config.json

# 4. See it
phai report daily-pulse
phai report card-summary
phai report monthly-spend
```

Data lives in `~/Library/Application Support/finance-os/finance-os.db` (macOS) or `~/.config/finance-os/finance-os.db` (Linux). Override with `FINANCE_OS_DATA_DIR`.

## First run (BigQuery backend)

Use this when you want the same dataset on multiple devices or you want to JOIN with Google Sheets.

```bash
# 1. In GCP: create a project + dataset (e.g. phai), create a service account
#    with BigQuery Data Editor + Job User, download its JSON key.

# 2. Tell phai
phai auth setup \
  --backend bigquery \
  --actor-id "$USER" \
  --project-id your-gcp-project \
  --dataset-id phai \
  --service-account-path /path/to/sa.json

# 3. Migrate + sync (same as local)
phai admin migrate
phai sync pluggy --pluggy-config pluggy-config.json
```

Optional: wire a Google Sheet as a category/context override source — see [google-sheets-overrides.md](google-sheets-overrides.md).

## Quick mental model

```
Pluggy ──sync──▶ FinanceStore ──views──▶ reports
                      │
                      └──audit_events (append-only log of every write)
```

- **You don't edit rows.** You issue commands (`tx categorize`, `tx set-context`, `tx split`) that emit `AuditEvent`s and update state.
- **You don't memorize categories.** You write **rules** (`phai rule upsert`) and the runtime resolves "effective category" via views.
- **Reports are presets.** They read views. If a question isn't answered by a preset, you can write SQL directly against the database.

For the full domain model see [ABSTRACTIONS.md](ABSTRACTIONS.md). For why each choice was made see [ARCHITECTURE.md](ARCHITECTURE.md) and the [ADR index](adr/README.md).

---

## Contributor setup

### Clone and build

```bash
git clone https://github.com/phai-run/phai.git
cd phai
cargo build --workspace
cargo test --workspace
```

`cargo test --workspace` runs both crates' tests. E2E tests live in `crates/phai-cli/tests/` and use a temporary SQLite directory (no network).

### The local feedback loop

```bash
cargo run -p phai-cli -- --help               # explore the CLI surface
cargo run -p phai-cli -- report daily-pulse   # iterate on a report
cargo test -p phai-cli                        # run E2E tests
cargo test -p phai-core                       # run unit tests
```

### Pre-PR checklist

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check licenses
cargo audit
sentrux check .                  # code-health gate (see AGENTS.md)
sentrux gate .                   # diff vs baseline; fails on degradation
```

CI runs all of this on every PR (`.github/workflows/ci.yml`).

### Conventional Commits

Release Please derives versions and `CHANGELOG.md` from your commit messages.

- `feat: …` → minor bump
- `fix: …` → patch bump
- `feat!: …` or `BREAKING CHANGE:` footer → major bump
- `docs:`, `test:`, `chore:`, `refactor:` → no version bump

Bad messages are reverted; this is the only ceremony around release.

### Where to find things

| Want to… | Look at |
|---|---|
| Add a CLI subcommand | `crates/phai-cli/src/main.rs` |
| Add or change a report | `crates/phai-core/src/storage/mod.rs` (trait) + `crates/phai-cli/src/human_format.rs` (presentation) |
| Add a migration | `schema/sqlite/NNN_*.sql` **and** `schema/bigquery/NNN_*.sql`; register in `crates/phai-core/src/migrations.rs` |
| Understand the storage trait | `crates/phai-core/src/storage/mod.rs` + [ABSTRACTIONS.md §FinanceStore](ABSTRACTIONS.md#the-storage-trait--financestore) |
| Understand Pluggy plumbing | `crates/phai-core/src/pluggy.rs` |
| Understand splits | `crates/phai-core/src/splits.rs` + `split_payload.rs` |
| Understand parcelas | `crates/phai-core/src/installments.rs` |
| Understand enrichment | `crates/phai-core/src/enrichment/` (pipeline, heuristics, cnpj, fuzzy, llm) |
| Read agent rules | [AGENTS.md](../AGENTS.md) |
| Read reporting voice | [REPORTING_UX.md](../REPORTING_UX.md) |
| Look up a decision | [docs/adr/README.md](adr/README.md) |

### Repository layout (recap)

```
phai/
├── crates/
│   ├── phai-core/           Domain + storage + Pluggy + rules + splits + enrichment
│   └── phai-cli/            Binary + report formatters + auto-update
├── schema/
│   ├── sqlite/              Local backend migrations
│   └── bigquery/            Production backend migrations (mirror semantics)
├── integrations/openclaw/   AI assistant skill + wrapper
├── docs/
│   ├── ARCHITECTURE.md      How the system is shaped
│   ├── ABSTRACTIONS.md      Domain models and contracts
│   ├── VISION.md            Product direction
│   ├── GETTING-STARTED.md   This file
│   └── adr/                 Architecture Decision Records
├── AGENTS.md                Agent guardrails
├── REPORTING_UX.md          Reporting voice & disambiguation rules
└── README.md                User-facing surface
```

---

## Troubleshooting

**`phai: command not found` after install.** `~/.local/bin` isn't in your `$PATH`. Either add it (`export PATH="$HOME/.local/bin:$PATH"` in your shell profile) or re-install with `--prefix=/usr/local`.

**Pluggy sync errors out with 401.** Confirm `PLUGGY_CLIENT_ID` / `PLUGGY_CLIENT_SECRET` are set in the same shell. Tokens auto-refresh; persistent 401 means the credentials are wrong or the item is in a re-consent state — check the Pluggy dashboard.

**BigQuery permission denied.** The service account needs both BigQuery Data Editor and BigQuery Job User (not just Data Viewer). The dataset must exist before `admin migrate`.

**Self-update loops or fails silently.** Set `FINANCE_OS_NO_AUTO_UPDATE=1` and run `phai self update` manually to see the diagnostic output. The update-state cache lives next to your DB; deleting `update-state.json` resets the 24h throttle.

**`cargo test` complains about parallel SQLite access.** E2E tests use `serial_test` for a reason. If you added a new test, mark it `#[serial]` or use a unique tempdir per test.
