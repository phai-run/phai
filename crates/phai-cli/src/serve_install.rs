//! `phai serve install` / `uninstall` — run the web app at login (macOS).
//!
//! Two flavors:
//!
//! - **User agent** (default, no `--system`) — a launchd *agent* in the
//!   `gui/<uid>` domain. No sudo, no profiles, fully removed by `uninstall`.
//!   A user agent cannot bind privileged ports (< 1024); on port 80 it
//!   crash-loops with `Permission denied`.
//! - **System daemon** (`--system`) — a launchd *daemon* at
//!   `/Library/LaunchDaemons/run.phai.serve.plist`, owned `root:wheel`, loaded
//!   into the `system` domain. This is the supported way to bind port 80
//!   (`http://phai.localhost/`). The privileged steps run through a *single*
//!   native macOS admin-auth prompt (`osascript … with administrator
//!   privileges`) rather than a hand-typed `sudo` sequence — see ADR-0029.
//!
//! Artifacts owned by these commands (ADR-0028, ADR-0029):
//!
//! 1. **launchd agent/daemon** — launchd keeps `phai serve` alive (RunAtLoad +
//!    KeepAlive) and restarts it across crashes and reboots. The plist points
//!    at the absolute path of the current binary; the self-updater replaces
//!    that path atomically, so it always starts whatever version is installed.
//!    A custom `PHAI_CONFIG_DIR`/`FINANCE_OS_*` in effect at install time is
//!    captured into the plist so the service serves the same store the user
//!    installed from. The system daemon additionally captures `HOME` (a root
//!    process inherits none of the user's environment) so the BigQuery
//!    `service_account_path` and config resolve, and runs with
//!    `--no-auto-update`.
//! 2. **Launcher app** `~/Applications/Phai.app` — a minimal bundle whose
//!    executable opens the web app in the default browser. It shows up in
//!    Launchpad/Spotlight with the φ icon and can be pinned to the Dock;
//!    "installing phai" ends with something clickable.
//!
//! ## Security trade-off (system daemon)
//!
//! The root daemon executes the per-user binary at `~/.local/bin/phai`, which
//! is writable by the (non-root) user. On a multi-user machine that is a local
//! privilege-escalation vector: anyone who can write that path gets code
//! execution as root at next launch. We accept this on a single-admin personal
//! Mac because it preserves `phai self update` (the updater rewrites that same
//! user-writable path in place). A signed/notarized `.pkg` that installs into a
//! root-owned location is the intended future hardening (see ADR-0029).

use anyhow::{bail, Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};
use std::process::Command;

const AGENT_LABEL: &str = "run.phai.serve";
const ICON_BYTES: &[u8] = include_bytes!("../assets/Phai.icns");

/// Absolute path of the system LaunchDaemon plist (`--system`).
const SYSTEM_DAEMON_PLIST: &str = "/Library/LaunchDaemons/run.phai.serve.plist";

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Port the service serves on (default 80 → http://phai.localhost).
    #[arg(long, default_value_t = 80)]
    pub port: u16,

    /// Install a root LaunchDaemon (port 80 capable) via one macOS admin-auth
    /// prompt, instead of a user agent. Required to bind privileged ports.
    #[arg(long)]
    pub system: bool,
}

