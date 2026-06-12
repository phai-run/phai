//! `phai serve install` / `uninstall` — run the web app at login (macOS).
//!
//! Two artifacts, both owned by these commands (ADR-0028):
//!
//! 1. **launchd agent** `~/Library/LaunchAgents/run.phai.serve.plist` —
//!    launchd keeps `phai serve` alive (RunAtLoad + KeepAlive) and restarts
//!    it across crashes and reboots. The plist points at the absolute path of
//!    the current binary; the self-updater replaces that path atomically, so
//!    the agent always starts whatever version is installed. A custom
//!    `PHAI_CONFIG_DIR`/`FINANCE_OS_CONFIG_DIR` in effect at install time is
//!    captured into the plist so the daemon serves the same store the user
//!    installed from.
//! 2. **Launcher app** `~/Applications/Phai.app` — a minimal bundle whose
//!    executable opens the web app in the default browser. It shows up in
//!    Launchpad/Spotlight with the φ icon and can be pinned to the Dock;
//!    "installing phai" ends with something clickable.
//!
//! Both are plain files under `$HOME` — no sudo, no profiles, fully removed
//! by `uninstall`.

use anyhow::{bail, Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::Command;

const AGENT_LABEL: &str = "run.phai.serve";
const ICON_BYTES: &[u8] = include_bytes!("../assets/Phai.icns");

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Port the agent serves on (default 80 → http://phai.localhost).
    #[arg(long, default_value_t = 80)]
    pub port: u16,
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn agent_plist_path() -> Result<PathBuf> {
    Ok(home()?.join("Library/LaunchAgents/run.phai.serve.plist"))
}

fn app_bundle_path() -> Result<PathBuf> {
    Ok(home()?.join("Applications/Phai.app"))
}

fn log_path() -> Result<PathBuf> {
    Ok(home()?.join("Library/Logs/phai/serve.log"))
}

/// The URL the launcher opens. Port 80 gets the friendly host.
fn app_url(port: u16) -> String {
    if port == 80 {
        "http://phai.localhost/".to_string()
    } else {
        format!("http://localhost:{port}/")
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Config-dir env vars worth freezing into the agent, if set right now.
fn captured_env() -> Vec<(String, String)> {
    [
        "PHAI_CONFIG_DIR",
        "FINANCE_OS_CONFIG_DIR",
        "FINANCE_OS_DATA_DIR",
    ]
    .into_iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
    .collect()
}

/// launchd property list for the serve agent.
fn agent_plist(exe: &Path, port: u16, log: &Path, env: &[(String, String)]) -> String {
    let env_block = if env.is_empty() {
        String::new()
    } else {
        let entries: String = env
            .iter()
            .map(|(k, v)| {
                format!(
                    "\n      <key>{}</key>\n      <string>{}</string>",
                    xml_escape(k),
                    xml_escape(v)
                )
            })
            .collect();
        format!("\n    <key>EnvironmentVariables</key>\n    <dict>{entries}\n    </dict>")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
      <string>{exe}</string>
      <string>serve</string>
      <string>--port</string>
      <string>{port}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>{env_block}
  </dict>
</plist>
"#,
        label = AGENT_LABEL,
        exe = xml_escape(&exe.display().to_string()),
        port = port,
        log = xml_escape(&log.display().to_string()),
    )
}

/// Info.plist for the launcher bundle.
fn app_info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>CFBundleName</key>
    <string>Phai</string>
    <key>CFBundleDisplayName</key>
    <string>Phai</string>
    <key>CFBundleIdentifier</key>
    <string>run.phai.launcher</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>phai-open</string>
    <key>CFBundleIconFile</key>
    <string>Phai</string>
  </dict>
</plist>
"#,
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// The bundle "binary": a shell launcher that opens the app URL and exits.
fn app_launcher_script(url: &str) -> String {
    format!("#!/bin/sh\nexec /usr/bin/open \"{url}\"\n")
}

fn launchctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("launchctl")
        .args(args)
        .output()
        .context("failed to run launchctl")
}

fn gui_domain() -> String {
    // launchd user-session domain. `id -u` equivalent without a libc dep.
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "501".to_string());
    format!("gui/{uid}")
}

fn write_app_bundle(port: u16) -> Result<PathBuf> {
    let bundle = app_bundle_path()?;
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos)?;
    std::fs::create_dir_all(&resources)?;
    std::fs::write(contents.join("Info.plist"), app_info_plist())?;
    std::fs::write(resources.join("Phai.icns"), ICON_BYTES)?;
    let launcher = macos.join("phai-open");
    std::fs::write(&launcher, app_launcher_script(&app_url(port)))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755))?;
    }
    // Nudge LaunchServices/Finder to pick up the (possibly new) icon.
    let _ = Command::new("touch").arg(&bundle).output();
    Ok(bundle)
}

