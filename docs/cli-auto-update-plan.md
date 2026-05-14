# CLI Auto-Update Plan

## Summary

Implement a pull-based auto-update mechanism for the Finance OS CLI:

- The CLI checks public GitHub releases at startup before dispatching real commands.
- Checks are throttled to at most once every 24 hours per installation.
- A successful update replaces the current executable and re-execs the original command.
- Manual commands remain available: `finance self check` and `finance self update`.

First release target: macOS ARM (`aarch64-apple-darwin`) only.

## Scope

**In scope — what gets updated:**

- The `finance-cli` binary at `$RUNTIME_ROOT/bin/finance-cli` (path discovered via `std::env::current_exe()`).

**Out of scope — explicitly preserved or unmanaged:**

| Item | Location | Why out of scope |
|---|---|---|
| User config | `$FINANCE_OS_CONFIG_DIR` (default `~/.config/finance-os/`) | Separate path — the updater never touches it. |
| User data (SQLite db, state) | `$FINANCE_OS_DATA_DIR` (default `~/.local/share/finance-os/`) | Separate path. Schema migrations, if any, are run by the new binary on next launch — that is existing CLI behavior, not the updater's responsibility. |
| Agent skill files (`SKILL.md`, `FINANCE_OS.md`, `finance.sh` wrapper) | `~/skills/finance-os/` (separate repo) | Versioned and distributed independently of the CLI binary. The wrapper script resolves the runtime root and forwards args — it does not need to change when the binary is bumped, as long as CLI flag compatibility is preserved. |
| `update-state.json` | `$FINANCE_OS_DATA_DIR/update-state.json` | Written by the updater itself; persists across updates. |

**Skill-binary compatibility:** because `finance.sh` only invokes the binary via `exec`, breaking changes to CLI flags or subcommand layout will silently break the skill. Treat CLI surface stability as a release-time check (covered by integration tests in the skill repo, not here). If a release intentionally breaks the surface, the skill repo needs a coordinated update — flag this in the release PR.

## File / Module Structure

New files (all under `crates/finance-cli/src/`):

```
src/
  main.rs              # Add Commands::Self variant, dispatch, auto-check hook
  update.rs            # Release discovery, download, checksum, extract, replace, re-exec
  update_state.rs      # UpdateState struct + read/write to update-state.json
  self_cmd.rs          # SelfCommand (Check, Update) + run() handler
```

No new logic goes into the 3839-line `main.rs` beyond the dispatch match arm and the auto-check call site.

## Dependencies

Add to `crates/finance-cli/Cargo.toml` (`[dependencies]`):

```toml
reqwest = { workspace = true }
sha2 = { workspace = true }
tempfile = { workspace = true }
flate2 = "1.0"
tar = "0.4"
```

`reqwest`, `sha2`, and `tempfile` are already workspace dependencies. `flate2` and `tar` are new (archive extraction). Consider adding `flate2` and `tar` to workspace dependencies if `finance-core` will ever need them.

> Note: `flate2`'s current stable major is **1.x** (1.0.x line). There is no 2.0 release. Earlier drafts of this plan listed `"2.0"` — that was wrong.

## Clap Command Structure

`self` is a reserved keyword in Rust, so the enum variant must be renamed and the user-facing subcommand name remapped via the clap attribute.

```rust
// In Commands enum (main.rs)
#[command(name = "self")]
SelfCmd(SelfCommand)

// New file: self_cmd.rs
#[derive(Subcommand)]
enum SelfCommand {
    /// Report current version, latest available version, and whether an update exists
    Check,
    /// Download and install the latest release, then re-exec
    Update,
}
```

## Runtime Behavior

### Auto-Check Hook

Place the auto-check in `async fn main()` **after** `Cli::parse()` but **before** the `Commands` match dispatch:

```rust
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();   // --version / --help exit here via clap
    // ... load config ...

    // Auto-update check (conditional). Never propagates errors — auto_check
    // logs to stderr internally and returns ().
    if should_run_auto_check(&cli) {
        update::auto_check(&config).await;
    }

    match cli.command {
        Commands::SelfCmd(cmd) => self_cmd::run(cmd, &config).await,
        // ... existing arms ...
    }
}

fn should_run_auto_check(cli: &Cli) -> bool {
    if std::env::var_os("FINANCE_OS_NO_AUTO_UPDATE").is_some() { return false; }
    if std::env::var_os("FINANCE_OS_UPDATED").is_some()         { return false; }
    if matches!(cli.command, Commands::SelfCmd(_))              { return false; }
    true
}
```

