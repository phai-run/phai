use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::update_state::{compute_exe_path_hash, state_file_path, UpdateState};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const REPO_OWNER: &str = "feliperun";
const REPO_NAME: &str = "finance-os";
pub const REPO_URL: &str = "https://github.com/feliperun/finance-os";

// Each compiled binary is locked to one platform. We expose the triple and
// matching asset names per architecture so the updater always asks GitHub
// Releases for the artifact that matches the running binary.
//
// Linux/Windows builds are not officially released, but CI runs on Ubuntu —
// we provide a sentinel fallback so the crate compiles there. The auto-update
// path is gated on macOS at install time (install.sh / release matrix), so
// the fallback is never exercised at runtime.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const TARGET_TRIPLE: &str = "aarch64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const TARGET_TRIPLE: &str = "x86_64-apple-darwin";
#[cfg(not(target_os = "macos"))]
pub const TARGET_TRIPLE: &str = "unsupported";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const ASSET_NAME: &str = "finance-cli-aarch64-apple-darwin.tar.gz";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const ASSET_NAME: &str = "finance-cli-x86_64-apple-darwin.tar.gz";
#[cfg(not(target_os = "macos"))]
const ASSET_NAME: &str = "finance-cli-unsupported.tar.gz";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CHECKSUM_ASSET_NAME: &str = "finance-cli-aarch64-apple-darwin.tar.gz.sha256";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const CHECKSUM_ASSET_NAME: &str = "finance-cli-x86_64-apple-darwin.tar.gz.sha256";
#[cfg(not(target_os = "macos"))]
const CHECKSUM_ASSET_NAME: &str = "finance-cli-unsupported.tar.gz.sha256";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const SIGNATURE_ASSET_NAME: &str = "finance-cli-aarch64-apple-darwin.tar.gz.minisig";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const SIGNATURE_ASSET_NAME: &str = "finance-cli-x86_64-apple-darwin.tar.gz.minisig";
#[cfg(not(target_os = "macos"))]
const SIGNATURE_ASSET_NAME: &str = "finance-cli-unsupported.tar.gz.minisig";

const BINARY_NAME: &str = "fin";

/// Minisign public key matched against release tarball signatures. See
/// [ADR-0017](../../docs/adr/0017-release-signature-verification.md) for the
/// rotation plan and the rationale for verifying signatures on top of SHA-256.
///
/// Placeholder until the release CI starts signing. Verification stays
/// best-effort while this is empty (or `REQUIRE_SIGNATURE` is `false`); once
/// CI signs reliably for one cycle, flip `REQUIRE_SIGNATURE = true`.
const SIGNING_PUBLIC_KEY: &str = "";

/// Once CI ships `.minisig` sidecars consistently, flip this to `true` so a
/// missing signature is rejected instead of warning. Until then the updater
/// stays in transition mode (warns once per run if the sidecar is absent).
const REQUIRE_SIGNATURE: bool = false;

/// Release Please produces tags like `v0.3.1` (include-component-in-tag: false,
/// include-v-in-tag: true). The updater strips the leading `v`.
/// This is also robust to component-prefixed tags like
/// `finance-cli-v0.3.1` in case the config changes.
pub fn strip_tag_prefix(tag: &str) -> &str {
    tag.trim_start_matches("finance-cli-")
        .trim_start_matches('v')
        .trim_start_matches('V')
}

// ---------------------------------------------------------------------------
// SemVer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

pub(crate) fn parse_version(s: &str) -> Result<SemVer> {
    let s = s.trim();
    let (base, pre) = match s.split_once('-') {
        Some((b, p)) => (b, Some(p.to_string())),
        None => (s, None),
    };
    let parts: Vec<&str> = base.split('.').collect();
    if parts.len() != 3 {
        bail!("version must be MAJOR.MINOR.PATCH, got: {s}");
    }
    let major = parts[0]
        .parse::<u64>()
        .with_context(|| format!("invalid major version: {}", parts[0]))?;
    let minor = parts[1]
        .parse::<u64>()
        .with_context(|| format!("invalid minor version: {}", parts[1]))?;
    let patch = parts[2]
        .parse::<u64>()
        .with_context(|| format!("invalid patch version: {}", parts[2]))?;
    Ok(SemVer {
        major,
        minor,
        patch,
        pre,
    })
}