pub fn install(args: InstallArgs) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("phai serve install is macOS-only for now (launchd)");
    }
    let exe = std::env::current_exe().context("cannot resolve current executable")?;
    let plist_path = agent_plist_path()?;
    let log = log_path()?;
    if let Some(parent) = log.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let env = captured_env();
    std::fs::write(&plist_path, agent_plist(&exe, args.port, &log, &env))
        .with_context(|| format!("failed to write {}", plist_path.display()))?;

    // Idempotent (re)load: boot out any previous instance, then bootstrap.
    let domain = gui_domain();
    let _ = launchctl(&["bootout", &format!("{domain}/{AGENT_LABEL}")]);
    let boot = launchctl(&["bootstrap", &domain, &plist_path.display().to_string()])?;
    if !boot.status.success() {
        bail!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&boot.stderr).trim()
        );
    }

    let bundle = write_app_bundle(args.port)?;
    let url = app_url(args.port);

    println!("✅ launchd agent installed: {}", plist_path.display());
    println!("   serving {url} at login (logs: {})", log.display());
    println!("✅ launcher app: {}", bundle.display());
    println!("   it's in Launchpad/Spotlight as “Phai” — drag it to the Dock to pin it.");
    if !env.is_empty() {
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        println!("   captured into the agent: {}", keys.join(", "));
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("phai serve uninstall is macOS-only for now (launchd)");
    }
    let plist_path = agent_plist_path()?;
    let _ = launchctl(&["bootout", &format!("{}/{AGENT_LABEL}", gui_domain())]);
    let mut removed = Vec::new();
    if plist_path.exists() {
        std::fs::remove_file(&plist_path)
            .with_context(|| format!("failed to remove {}", plist_path.display()))?;
        removed.push(plist_path.display().to_string());
    }
    let bundle = app_bundle_path()?;
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle)
            .with_context(|| format!("failed to remove {}", bundle.display()))?;
        removed.push(bundle.display().to_string());
    }
    if removed.is_empty() {
        println!("nothing to remove — agent and launcher were not installed.");
    } else {
        for r in removed {
            println!("removed {r}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_plist_carries_exe_port_logs_and_env() {
        let plist = agent_plist(
            Path::new("/Users/alex/.local/bin/phai"),
            4321,
            Path::new("/Users/alex/Library/Logs/phai/serve.log"),
            &[("PHAI_CONFIG_DIR".into(), "/Users/alex/cfg".into())],
        );
        assert!(plist.contains("<string>run.phai.serve</string>"));
        assert!(plist.contains("<string>/Users/alex/.local/bin/phai</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("<string>4321</string>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("serve.log"));
        assert!(plist.contains("<key>PHAI_CONFIG_DIR</key>"));
        assert!(plist.contains("<string>/Users/alex/cfg</string>"));
    }

    #[test]
    fn agent_plist_omits_env_block_when_nothing_captured() {
        let plist = agent_plist(
            Path::new("/usr/local/bin/phai"),
            80,
            Path::new("/tmp/serve.log"),
            &[],
        );
        assert!(!plist.contains("EnvironmentVariables"));
    }

    #[test]
    fn plist_escapes_xml_sensitive_paths() {
        let plist = agent_plist(
            Path::new("/Users/a&b/phai"),
            80,
            Path::new("/tmp/serve.log"),
            &[],
        );
        assert!(plist.contains("/Users/a&amp;b/phai"));
        assert!(!plist.contains("a&b"));
    }

    #[test]
    fn launcher_opens_friendly_host_only_on_port_80() {
        assert_eq!(app_url(80), "http://phai.localhost/");
        assert_eq!(app_url(4317), "http://localhost:4317/");
        let script = app_launcher_script(&app_url(4317));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("exec /usr/bin/open \"http://localhost:4317/\""));
    }

    #[test]
    fn info_plist_names_the_bundle_and_icon() {
        let info = app_info_plist();
        assert!(info.contains("<string>run.phai.launcher</string>"));
        assert!(info.contains("<string>phai-open</string>"));
        assert!(info.contains("<key>CFBundleIconFile</key>"));
        assert!(info.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn icon_asset_is_a_real_icns() {
        // "icns" magic + non-trivial size — guards against a corrupted asset.
        assert_eq!(&ICON_BYTES[..4], b"icns");
        assert!(ICON_BYTES.len() > 50_000);
    }
}