`--version` and `--help` are handled by clap internally before we reach this code — no special skip logic needed for those flags.

**Hard timeout requirement:** `update::auto_check` must build a `reqwest::Client` with `.timeout(Duration::from_secs(2))` and `.connect_timeout(Duration::from_secs(1))`. The auto-check runs synchronously before every command — a slow GitHub or captive portal must not delay startup beyond a couple of seconds. Timeouts are treated as silent skips (logged at `debug!`, not `warn!`).

### Skip Conditions

The check is skipped when any of:

| Condition | Rationale |
|---|---|
| `FINANCE_OS_NO_AUTO_UPDATE=1` | Explicit user opt-out |
| Subcommand is `Commands::SelfCmd(_)` | `self check` / `self update` handle version discovery explicitly |
| Last check was < 24 hours ago (per `update-state.json`) | Throttle |
| `FINANCE_OS_UPDATED=<version>` is set | Process is the post-exec child; prevents update loops |
| HTTP request times out (1-2 s) | Captive portal / slow network — never block startup |

The env-var and command-variant checks happen in `should_run_auto_check` (cheap, no I/O). The 24h throttle check happens inside `auto_check` after reading `update-state.json`.

### Update State File

Path: `{FINANCE_OS_DATA_DIR}/update-state.json` (defaults to `~/.local/share/finance-os/update-state.json`)

Schema:

```json
{
  "last_check": "2026-05-13T10:30:00Z",
  "last_seen_version": "0.3.1",
  "last_error": null,
  "exe_path_hash": "a1b2c3..."
}
```

- `exe_path_hash`: SHA-256 of `std::env::current_exe()` (canonicalized). Used to invalidate the throttle when the binary moves between installations (e.g., user switched from a dev build to a release). Note: this hashes the **binary path**, not the data dir — the data dir is shared across installations and was the wrong key for this purpose.
- `last_error`: cleared on the next successful check. The CLI never bails on a non-null `last_error`; it is for diagnostics only.

**Writes must be atomic** — write to `update-state.json.tmp`, then `fs::rename` to the final path. A truncated JSON would break every subsequent run's throttle parse.

### macOS Self-Replacement

Unlike Linux's `ETXTBSY`, macOS **does** allow replacing a running executable via `rename(2)` — the kernel keeps the open inode alive for the running process, while the path now points to a new inode. This lets us avoid the copy-and-cleanup dance.

Strategy:

1. Resolve `current_exe = std::env::current_exe()?.canonicalize()?`.
2. Download tarball to a `tempfile::TempDir` placed **on the same filesystem** as `current_exe` (so `rename` is atomic — `tempfile::Builder::new().tempdir_in(parent_of_current_exe)`).
3. Validate SHA-256 of the tarball against the `.sha256` asset **before** unpacking.
4. Extract the tarball, guarding each entry against path traversal: reject any entry whose canonicalized path does not start with the tempdir.
5. `chmod 0755` the extracted binary.
6. `fs::rename(extracted_binary, &current_exe)` — atomic on the same filesystem; the running process keeps executing the old inode until it exits.
7. `unistd::execv(&current_exe, &original_argv)` (via the `nix` crate, or `std::os::unix::process::CommandExt::exec`) with the env sentinel `FINANCE_OS_UPDATED=<new_version>` injected. The new version is encoded into the sentinel so we can detect re-exec into the wrong binary (e.g., rename succeeded but somehow the old inode resolved on the new path — paranoid check).
8. On `execv` failure: print a warning to stderr (the process is still alive — the exec only replaces on success) and continue with the current binary; mark `last_error` in update state.

**Gatekeeper / quarantine note:** binaries downloaded over HTTP by `reqwest` do **not** receive the `com.apple.quarantine` xattr (that flag is set by GUI apps using `LSFileQuarantineEnabled`, not by libcurl-style downloads). So Gatekeeper will not block the re-exec. However, on first install via a browser or AirDrop, the user may have a quarantined binary; that is an install-time concern, not the updater's.

**Code signing:** the v1 binary is unsigned. SHA-256 of the tarball (downloaded from GitHub Releases over TLS) is the integrity boundary. If the GitHub repo is compromised, the updater is compromised — accept this risk explicitly. Code signing + notarization is a follow-up.

