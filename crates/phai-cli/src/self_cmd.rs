use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use crate::update;

#[derive(Subcommand)]
pub enum SelfCommand {
    /// Report current version, latest available version, and whether an update exists
    Check(SelfCheckArgs),
    /// Download and install the latest release, then re-exec
    Update,
}

#[derive(Args)]
pub struct SelfCheckArgs {
    /// Emit machine-readable JSON instead of the default human-readable table
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct SelfCheckReport<'a> {
    current_version: &'a str,
    target_triple: &'a str,
    latest_version: Option<String>,
    release_url: Option<String>,
    update_available: Option<bool>,
    error: Option<String>,
}

pub async fn run(cmd: SelfCommand) -> Result<()> {
    match cmd {
        SelfCommand::Check(args) => self_check(args).await,
        SelfCommand::Update => self_update().await,
    }
}

async fn self_check(args: SelfCheckArgs) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let target_triple = update::TARGET_TRIPLE;

    let client = update::http_client(30);
    let result = update::get_latest_release(&client).await;

    let report = match &result {
        Ok(release) => {
            let latest_str = update::strip_tag_prefix(&release.tag_name).to_string();
            let release_url = format!("{}/releases/tag/{}", update::REPO_URL, release.tag_name);
            let update_available = match (
                update::parse_version(current_version),
                update::parse_version(&latest_str),
            ) {
                (Ok(c), Ok(l)) => Some(update::is_newer(&l, &c)),
                _ => None,
            };
            SelfCheckReport {
                current_version,
                target_triple,
                latest_version: Some(latest_str),
                release_url: Some(release_url),
                update_available,
                error: None,
            }
        }
        Err(e) => SelfCheckReport {
            current_version,
            target_triple,
            latest_version: None,
            release_url: None,
            update_available: None,
            error: Some(e.to_string()),
        },
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Current version: {}", report.current_version);
        println!("Target triple:   {}", report.target_triple);
        match (&report.latest_version, &report.release_url) {
            (Some(v), Some(u)) => {
                println!("Latest version:  {v}");
                println!("Release URL:     {u}");
                match report.update_available {
                    Some(true) => println!("Update available: yes"),
                    Some(false) => println!("Update available: no (already up to date)"),
                    None => println!("Update available: unknown (version parse failed)"),
                }
            }
            _ => {
                println!("Latest version:  unavailable");
                println!("Update available: unknown");
                if let Some(e) = &report.error {
                    eprintln!("Warning: could not check for updates: {e}");
                }
            }
        }
    }

    Ok(())
}

async fn self_update() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let current = match update::parse_version(current_version) {
        Ok(v) => v,
        Err(_) => {
            println!("Current version {current_version} is malformed — cannot check for updates.");
            return Ok(());
        }
    };

    let client = update::http_client(30);
    let release = match update::get_latest_release(&client).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: could not check for updates: {e}");
            return Ok(());
        }
    };

    let latest_str = update::strip_tag_prefix(&release.tag_name);
    let latest = match update::parse_version(latest_str) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Error: could not parse latest version: {latest_str}");
            return Ok(());
        }
    };

    if !update::is_newer(&latest, &current) {
        println!("Already up to date ({current_version}).");
        return Ok(());
    }

    println!("Updating from {current_version} to {latest_str}...");

    match update::download_and_replace(&client, &release).await {
        Ok(()) => {
            println!("Updated from {current_version} to {latest_str}. Restarting...");
            update::exec_new_binary(latest_str);
            // exec replaces the process; if we reach here, exec failed
            eprintln!(
                "Error: failed to restart after update. Please run 'phai self update' again."
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("Error: update failed: {e}");
            Ok(())
        }
    }
}
