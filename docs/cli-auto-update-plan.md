# CLI Auto-Update Plan

## Summary

Implement a pull-based auto-update mechanism for the Finance OS CLI:

- The CLI checks public GitHub releases at startup before running real commands.
- Checks are throttled to at most once every 24 hours per installation.
- A successful update replaces the current executable and re-runs the original command once.
- Manual commands remain available for deterministic operation: `finance self check` and `finance self update`.

The first release target is macOS ARM (`aarch64-apple-darwin`) only.

## Runtime Behavior

- Add a CLI module responsible for release discovery, download, checksum validation, archive extraction, executable replacement, and command re-exec.
- Add a top-level `self` command group with:
  - `finance self check`: report current version, latest available version, target triple, and whether an update is available.
  - `finance self update`: perform an immediate update check and replacement when a newer compatible release exists.
- Run the auto-check before dispatching normal commands, but skip it for:
  - `--help` and `--version`.
  - Any `finance self ...` command.
  - `FINANCE_OS_NO_AUTO_UPDATE=1`.
  - Re-executed commands marked by an internal env sentinel, to avoid update loops.
  - Commands executed before the 24-hour throttle window expires.
- Store updater state under `FINANCE_OS_DATA_DIR/update-state.json`, including last check time, last seen version, and recent error text.
- On update failure, print a concise warning to stderr and continue with the current executable.

## Release Assets

- Use the existing production release channel created by Release Please.
- Publish these assets to each production GitHub Release:
  - `finance-cli-aarch64-apple-darwin.tar.gz`
  - `finance-cli-aarch64-apple-darwin.tar.gz.sha256`
- The tarball contains the executable at archive root as `finance-cli`.
- The updater validates the SHA-256 file before replacing the executable.
- Releases/assets are assumed public for this first version. If the repository remains private, switch to token-based authentication via environment variable before implementation.

## GitHub Actions Changes

- Extend `.github/workflows/release-please.yml` so the `release-please` step has an `id` and exposes path-prefixed outputs for `crates/finance-cli`.
- Add a release asset job gated by the package release output, for example when `crates/finance-cli--release_created` is true.
- Run the asset job on a macOS ARM runner.
- Build with `cargo build --release --package finance-cli`.
- Package `target/release/finance-cli`, generate a `.sha256`, and upload both files with `gh release upload --clobber`.

## Documentation Changes

- Update the README installation path to recommend a runtime layout compatible with the existing OpenClaw wrapper:

```text
~/.local/share/finance-os/runtime/bin/finance-cli
```

- Document `finance self check`, `finance self update`, `FINANCE_OS_NO_AUTO_UPDATE=1`, and the 24-hour auto-check throttle.

## Test Plan

- Unit tests for:
  - Version comparison.
  - Asset name selection for `aarch64-apple-darwin`.
  - Checksum parsing and mismatch handling.
  - Throttle behavior.
  - Auto-check skip rules.
- Mocked HTTP tests for:
  - Latest release available.
  - Already up-to-date release.
  - Missing target asset.
  - Invalid checksum.
  - Download/network failure.
- CLI tests for:
  - `finance self check` output.
  - `finance self update` no-op when current version is latest.
  - `finance --version` does not trigger updater state writes.
- Run before merge:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features`
  - `cargo test --workspace`

## Assumptions

- First implementation supports only `aarch64-apple-darwin`.
- No GitHub token or secret is embedded in the binary.
- No code signing is included in v1; SHA-256 validation is the minimum integrity check.
- Update failures must not block normal finance commands.