### Manual Commands

#### `finance self check`

Output:
```
Current version: 0.3.1
Latest version:  0.4.0
Target triple:   aarch64-apple-darwin
Update available: yes
Release URL:     https://github.com/.../releases/tag/v0.4.0
```

Exit code 0 regardless of update availability. Uses exit code only for actual errors (network failure, etc.).

If latest version ≤ current version:
```
Update available: no (already up to date)
```

#### `finance self update`

Same check flow, but downloads and replaces if a newer version exists. Reports outcome:
```
Already up to date (0.3.1).
```
or
```
Updated from 0.3.1 to 0.4.0. Restarting...
```

After successful update, re-execs the same way auto-check does.

## Release Assets

- Published to each production GitHub Release by the CI job.
- Asset naming:
  - `finance-cli-aarch64-apple-darwin.tar.gz`
  - `finance-cli-aarch64-apple-darwin.tar.gz.sha256`
- Tarball contains the executable at archive root as `finance-cli`.
- The updater validates the SHA-256 file before replacing the executable.
- No GitHub token embedded in the binary. The GitHub Releases API for public repositories does not require authentication for download. (The repo is being flipped to public — see Phase 6.)

**HTTP client requirements:**
- `User-Agent: finance-cli/<version>` — GitHub API returns 403 without a User-Agent.
- `Accept: application/vnd.github+json` for API calls; `Accept: application/octet-stream` for asset downloads.
- `timeout = 2s` and `connect_timeout = 1s` for the auto-check codepath. Manual `self update` may use a longer timeout (e.g., 30s) since the user is waiting.

**Tag prefix:** release-please produces tags like `finance-cli-v0.3.1`. The updater must strip both the component prefix (`finance-cli-`) and the leading `v` before semver comparison. Add a unit test for this specifically.

## GitHub Actions Changes

Extend `.github/workflows/release-please.yml`:

```yaml
name: Release Please

on:
  push:
    branches: [main]

permissions:
  contents: write
  issues: write
  pull-requests: write

jobs:
  release-please:
    runs-on: ubuntu-latest
    outputs:
      finance-cli-released: ${{ steps.release.outputs['crates/finance-cli--release_created'] }}
      finance-cli-tag: ${{ steps.release.outputs['crates/finance-cli--tag_name'] }}
    steps:
      - uses: googleapis/release-please-action@v4
        id: release
        with:
          config-file: release-please-config.json
          manifest-file: .release-please-manifest.json

  release-assets:
    needs: release-please
    if: needs.release-please.outputs.finance-cli-released == 'true'
    runs-on: macos-14  # ARM runner (pinned; macos-latest is ARM as of 2025+ but pin for stability)
    steps:
      - uses: actions/checkout@v4
        with:
          ref: ${{ needs.release-please.outputs.finance-cli-tag }}
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release --package finance-cli
      - name: Package and checksum
        working-directory: target/release
        run: |
          tar czf finance-cli-aarch64-apple-darwin.tar.gz finance-cli
          shasum -a 256 finance-cli-aarch64-apple-darwin.tar.gz \
            > finance-cli-aarch64-apple-darwin.tar.gz.sha256
      - name: Upload to release
        run: |
          gh release upload "${{ needs.release-please.outputs.finance-cli-tag }}" \
            target/release/finance-cli-aarch64-apple-darwin.tar.gz \
            target/release/finance-cli-aarch64-apple-darwin.tar.gz.sha256 \
            --clobber
        env:
          GH_TOKEN: ${{ github.token }}
```

Key changes from current workflow:
- Add `id: release` to the release-please step.
- Add `outputs` at the job level exposing release-please's per-component outputs. The exact output key format (`crates/finance-cli--release_created` etc.) depends on the `release-please-config.json` component path — **verify by inspecting the action's output in a dry run before merging**, because a wrong key silently makes the job skip with no error.
- Add a `release-assets` job gated on the release output, running on `macos-14` (ARM).
- Build, package, checksum (`shasum -a 256` — macOS has no `sha256sum` by default), and upload assets with `gh release upload --clobber`.

Versionless asset names (`finance-cli-aarch64-apple-darwin.tar.gz`) mean the "latest" release API always returns the current asset — the updater doesn't need to construct version-specific URLs.