#[derive(Args, Debug)]
pub struct UninstallArgs {
    /// Remove the root LaunchDaemon (installed with `--system`) via one macOS
    /// admin-auth prompt, instead of the user agent.
    #[arg(long)]
    pub system: bool,
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

/// Log path for the root daemon. A root process cannot write the user's
/// `~/Library/Logs`, so the system daemon logs under the machine-wide tree.
fn system_log_path() -> PathBuf {
    PathBuf::from("/Library/Logs/phai/serve.log")
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

/// Env for the root daemon: the captured config dirs plus `HOME`. A root
/// process inherits none of the user's environment, so without `HOME` the
/// BigQuery `service_account_path` and on-disk config would not resolve.
fn captured_system_env(home: &Path) -> Vec<(String, String)> {
    let mut env = vec![("HOME".to_string(), home.display().to_string())];
    env.extend(captured_env());
    env
}

/// Single-quote a string for safe inclusion in a `/bin/sh` command, closing and
/// reopening the quote around any embedded `'`. The result is injection-proof:
/// no shell metacharacter inside `s` is interpreted.
fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Build the `/bin/sh` command that installs the root daemon: create the log
/// dir, copy the staged plist into place, fix ownership/mode, then reload it.
/// Run as root via `osascript … with administrator privileges`. `staged` is a
/// temp file holding the plist body; `log_dir` is where the daemon writes its
/// log. Every path is single-quoted so spaces and metacharacters are safe.
fn system_install_shell(staged: &Path, dest: &str, log_dir: &Path) -> String {
    let staged = sh_single_quote(&staged.display().to_string());
    let dest = sh_single_quote(dest);
    let log_dir = sh_single_quote(&log_dir.display().to_string());
    format!(
        "mkdir -p {log_dir} && cp {staged} {dest} && \
         chown root:wheel {dest} && chmod 644 {dest} && \
         (launchctl bootout system/{label} 2>/dev/null || true) && \
         launchctl bootstrap system {dest}",
        label = AGENT_LABEL,
    )
}

/// Build the `/bin/sh` command that removes the root daemon: bootout (ignore
/// failure) then delete the plist. Run as root via osascript.
fn system_uninstall_shell(dest: &str) -> String {
    let dest = sh_single_quote(dest);
    format!(
        "(launchctl bootout system/{label} 2>/dev/null || true) && rm -f {dest}",
        label = AGENT_LABEL,
    )
}

/// Wrap a `/bin/sh` command so `osascript` runs it as root behind the native
/// admin-auth dialog. The inner command is embedded as an AppleScript string
/// literal, so we escape AppleScript's `\` and `"`.
fn osascript_admin(shell_cmd: &str) -> String {
    let applescript_literal = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
    format!("do shell script \"{applescript_literal}\" with administrator privileges")
}

/// `<key>EnvironmentVariables</key>` block, or empty when nothing is captured.
fn env_block(env: &[(String, String)]) -> String {
    if env.is_empty() {
        return String::new();
    }
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
}

/// launchd property list for the serve agent (user domain).
fn agent_plist(exe: &Path, port: u16, log: &Path, env: &[(String, String)]) -> String {
    serve_plist(exe, port, log, env, &[])
}

/// launchd property list for the root serve daemon (`--system`).
///
/// Identical shape to the user agent plus `--no-auto-update` (the root daemon
/// must not rewrite its own binary) and a captured `HOME` so a root process
/// resolves the user's config and BigQuery `service_account_path`.
fn system_daemon_plist(exe: &Path, port: u16, log: &Path, env: &[(String, String)]) -> String {
    serve_plist(exe, port, log, env, &["--no-auto-update"])
}

/// Shared plist body. `extra_args` are appended to `ProgramArguments` after the
/// `--port N` pair (e.g. `--no-auto-update` for the root daemon).
fn serve_plist(
    exe: &Path,
    port: u16,
    log: &Path,
    env: &[(String, String)],
    extra_args: &[&str],
) -> String {
    let extra: String = extra_args
        .iter()
        .map(|a| format!("\n      <string>{}</string>", xml_escape(a)))
        .collect();
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
      <string>{port}</string>{extra}
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
        env_block = env_block(env),
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

/// Run `shell_cmd` as root behind the native macOS admin-auth dialog. Returns
/// an error if the user cancels the prompt or the privileged command fails.
fn run_admin(shell_cmd: &str) -> Result<()> {
    let script = osascript_admin(shell_cmd);
    let out = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .context("failed to run osascript")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        // osascript error -128 == user cancelled the auth dialog.
        if err.contains("-128") {
            bail!("admin authorization cancelled");
        }
        bail!("privileged step failed: {err}");
    }
    Ok(())
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

fn dock_plist() -> Result<PathBuf> {
    Ok(home()?.join("Library/Preferences/com.apple.dock.plist"))
}

/// One `persistent-apps` array entry pointing at `bundle`. `_CFURLStringType`
/// `0` is an absolute path; macOS rewrites it to a `file://` URL on read, so
/// [`dock_already_pins`] matches by path substring rather than exact value.
fn dock_tile_entry(bundle: &Path) -> String {
    format!(
        "<dict><key>tile-data</key><dict><key>file-data</key><dict>\
         <key>_CFURLString</key><string>{}</string>\
         <key>_CFURLStringType</key><integer>0</integer>\
         </dict></dict></dict>",
        xml_escape(&bundle.display().to_string())
    )
}

/// True when a `defaults read … persistent-apps` dump already lists `bundle`.
/// Matched on the absolute path, which survives macOS rewriting it into a
/// `file://` URL (the URL still contains the path).
fn dock_already_pins(defaults_dump: &str, bundle: &Path) -> bool {
    defaults_dump.contains(&bundle.display().to_string())
}

/// Add `bundle` to the Dock (idempotent). Returns `true` if a tile was added,
/// `false` if it was already pinned. Best-effort: the Dock is a convenience,
/// never a reason to fail the install.
fn pin_to_dock(bundle: &Path) -> Result<bool> {
    let dump = Command::new("defaults")
        .args(["read", "com.apple.dock", "persistent-apps"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if dock_already_pins(&dump, bundle) {
        return Ok(false);
    }
    let add = Command::new("defaults")
        .args([
            "write",
            "com.apple.dock",
            "persistent-apps",
            "-array-add",
            &dock_tile_entry(bundle),
        ])
        .output()
        .context("failed to run defaults write for the Dock")?;
    if !add.status.success() {
        bail!(
            "defaults write failed: {}",
            String::from_utf8_lossy(&add.stderr).trim()
        );
    }
    let _ = Command::new("killall").arg("Dock").output();
    Ok(true)
}

/// Remove every Dock tile pointing at `bundle`. Best-effort and never fails the
/// uninstall — a leftover tile is a cosmetic annoyance the user can drag out.
/// Returns `true` if at least one tile was removed.
fn unpin_from_dock(bundle: &Path) -> Result<bool> {
    let needle = bundle.display().to_string();
    let plist = dock_plist()?;
    let plist_str = plist.display().to_string();
    // Flush the cfprefs cache to disk so PlistBuddy reads the live array.
    let _ = Command::new("defaults")
        .args(["read", "com.apple.dock", "persistent-apps"])
        .output();
    let mut matching = Vec::new();
    let mut index = 0;
    loop {
        let probe = Command::new("/usr/libexec/PlistBuddy")
            .args([
                "-c",
                &format!("Print persistent-apps:{index}:tile-data:file-data:_CFURLString"),
                &plist_str,
            ])
            .output();
        match probe {
            Ok(out) if out.status.success() => {
                if String::from_utf8_lossy(&out.stdout).contains(&needle) {
                    matching.push(index);
                }
                index += 1;
            }
            // Index out of range (or PlistBuddy missing) → end of the array.
            _ => break,
        }
    }
    if matching.is_empty() {
        return Ok(false);
    }
    // Delete from the tail so earlier indices stay valid as the array shrinks.
    for idx in matching.iter().rev() {
        let _ = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", &format!("Delete persistent-apps:{idx}"), &plist_str])
            .output();
    }
    // Reload the edited prefs and redraw the Dock.
    let _ = Command::new("killall").arg("cfprefsd").output();
    let _ = Command::new("killall").arg("Dock").output();
    Ok(true)
}

pub fn install(args: InstallArgs) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("phai serve install is macOS-only for now (launchd)");
    }
    if args.system {
        install_system(args.port)
    } else {
        install_user_agent(args.port)
    }
}

/// User-domain launchd agent (default). No sudo; cannot bind ports < 1024.
fn install_user_agent(port: u16) -> Result<()> {
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
    std::fs::write(&plist_path, agent_plist(&exe, port, &log, &env))
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

    let bundle = write_app_bundle(port)?;
    let url = app_url(port);

    println!("✅ launchd agent installed: {}", plist_path.display());
    println!("   serving {url} at login (logs: {})", log.display());
    println!("✅ launcher app: {}", bundle.display());
    match pin_to_dock(&bundle) {
        Ok(true) => println!("   pinned to the Dock — click the φ icon to open it."),
        Ok(false) => println!("   already in the Dock — click the φ icon to open it."),
        Err(e) => println!(
            "   (couldn't pin to the Dock automatically: {e} — drag {} there yourself)",
            bundle.display()
        ),
    }
    if !env.is_empty() {
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        println!("   captured into the agent: {}", keys.join(", "));
    }
    Ok(())
}

/// Root LaunchDaemon (`--system`) — the only way to bind port 80. All
/// privileged steps run through a single native admin-auth prompt (ADR-0029).
fn install_system(port: u16) -> Result<()> {
    let exe = std::env::current_exe().context("cannot resolve current executable")?;
    let home = home()?;
    let log = system_log_path();
    let env = captured_system_env(&home);

    // Security: the root daemon executes the user-writable binary at
    // `~/.local/bin/phai` — a local privilege-escalation vector on multi-user
    // machines. Accepted on a single-admin personal Mac to preserve
    // `phai self update`. See the module header and ADR-0029.
    println!(
        "⚠️  the root daemon runs {} as root — keep that path writable only by you.",
        exe.display()
    );

    // Stage the plist in a temp file, then copy it into place as root. This
    // keeps the osascript command short and avoids embedding the plist body.
    let body = system_daemon_plist(&exe, port, &log, &env);
    let staged = std::env::temp_dir().join("run.phai.serve.plist.staged");
    std::fs::write(&staged, &body)
        .with_context(|| format!("failed to stage plist at {}", staged.display()))?;

    let log_dir = log.parent().unwrap_or(Path::new("/Library/Logs/phai"));
    let shell = system_install_shell(&staged, SYSTEM_DAEMON_PLIST, log_dir);
    let result = run_admin(&shell);
    let _ = std::fs::remove_file(&staged);
    result.context("installing the root daemon")?;

    let bundle = write_app_bundle(port)?;
    let url = app_url(port);

    println!("✅ root LaunchDaemon installed: {SYSTEM_DAEMON_PLIST}");
    println!("   serving {url} at boot (logs: {})", log.display());
    println!("✅ launcher app: {}", bundle.display());
    match pin_to_dock(&bundle) {
        Ok(true) => println!("   pinned to the Dock — click the φ icon to open it."),
        Ok(false) => println!("   already in the Dock — click the φ icon to open it."),
        Err(e) => println!(
            "   (couldn't pin to the Dock automatically: {e} — drag {} there yourself)",
            bundle.display()
        ),
    }
    let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
    println!("   captured into the daemon: {}", keys.join(", "));
    Ok(())
}

pub fn uninstall(args: UninstallArgs) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("phai serve uninstall is macOS-only for now (launchd)");
    }
    if args.system {
        uninstall_system()
    } else {
        uninstall_user_agent()
    }
}

fn uninstall_user_agent() -> Result<()> {
    let plist_path = agent_plist_path()?;
    let _ = launchctl(&["bootout", &format!("{}/{AGENT_LABEL}", gui_domain())]);
    let mut removed = Vec::new();
    if plist_path.exists() {
        std::fs::remove_file(&plist_path)
            .with_context(|| format!("failed to remove {}", plist_path.display()))?;
        removed.push(plist_path.display().to_string());
    }
    remove_launcher_bundle(&mut removed)?;
    report_removed(removed, "agent and launcher");
    Ok(())
}

/// Remove the root daemon (`--system`) via one admin-auth prompt, then remove
/// the user-owned launcher bundle without elevation.
fn uninstall_system() -> Result<()> {
    let shell = system_uninstall_shell(SYSTEM_DAEMON_PLIST);
    run_admin(&shell).context("removing the root daemon")?;

    let mut removed = vec![SYSTEM_DAEMON_PLIST.to_string()];
    remove_launcher_bundle(&mut removed)?;
    report_removed(removed, "daemon and launcher");
    Ok(())
}

fn remove_launcher_bundle(removed: &mut Vec<String>) -> Result<()> {
    let bundle = app_bundle_path()?;
    // Drop the Dock tile before the bundle so it never lingers as a broken icon.
    if let Ok(true) = unpin_from_dock(&bundle) {
        removed.push(format!("{} (Dock tile)", bundle.display()));
    }
    if bundle.exists() {
        std::fs::remove_dir_all(&bundle)
            .with_context(|| format!("failed to remove {}", bundle.display()))?;
        removed.push(bundle.display().to_string());
    }
    Ok(())
}

fn report_removed(removed: Vec<String>, what: &str) {
    if removed.is_empty() {
        println!("nothing to remove — {what} were not installed.");
    } else {
        for r in removed {
            println!("removed {r}");
        }
    }
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

    #[test]
    fn dock_tile_entry_points_at_the_bundle() {
        let entry = dock_tile_entry(Path::new("/Users/alex/Applications/Phai.app"));
        assert!(entry.contains("<key>_CFURLString</key>"));
        assert!(entry.contains("<string>/Users/alex/Applications/Phai.app</string>"));
        assert!(entry.contains("<key>_CFURLStringType</key>"));
        assert!(entry.contains("<integer>0</integer>"));
    }

    #[test]
    fn dock_tile_entry_escapes_xml_in_path() {
        let entry = dock_tile_entry(Path::new("/Users/a&b/Applications/Phai.app"));
        assert!(entry.contains("/Users/a&amp;b/Applications/Phai.app"));
        assert!(!entry.contains("a&b"));
    }

    #[test]
    fn dock_already_pins_matches_file_url_form() {
        // macOS rewrites the stored path into a file:// URL; the path substring
        // still matches.
        let dump = r#"(
            { "tile-data" = { "file-data" = {
                "_CFURLString" = "file:///Users/alex/Applications/Phai.app/"; }; }; }
        )"#;
        let bundle = Path::new("/Users/alex/Applications/Phai.app");
        assert!(dock_already_pins(dump, bundle));
    }

