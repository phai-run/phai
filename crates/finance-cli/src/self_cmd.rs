use anyhow::Result;
use clap::Subcommand;

use crate::update;

#[derive(Subcommand)]
pub enum SelfCommand {
    /// Report current version, latest available version, and whether an update exists
    Check,
    /// Download and install the latest release, then re-exec
    Update,
}

pub async fn run(cmd: SelfCommand) -> Result<()> {
    match cmd {
        SelfCommand::Check => self_check().await,
        SelfCommand::Update => self_update().await,
    }
}

async fn self_check() -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let target_triple = update::TARGET_TRIPLE;

    println!("Current version: {current_version}");
    println!("Target triple:   {target_triple}");

    let client = update::http_client(30);
    match update::get_latest_release(&client).await {
        Ok(release) => {
            let latest_str = update::strip_tag_prefix(&release.tag_name);
            println!("Latest version:  {latest_str}");
            println!(
                "Release URL:     {}/releases/tag/{}",
                update::REPO_URL,
                release.tag_name
            );

            let latest = update::parse_version(latest_str);
            let current = update::parse_version(current_version);
            match (current, latest) {
                (Ok(c), Ok(l)) if update::is_newer(&l, &c) => {
                    println!("Update available: yes");
                }
                _ => {
                    println!("Update available: no (already up to date)");
                }
            }
        }
        Err(e) => {
            println!("Latest version:  unavailable");
            println!("Update available: unknown");
            eprintln!("Warning: could not check for updates: {e}");
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
                "Error: failed to restart after update. Please run 'finance self update' again."
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("Error: update failed: {e}");
            Ok(())
        }
    }
}