## Implementation Phases

### Phase 1: Module Scaffold + State

1. Add dependencies to `finance-cli/Cargo.toml`.
2. Create `src/update_state.rs` — `UpdateState` struct with serde, `read(path)`, `write(&self, path)`, `should_check()` (24h throttle).
3. Create `src/self_cmd.rs` — `SelfCommand` enum, stub `run()`.
4. Create `src/update.rs` — stub module with `auto_check()`.
5. Add `Commands::Self(SelfCommand)` to `main.rs`, wire dispatch.
6. Add `mod update; mod update_state; mod self_cmd;` to `main.rs`.
7. Verify `cargo build` and `cargo test` pass.

### Phase 2: GitHub Release Discovery

1. Implement `update::http_client() -> reqwest::Client` — sets User-Agent, Accept, and timeouts (2s/1s for auto-check; a separate constructor with longer timeout for manual `self update`).
2. Implement `update::latest_release_tag(client) -> Result<String>` — calls `GET /repos/{owner}/{repo}/releases/latest` on the GitHub API.
3. Implement `update::parse_tag(tag) -> Result<SemVer>` — strips `finance-cli-` component prefix and the leading `v`.
4. Implement `update::compare_versions(current, latest) -> Ordering` using simple semver parsing (split on `.`, compare numerically). Pre-release suffixes (`-rc1`, `-beta`) sort as older than the same `MAJOR.MINOR.PATCH` — match standard semver precedence.
5. Wire `self_cmd::run(Check, config)` to fetch latest release and print comparison.
6. Unit tests for tag parsing (with prefix, without prefix, malformed) and version comparison (equal, newer patch/minor/major, malformed, pre-release).

### Phase 3: Download + Validate + Replace

1. Implement `update::download_asset(client, tag, asset_name) -> Result<Bytes>` — downloads the tarball.
2. Implement `update::download_checksum(client, tag) -> Result<String>` — downloads the `.sha256` file, parses the hex digest (handles both `<hex>  <filename>` and bare-hex forms).
3. Implement `update::extract_and_validate(tarball_bytes, expected_sha256, tempdir) -> Result<PathBuf>`:
   - Verify SHA-256 of the tarball bytes against `expected_sha256` **before** unpacking.
   - Unpack manually via `tar::Archive::entries()`, validating each entry's path resolves inside `tempdir` (zip-slip guard).
   - Return path to the extracted `finance-cli`.
4. Implement `update::replace_and_reexec(new_binary: &Path, new_version: &str)`:
   - `chmod 0755` the new binary.
   - `fs::rename` it over `current_exe()` (tempdir was created on same filesystem in step 1 to make this atomic).
   - `execv` with original argv and `FINANCE_OS_UPDATED=<new_version>` env injected.
5. Wire `self_cmd::run(Update, config)` to run the full pipeline.

### Phase 4: Auto-Check Integration

1. Implement `update::auto_check(config)` — signature returns `()` (never propagates). Reads state, checks throttle, calls release discovery, compares versions, runs update pipeline if newer available. Atomically writes `update-state.json` (tmp + rename).
2. Add the auto-check call site in `main()` after `Cli::parse()`, gated by `should_run_auto_check(&cli)`.
3. Implement `FINANCE_OS_NO_AUTO_UPDATE=1` and `FINANCE_OS_UPDATED=<version>` skip logic.
4. Ensure all errors in auto-check are caught — `warn!` for actionable failures (checksum mismatch, exec failure), `debug!` for expected ones (timeout, network down). Record in `last_error`. Never abort the real command.

### Phase 5: CI + Docs

1. Update `.github/workflows/release-please.yml` with asset job.
2. Update `README.md`:
   - Add `finance self check` and `finance self update` to the CLI commands table.
   - Add `FINANCE_OS_NO_AUTO_UPDATE=1` to the environment variables table.
   - Document the 24-hour auto-check throttle.
   - Document the runtime binary layout (`~/.local/share/finance-os/runtime/bin/finance-cli`).

### Phase 6: Public Release Preparation

The auto-update mechanism depends on the GitHub Releases API being reachable without authentication. The repo is flipped to public in this phase. **This phase must merge before the first release that ships with auto-update enabled** — otherwise the updater hits 404s or auth walls.