pub(crate) fn is_newer(latest: &SemVer, current: &SemVer) -> bool {
    if latest.major != current.major {
        return latest.major > current.major;
    }
    if latest.minor != current.minor {
        return latest.minor > current.minor;
    }
    if latest.patch != current.patch {
        return latest.patch > current.patch;
    }
    // Same MAJOR.MINOR.PATCH: a version without pre-release is newer than one with
    match (&latest.pre, &current.pre) {
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (Some(lp), Some(cp)) => lp > cp,
        (None, None) => false, // equal
    }
}

// ---------------------------------------------------------------------------
// GitHub API types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubRelease {
    pub(crate) tag_name: String,
    #[allow(dead_code)]
    pub(crate) html_url: String,
    pub(crate) assets: Vec<GitHubAsset>,
    /// Release notes body (Markdown). Set by release-please when the release
    /// is created. May be empty for manually-cut releases.
    #[serde(default)]
    pub(crate) body: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

pub(crate) fn http_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("finance-cli/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(1))
        .build()
        .expect("reqwest Client::build should not fail with valid config")
}

// ---------------------------------------------------------------------------
// GitHub API
// ---------------------------------------------------------------------------

pub(crate) async fn get_latest_release(client: &reqwest::Client) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("failed to fetch latest release")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("GitHub API returned {status}: {body}");
    }

    resp.json::<GitHubRelease>()
        .await
        .context("failed to parse release JSON")
}

fn find_asset<'a>(release: &'a GitHubRelease, name: &str) -> Result<&'a GitHubAsset> {
    release
        .assets
        .iter()
        .find(|a| a.name == name)
        .with_context(|| format!("asset '{name}' not found in release {}", release.tag_name))
}

// ---------------------------------------------------------------------------
// Download + checksum
// ---------------------------------------------------------------------------

async fn download_tarball(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .context("failed to download tarball")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("download returned {status}: {body}");
    }

    resp.bytes()
        .await
        .context("failed to read tarball body")
        .map(|b| b.to_vec())
}

async fn download_checksum(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .context("failed to download checksum")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("checksum download returned {status}: {body}");
    }

    resp.text()
        .await
        .context("failed to read checksum body")
        .map(|s| s.trim().to_string())
}

fn parse_sha256(checksum_file: &str) -> Result<String> {
    // Format: "<hex>  <filename>" or bare "<hex>"
    let hex = checksum_file
        .split_whitespace()
        .next()
        .context("empty checksum file")?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid sha256 hex: {hex}");
    }
    Ok(hex.to_lowercase())
}

// ---------------------------------------------------------------------------
// Extract + validate
// ---------------------------------------------------------------------------

fn validate_tarball_sha256(data: &[u8], expected_hex: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_hex {
        bail!("checksum mismatch: expected {expected_hex}, got {actual}");
    }
    Ok(())
}

/// Verify a minisign signature over `tarball` against the embedded
/// `SIGNING_PUBLIC_KEY`. See ADR-0017.
///
/// Returns:
/// - `Ok(())` if the signature parses and verifies.
/// - `Err(_)` if the signature is present but invalid — callers MUST treat
///   this as a hard failure regardless of `REQUIRE_SIGNATURE`.
fn verify_minisign_signature(tarball: &[u8], signature_text: &str) -> Result<()> {
    let public_key = minisign_verify::PublicKey::decode(SIGNING_PUBLIC_KEY)
        .context("embedded signing public key failed to parse")?;
    let signature = minisign_verify::Signature::decode(signature_text)
        .context("release signature failed to parse")?;
    public_key
        .verify(tarball, &signature, false)
        .context("release signature did not verify against the embedded public key")
}