    #[test]
    fn dock_already_pins_is_false_when_absent() {
        let dump = r#"( { "tile-data" = { "file-data" = {
            "_CFURLString" = "file:///Applications/Safari.app/"; }; }; } )"#;
        assert!(!dock_already_pins(
            dump,
            Path::new("/Users/alex/Applications/Phai.app")
        ));
    }

    #[test]
    fn system_daemon_plist_has_label_port_no_auto_update_and_home() {
        let plist = system_daemon_plist(
            Path::new("/Users/alex/.local/bin/phai"),
            80,
            Path::new("/Library/Logs/phai/serve.log"),
            &[
                ("HOME".into(), "/Users/alex".into()),
                ("PHAI_CONFIG_DIR".into(), "/Users/alex/cfg".into()),
            ],
        );
        assert!(plist.contains("<string>run.phai.serve</string>"));
        assert!(plist.contains("<string>/Users/alex/.local/bin/phai</string>"));
        assert!(plist.contains("<string>--port</string>"));
        assert!(plist.contains("<string>80</string>"));
        // The root daemon must never rewrite its own binary.
        assert!(plist.contains("<string>--no-auto-update</string>"));
        // HOME is required so a root process resolves the user's config.
        assert!(plist.contains("<key>HOME</key>"));
        assert!(plist.contains("<string>/Users/alex</string>"));
        assert!(plist.contains("<key>PHAI_CONFIG_DIR</key>"));
    }

    #[test]
    fn user_agent_plist_has_no_no_auto_update_flag() {
        let plist = agent_plist(
            Path::new("/usr/local/bin/phai"),
            80,
            Path::new("/tmp/x.log"),
            &[],
        );
        assert!(!plist.contains("--no-auto-update"));
    }

    #[test]
    fn sh_single_quote_neutralizes_metacharacters() {
        assert_eq!(sh_single_quote("/tmp/plain"), "'/tmp/plain'");
        // A space stays inside the quotes; a single quote is closed/reopened.
        assert_eq!(sh_single_quote("/Users/a b/x"), "'/Users/a b/x'");
        assert_eq!(sh_single_quote("a'b"), "'a'\\''b'");
        // No way to break out: `$(...)`, `;`, `&&` are all literal.
        let q = sh_single_quote("x; rm -rf / #$(whoami)");
        assert!(q.starts_with('\'') && q.ends_with('\''));
        assert!(!q.contains("'; rm"));
    }

    #[test]
    fn system_install_shell_quotes_paths_with_spaces() {
        let cmd = system_install_shell(
            Path::new("/tmp/My Stage/run.phai.serve.plist"),
            "/Library/LaunchDaemons/run.phai.serve.plist",
            Path::new("/Library/Logs/phai"),
        );
        // Staged path with a space is single-quoted as one argument.
        assert!(cmd.contains("'/tmp/My Stage/run.phai.serve.plist'"));
        assert!(cmd.contains("chown root:wheel '/Library/LaunchDaemons/run.phai.serve.plist'"));
        assert!(cmd.contains("chmod 644 '/Library/LaunchDaemons/run.phai.serve.plist'"));
        assert!(cmd.contains("mkdir -p '/Library/Logs/phai'"));
        assert!(cmd.contains("launchctl bootout system/run.phai.serve"));
        assert!(cmd
            .contains("launchctl bootstrap system '/Library/LaunchDaemons/run.phai.serve.plist'"));
    }

    #[test]
    fn system_uninstall_shell_boots_out_then_removes() {
        let cmd = system_uninstall_shell("/Library/LaunchDaemons/run.phai.serve.plist");
        assert!(cmd.contains("launchctl bootout system/run.phai.serve"));
        assert!(cmd.contains("rm -f '/Library/LaunchDaemons/run.phai.serve.plist'"));
        // bootout must tolerate "not loaded" so uninstall is idempotent.
        assert!(cmd.contains("|| true"));
    }

    #[test]
    fn osascript_admin_wraps_and_escapes_for_applescript() {
        let script = osascript_admin("cp 'a' 'b' && echo \"hi\"");
        assert!(script.starts_with("do shell script \""));
        assert!(script.ends_with("\" with administrator privileges"));
        // The inner double quotes are escaped for the AppleScript string literal.
        assert!(script.contains("echo \\\"hi\\\""));
    }
}