Preliminary audit at planning time: LICENSE, README.md, CONTRIBUTING.md, CHANGELOG.md already exist; `.gitignore` already covers `*.env`, service accounts, pluggy configs, finance data, and local overlays; no hits for personal paths or names in tracked files; 20 commits total (small surface to audit).

#### Pre-flip audit

1. **Secret scan (history, not just HEAD).** Run `gitleaks detect --source . --log-opts="--all"` and `trufflehog git file://.` over the full history. Resolve any hits — if a secret ever existed, rotate it and rewrite history with `git filter-repo` before flipping public. Treat a "clean" pass as a hard gate, not a nice-to-have.
2. **Dependency hygiene.** `cargo audit` (no advisories), `cargo deny check` (licenses + bans). Add both to CI as required checks if not already.
3. **AGENTS.md compliance pass.** AGENTS.md forbids personal counterparty names, account labels, statement fingerprints, etc. in shared source. Grep migrations, fixtures, tests, and rule SQL for anything resembling real-world entity names that may have slipped in pre-AGENTS.md commits.
4. **Public-surface review of `docs/`.** Read every file in `docs/` as if you were an outside reader — anything that names internal infra, hostnames, or assumes context the public won't have? Edit or move to a private location.
5. **CLI help / error text review.** `finance --help` and error messages will be the first thing users see. Skim for jargon, internal codenames, or stale references.

#### Public-facing files

6. **Add `SECURITY.md`** with a single contact for responsible disclosure (email or GitHub security advisories) and the supported-versions policy (initially: only the latest minor).
7. **Add `.github/ISSUE_TEMPLATE/bug_report.md`** and `feature_request.md`. Keep them short — long templates depress reporting.
8. **Add `.github/PULL_REQUEST_TEMPLATE.md`** with a checklist matching the existing pre-merge checks (`cargo fmt`, `clippy`, `test`).
9. **Update `README.md`** for an external audience:
   - One-paragraph "what this is" at the top (current README assumes context).
   - Install instructions (`curl | sh` once auto-update works, plus `cargo install` fallback).
   - Quickstart that runs against a synthetic fixture, not personal data.
   - Badges: build status, latest release, license.
10. **Update `CONTRIBUTING.md`** if it currently assumes the contributor has internal access (e.g., to private configs in `~/ford/`).

#### GitHub repo settings (UI, not code)

11. **Repo description and topics** on the GitHub page — short tagline, topics like `finance`, `rust`, `cli`, `pluggy`, `bigquery`.
12. **Default branch protection** on `main`: require PR, require status checks (`fmt`, `clippy`, `test`, `audit`).
13. **Discussions / Issues / Wiki toggles** — decide which to enable. Discussions on, Wiki off is a sensible default.
14. **Security tab**: enable Dependabot alerts, Dependabot security updates, secret scanning (free on public repos), and code scanning (CodeQL) via the default workflow.
15. **Social preview image** — optional but cheap polish.

#### Flip and verify

16. Settings → Danger Zone → **Change visibility → Public**. Confirm by typing the repo name.
17. Verify `https://api.github.com/repos/<owner>/<repo>/releases/latest` returns JSON without auth from a logged-out browser session.
18. Verify a prior release tarball is publicly downloadable via `curl`.
19. Cut a no-op patch release (`0.3.2` or similar) to exercise the now-public release flow end-to-end before the first auto-update-capable release.

#### Updater wiring

20. Hardcode the public `owner/repo` constants in `update.rs` (or pull from `env!("CARGO_PKG_REPOSITORY")` if `Cargo.toml` already has the repo URL — preferred, so the constant has a single source of truth).

## Test Plan

### Unit Tests (in `update.rs`)

| Test | What it verifies |
|------|-----------------|
| `version_comparison_equal` | `0.3.1 == 0.3.1` |
| `version_comparison_newer_patch` | `0.3.2 > 0.3.1` |
| `version_comparison_newer_minor` | `0.4.0 > 0.3.9` |
| `version_comparison_newer_major` | `1.0.0 > 0.9.9` |
| `version_comparison_malformed_current` | Malformed current version → skip update (no panic) |
| `version_comparison_malformed_latest` | Malformed latest version → skip update |
| `asset_name_for_target` | Returns correct asset name for `aarch64-apple-darwin` |
| `checksum_parse_valid` | Parses `abc123  finance-cli-....tar.gz` correctly |
| `checksum_mismatch` | Wrong checksum → error |
| `throttle_should_skip` | State with check < 24h ago → `should_check()` returns false |
| `throttle_should_check` | State with check > 24h ago → `should_check()` returns true |
| `throttle_no_state` | No state file → `should_check()` returns true |
| `skip_on_no_auto_update_env` | `FINANCE_OS_NO_AUTO_UPDATE=1` → auto_check returns early |
| `skip_on_updated_sentinel` | `FINANCE_OS_UPDATED=1` → auto_check returns early |