fn extract_binary(tarball: &[u8], dest_dir: &Path) -> Result<PathBuf> {
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(tarball));
    let mut archive = tar::Archive::new(decoder);

    // We only expect one entry: `finance-cli`
    let mut binary_path: Option<PathBuf> = None;

    for entry in archive
        .entries()
        .context("failed to iterate tarball entries")?
    {
        let mut entry = entry.context("failed to read tarball entry")?;

        // Path traversal guard: reject entries with `..`, absolute paths, or
        // root prefixes. The file doesn't exist yet, so canonicalize-based
        // checks don't work — inspect the path components instead.
        let entry_path = entry
            .path()
            .context("failed to read entry path")?
            .to_path_buf();
        for component in entry_path.components() {
            match component {
                std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_) => {
                    bail!(
                        "path traversal rejected: {} resolves outside temp dir",
                        entry_path.display()
                    );
                }
                _ => {}
            }
        }
        let full_path = dest_dir.join(&entry_path);

        entry
            .unpack_in(dest_dir)
            .context("failed to unpack tarball entry")?;

        if entry_path == Path::new(BINARY_NAME) {
            binary_path = Some(full_path);
        }
    }

    binary_path.context("executable 'fin' not found in tarball")
}

// ---------------------------------------------------------------------------
// Replace + re-exec
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub(crate) fn exec_new_binary(new_version: &str) {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("finance-cli"));
    let args: Vec<String> = std::env::args().collect();
    let err = std::process::Command::new(&exe)
        .args(&args[1..]) // skip argv[0]
        .env("FINANCE_OS_UPDATED", new_version)
        .exec();
    // exec replaces the process image on success; reaching here means failure
    eprintln!("Warning: failed to re-exec after update: {err}");
}

#[cfg(not(unix))]
fn exec_new_binary(_new_version: &str) {
    eprintln!("Warning: process re-exec is not supported on this platform.");
    eprintln!("Please restart finance-cli manually.");
}

// ---------------------------------------------------------------------------
// Download-and-replace pipeline
// ---------------------------------------------------------------------------

pub(crate) async fn download_and_replace(
    _api_client: &reqwest::Client,
    release: &GitHubRelease,
) -> Result<()> {
    let tarball_asset = find_asset(release, ASSET_NAME)?;
    let checksum_asset = find_asset(release, CHECKSUM_ASSET_NAME)?;
    // Signature sidecar is optional during the transition phase described in
    // ADR-0017. If the release ships one, we verify it; if it doesn't, we
    // either warn (default) or fail (when REQUIRE_SIGNATURE is flipped on).
    let signature_asset = find_asset(release, SIGNATURE_ASSET_NAME).ok();

    // Use a dedicated client for the download with a generous timeout. The
    // API-check client passed in by auto_check has a 2s timeout (tight to
    // avoid blocking startup), which is too short for a multi-megabyte
    // tarball on average networks.
    let download_client = http_client(30);

    // Download tarball + checksum first (always required).
    let (tarball, checksum_text) = tokio::try_join!(
        download_tarball(&download_client, &tarball_asset.browser_download_url),
        download_checksum(&download_client, &checksum_asset.browser_download_url),
    )?;

    let expected_sha256 = parse_sha256(&checksum_text)?;

    // Validate tarball checksum before any signature work or unpacking.
    validate_tarball_sha256(&tarball, &expected_sha256)?;

    // Then verify the minisign signature (authenticity gate). See ADR-0017.
    let skip_sig = std::env::var("FINANCE_OS_SKIP_SIG_VERIFY").ok().as_deref() == Some("1");
    if skip_sig {
        eprintln!(
            "Warning: signature verification skipped via FINANCE_OS_SKIP_SIG_VERIFY=1 \
             — only use this for break-glass scenarios."
        );
    } else {
        match signature_asset {
            Some(asset) => {
                let sig_text =
                    download_checksum(&download_client, &asset.browser_download_url).await?;
                verify_minisign_signature(&tarball, &sig_text)?;
            }
            None if REQUIRE_SIGNATURE => {
                bail!(
                    "release {} does not ship a {} sidecar; refusing to update without a verifiable signature",
                    release.tag_name,
                    SIGNATURE_ASSET_NAME
                );
            }
            None => {
                eprintln!(
                    "Warning: release {} ships no {} sidecar — falling back to SHA-256-only verification (ADR-0017 transition mode).",
                    release.tag_name, SIGNATURE_ASSET_NAME
                );
            }
        }
    }

    // Create tempdir next to the current executable (same filesystem for atomic rename).
    // Canonicalize to resolve symlinks — otherwise the tempdir may land on a
    // different filesystem from the real binary and the rename would fail.
    let current_exe = std::env::current_exe()
        .context("failed to resolve current exe")?
        .canonicalize()
        .context("failed to canonicalize current exe path")?;
    let exe_parent = current_exe
        .parent()
        .context("current exe has no parent directory")?;
    let tempdir = tempfile::Builder::new()
        .prefix("finance-cli-update-")
        .tempdir_in(exe_parent)
        .context("failed to create tempdir for update")?;

    // Extract and validate path traversal
    let extracted = extract_binary(&tarball, tempdir.path())?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&extracted)
            .context("failed to read extracted binary metadata")?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&extracted, perms)
            .context("failed to set executable permissions")?;
    }

    // Atomic rename: on macOS, the running process keeps the old inode alive.
    std::fs::rename(&extracted, &current_exe).context("failed to replace current executable")?;

    // Binary was moved out of tempdir; the directory is now empty and can be cleaned up.
    drop(tempdir);

    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-check
// ---------------------------------------------------------------------------

/// Run the auto-update check. Never propagates errors — logs to stderr and
/// continues silently on failure.
pub async fn auto_check(data_dir: &Path) {
    let result = auto_check_inner(data_dir).await;
    if let Err(ref e) = result {
        eprintln!("Warning: auto-update check failed: {e}");
    }
}

/// Unconditional update check — bypasses the rate-limit state gate.
/// Used before critical long-running commands (e.g. `sync pluggy`) to ensure
/// we're always running the latest version. Never propagates errors.
pub async fn force_check(data_dir: &Path) {
    let result = force_check_inner(data_dir).await;
    if let Err(ref e) = result {
        eprintln!("Warning: verificação de atualização falhou: {e}");
    }
}