### Mocked HTTP Tests (with `wiremock` or `httptest`)

| Test | Scenario |
|------|----------|
| `latest_release_available` | Server returns tag > current |
| `already_up_to_date` | Server returns tag == current |
| `older_release_on_server` | Server returns tag < current → skip |
| `missing_target_asset` | Release exists but no `aarch64-apple-darwin` asset |
| `invalid_checksum` | Downloaded tarball doesn't match `.sha256` |
| `download_network_failure` | Server returns 5xx |
| `rate_limited` | Server returns 429 → graceful degradation |

### CLI Integration Tests (`tests/e2e.rs`)

| Test | What it verifies |
|------|-----------------|
| `self_check_output_format` | `finance self check` prints expected fields |
| `self_update_noop_when_current` | `finance self update` with current == latest prints "already up to date" |
| `version_flag_no_update` | `finance --version` does not create `update-state.json` |
| `tag_prefix_stripped` | `finance-cli-v0.4.0` → parses to `0.4.0` |
| `reexec_passes_argv` | Stub-binary test: replacement binary receives original argv and `FINANCE_OS_UPDATED` env |
| `atomic_state_write` | Killing the writer mid-write leaves the previous `update-state.json` intact (no truncation) |
| `tar_path_traversal_rejected` | Tarball containing `../evil` entry fails extraction |

### Pre-Merge Checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
cargo test --workspace
```

## Error Handling

All update errors (network, checksum, extraction, exec) must:
- Print a concise warning to stderr (not stdout).
- Never propagate up to abort the real command.
- Be recorded in `update_state.last_error` for debugging.
- Use `log::warn!` / `log::debug!` or `eprintln!` — not `anyhow::bail!`.

`last_error` lifecycle:
- Set on any failure path inside `auto_check` or `self update`.
- Cleared on the **next** successful check (even a no-op "already up to date" counts).
- Never read by the CLI to gate behavior — diagnostic only.

State writes:
- Always go through a `write_state_atomic(path, &state)` helper: serialize to `path.tmp`, `fs::sync_all`, `fs::rename` to `path`.
- A failed write must not panic — log and continue.

## Assumptions

- **Scope:** the updater replaces only the `finance-cli` binary. User config (`$FINANCE_OS_CONFIG_DIR`) and data (`$FINANCE_OS_DATA_DIR`) are separate paths and are never touched. Agent skill files in `~/skills/finance-os/` are versioned independently and out of scope.
- **CLI surface stability:** the skill wrapper `finance.sh` invokes the binary via `exec`, forwarding arguments. Breaking changes to subcommand layout or flag names will break the skill silently. Treat CLI-surface compatibility as a release-time concern; flag intentional breaks in the release PR so the skill repo can be updated in lockstep.
- **Target triple:** first implementation supports only `aarch64-apple-darwin`. Target triple is hardcoded. Multi-target support is deferred to a follow-up.
- **GitHub auth:** no token or secret embedded. The repo will be flipped to public before the first auto-update-capable release (see Phase 6); the GitHub API for public releases does not require authentication for downloads.
- **Integrity:** no code signing in v1. SHA-256 of the tarball validated against the published `.sha256` is the integrity boundary. A GitHub repo compromise compromises the updater — accept this risk explicitly.
- **Availability:** update failures must not block normal finance commands. The auto-check has a hard 2-second timeout.
- **Binary path:** discovered at runtime via `std::env::current_exe()`. The plan does **not** assume a fixed install location (Brew, cargo install, curl-sh, and the OpenClaw `$RUNTIME_ROOT/bin/finance-cli` layout all work as long as `current_exe()` returns the actual binary path).
- **Atomic replace:** on macOS, `rename(2)` of the new binary over the running executable is atomic and safe — the kernel preserves the running inode. Tempdir must be on the same filesystem as the target.
- **Re-exec:** `execv` replaces the process image in-place — no second process, no background cleanup script.