async fn force_check_inner(data_dir: &Path) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let current = match parse_version(current_version) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    let client = http_client(5);
    let release = match get_latest_release(&client).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: não foi possível verificar atualizações: {e}");
            return Ok(());
        }
    };

    let latest_str = strip_tag_prefix(&release.tag_name);
    let latest = match parse_version(latest_str) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    // Advance the last_check timestamp so the background auto-check skips
    // the next invocation (we just checked).
    let state_path = state_file_path(data_dir);
    let exe_hash = compute_exe_path_hash().unwrap_or_default();
    let mut state = UpdateState::read(&state_path);
    state.mark_checked(latest_str, &exe_hash);
    let _ = state.write_atomic(&state_path);

    if !is_newer(&latest, &current) {
        return Ok(());
    }

    eprintln!(
        "finance-cli: atualização disponível: {current_version} → {latest_str}. \
         Atualizando antes de sincronizar..."
    );
    match download_and_replace(&client, &release).await {
        Ok(()) => {
            exec_new_binary(latest_str);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn auto_check_inner(data_dir: &Path) -> Result<()> {
    let state_path = state_file_path(data_dir);
    let exe_hash = compute_exe_path_hash()?;
    let mut state = UpdateState::read(&state_path);

    if !state.should_check(&exe_hash) {
        return Ok(());
    }

    let current_version = env!("CARGO_PKG_VERSION");
    let current = match parse_version(current_version) {
        Ok(v) => v,
        Err(_) => {
            state.mark_error(&format!("malformed current version: {current_version}"));
            let _ = state.write_atomic(&state_path);
            return Ok(());
        }
    };

    let client = http_client(2); // auto-check: tight timeouts

    let release = match get_latest_release(&client).await {
        Ok(r) => r,
        Err(e) => {
            // Always advance last_check so we don't hammer the network on every
            // invocation when a transient error occurs (Homebrew/rustup behavior).
            let desc = if let Some(re) = e.downcast_ref::<reqwest::Error>() {
                if re.is_timeout() || re.is_connect() || re.is_request() {
                    format!("transient network error: {re}")
                } else {
                    format!("release fetch: {e}")
                }
            } else {
                format!("release fetch: {e}")
            };
            state.mark_error(&desc);
            let _ = state.write_atomic(&state_path);
            return Ok(());
        }
    };

    let latest_str = strip_tag_prefix(&release.tag_name);
    let latest = match parse_version(latest_str) {
        Ok(v) => v,
        Err(e) => {
            state.mark_error(&format!("parse latest version '{latest_str}': {e}"));
            let _ = state.write_atomic(&state_path);
            return Ok(());
        }
    };

    state.mark_checked(latest_str, &exe_hash);
    let _ = state.write_atomic(&state_path);

    if !is_newer(&latest, &current) {
        return Ok(());
    }

    // Update available — try to download and replace
    eprintln!("finance-cli: update available: {current_version} → {latest_str}. Downloading...");

    match download_and_replace(&client, &release).await {
        Ok(()) => {
            exec_new_binary(latest_str);
            // exec failed — update state
            let mut state = UpdateState::read(&state_path);
            state.mark_error("exec after update failed");
            let _ = state.write_atomic(&state_path);
            Ok(())
        }
        Err(e) => {
            let mut state = UpdateState::read(&state_path);
            state.mark_error(&format!("download/replace: {e}"));
            let _ = state.write_atomic(&state_path);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Post-update release notes
// ---------------------------------------------------------------------------

/// Fetch the release notes (CHANGELOG body) for `version` from GitHub and
/// print them to stderr. Called once by the new binary right after a
/// successful self-update, so the user (and any watching agent) sees what
/// changed in the version they just got bumped to.
///
/// English-only — release-please generates the body from Conventional
/// Commits, which by convention are written in English. Failures
/// (network, rate-limit, missing release) are silent: the upgrade
/// already succeeded, missing release notes is not worth a warning.
pub async fn print_release_notes(version: &str) {
    let client = http_client(5);
    let url =
        format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/tags/v{version}");

    let resp = match client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return,
    };

    if !resp.status().is_success() {
        return;
    }

    let release: GitHubRelease = match resp.json().await {
        Ok(r) => r,
        Err(_) => return,
    };

    let body = match release.body.as_deref() {
        Some(b) if !b.trim().is_empty() => b.trim(),
        _ => return,
    };

    let release_url = format!("{REPO_URL}/releases/tag/v{version}");
    eprintln!();
    eprintln!("🎉 finance-cli updated to v{version}");
    eprintln!();
    eprintln!("{}", indent_lines(body, "  "));
    eprintln!();
    eprintln!("  Full release notes: {release_url}");
    eprintln!();
}

/// Prefix every line of `s` with `prefix`. Used to gently indent the
/// Markdown release-notes body so it's visually grouped under the
/// "Updated to ..." banner.
fn indent_lines(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a gzip-compressed tar archive in memory with the given entries.
    /// Each entry is `(path_in_tar, file_contents)`.
    ///
    /// Uses `append_data` for normal paths and a raw-bytes path for entries
    /// that contain `..` (which the tar crate's safe API rejects).
    fn build_tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Vec::new();
        let enc = flate2::write::GzEncoder::new(buf, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            // Write path bytes directly into the GNU long-name field so we can
            // embed `..` without the safe API rejecting it.
            header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            builder
                .append(&header, std::io::Cursor::new(data))
                .expect("append should not fail");
        }
        let enc = builder.into_inner().expect("into_inner should not fail");
        enc.finish().expect("gz finish should not fail")
    }

    fn sha256_hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    }

    // -----------------------------------------------------------------------
    // strip_tag_prefix
    // -----------------------------------------------------------------------

    #[test]
    fn strip_tag_prefix_lowercase_v() {
        assert_eq!(strip_tag_prefix("v0.3.1"), "0.3.1");
    }

    #[test]
    fn strip_tag_prefix_uppercase_v() {
        assert_eq!(strip_tag_prefix("V0.3.1"), "0.3.1");
    }

    #[test]
    fn strip_tag_prefix_component_prefix() {
        assert_eq!(strip_tag_prefix("finance-cli-v0.3.1"), "0.3.1");
    }

    #[test]
    fn strip_tag_prefix_no_prefix() {
        assert_eq!(strip_tag_prefix("0.3.1"), "0.3.1");
    }

    #[test]
    fn strip_tag_prefix_empty() {
        assert_eq!(strip_tag_prefix(""), "");
    }

    #[test]
    fn strip_tag_prefix_component_only_no_v() {
        // e.g. "finance-cli-0.3.1" — strips the component prefix, no v to strip
        assert_eq!(strip_tag_prefix("finance-cli-0.3.1"), "0.3.1");
    }

    // -----------------------------------------------------------------------
    // parse_version
    // -----------------------------------------------------------------------

    #[test]
    fn parse_version_valid() {
        let v = parse_version("0.3.1").unwrap();
        assert_eq!(v.major, 0);
        assert_eq!(v.minor, 3);
        assert_eq!(v.patch, 1);
        assert!(v.pre.is_none());
    }

    #[test]
    fn parse_version_pre_release() {
        let v = parse_version("0.3.1-rc1").unwrap();
        assert_eq!(v.patch, 1);
        assert_eq!(v.pre.as_deref(), Some("rc1"));
    }

    #[test]
    fn parse_version_malformed_alpha() {
        assert!(parse_version("abc").is_err());
    }

    #[test]
    fn parse_version_only_two_parts() {
        assert!(parse_version("0.3").is_err());
    }

    #[test]
    fn parse_version_four_parts() {
        assert!(parse_version("0.3.1.4").is_err());
    }

    #[test]
    fn parse_version_empty_string() {
        assert!(parse_version("").is_err());
    }

    #[test]
    fn parse_version_whitespace_trimmed() {
        // Leading/trailing whitespace should be tolerated
        let v = parse_version("  1.2.3  ").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    // -----------------------------------------------------------------------
    // is_newer
    // -----------------------------------------------------------------------

    fn ver(s: &str) -> SemVer {
        parse_version(s).unwrap()
    }

    #[test]
    fn is_newer_equal_versions() {
        assert!(!is_newer(&ver("1.0.0"), &ver("1.0.0")));
    }

    #[test]
    fn is_newer_newer_patch() {
        assert!(is_newer(&ver("1.0.2"), &ver("1.0.1")));
    }

    #[test]
    fn is_newer_older_patch() {
        assert!(!is_newer(&ver("1.0.0"), &ver("1.0.1")));
    }

    #[test]
    fn is_newer_newer_minor() {
        assert!(is_newer(&ver("1.1.0"), &ver("1.0.9")));
    }

    #[test]
    fn is_newer_newer_major() {
        assert!(is_newer(&ver("2.0.0"), &ver("1.9.9")));
    }

    #[test]
    fn is_newer_release_beats_pre_release() {
        // 1.0.0 is newer than 1.0.0-rc1
        assert!(is_newer(&ver("1.0.0"), &ver("1.0.0-rc1")));
    }

    #[test]
    fn is_newer_pre_release_does_not_beat_release() {
        assert!(!is_newer(&ver("1.0.0-rc1"), &ver("1.0.0")));
    }

    #[test]
    fn is_newer_later_pre_release_beats_earlier() {
        // rc2 > rc1 lexicographically
        assert!(is_newer(&ver("1.0.0-rc2"), &ver("1.0.0-rc1")));
    }

    // -----------------------------------------------------------------------
    // parse_sha256
    // -----------------------------------------------------------------------

    #[test]
    fn parse_sha256_valid_with_filename() {
        let hex = "a".repeat(64);
        let input = format!("{}  somefile.tar.gz", hex);
        let result = parse_sha256(&input).unwrap();
        assert_eq!(result, hex);
    }

    #[test]
    fn parse_sha256_bare_hex() {
        let hex = "b".repeat(64);
        let result = parse_sha256(&hex).unwrap();
        assert_eq!(result, hex);
    }

    #[test]
    fn parse_sha256_too_short() {
        assert!(parse_sha256("abc123").is_err());
    }

    #[test]
    fn parse_sha256_non_hex_chars() {
        let bad = "z".repeat(64);
        assert!(parse_sha256(&bad).is_err());
    }

    #[test]
    fn parse_sha256_empty() {
        assert!(parse_sha256("").is_err());
    }

    #[test]
    fn parse_sha256_uppercase_normalized() {
        let upper = "A".repeat(64);
        let result = parse_sha256(&upper).unwrap();
        assert_eq!(result, "a".repeat(64));
    }

    // -----------------------------------------------------------------------
    // validate_tarball_sha256
    // -----------------------------------------------------------------------

    #[test]
    fn validate_sha256_matching() {
        let data = b"hello world";
        let hex = sha256_hex(data);
        validate_tarball_sha256(data, &hex).unwrap();
    }

    #[test]
    fn validate_sha256_mismatch() {
        let data = b"hello world";
        let wrong = "0".repeat(64);
        assert!(validate_tarball_sha256(data, &wrong).is_err());
    }

    // -----------------------------------------------------------------------
    // extract_binary — path traversal
    // -----------------------------------------------------------------------

    #[test]
    fn extract_binary_rejects_path_traversal() {
        let tempdir = tempfile::tempdir().unwrap();
        // Build a tarball with a traversal path
        let tarball = build_tarball(&[("../evil", b"evil content")]);
        let result = extract_binary(&tarball, tempdir.path());
        assert!(result.is_err(), "expected path traversal to be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("path traversal") || msg.contains("outside"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn extract_binary_happy_path() {
        let tempdir = tempfile::tempdir().unwrap();
        // Canonicalize so the path traversal guard uses the same resolved prefix
        // (on macOS, tempdir may sit under /var/... which symlinks to /private/var/...).
        let dest = tempdir.path().canonicalize().unwrap();
        let binary_data = b"fake binary content";
        let tarball = build_tarball(&[("fin", binary_data)]);
        let extracted = extract_binary(&tarball, &dest).unwrap();
        assert!(extracted.exists(), "extracted binary should exist");
        let contents = std::fs::read(&extracted).unwrap();
        assert_eq!(contents, binary_data);
    }

    // -----------------------------------------------------------------------
    // indent_lines
    // -----------------------------------------------------------------------

    #[test]
    fn indent_lines_prefixes_each_line() {
        let input = "line one\nline two\nline three";
        assert_eq!(
            indent_lines(input, "  "),
            "  line one\n  line two\n  line three"
        );
    }

    #[test]
    fn indent_lines_preserves_empty_lines() {
        let input = "a\n\nb";
        assert_eq!(indent_lines(input, ">"), ">a\n>\n>b");
    }

    #[test]
    fn indent_lines_handles_single_line() {
        assert_eq!(indent_lines("just one", "..."), "...just one");
    }

    #[test]
    fn indent_lines_handles_empty_string() {
        assert_eq!(indent_lines("", "  "), "");
    }
}
