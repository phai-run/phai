use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::NamedTempFile;

pub(crate) fn is_production_port(port: u16) -> bool {
    port == 80
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ServeLock {
    pid: u32,
    version: String,
    started_at: String,
}

pub(crate) async fn acquire(data_dir: &Path, port: u16) -> Result<()> {
    if !should_lock(port) {
        return Ok(());
    }

    fs::create_dir_all(data_dir)
        .with_context(|| format!("failed to create data dir {}", data_dir.display()))?;
    let _guard = ExclusiveGuard::acquire(data_dir, port).await?;
    let path = lock_path(data_dir, port);

    if let Some(existing) = read_lock(&path)? {
        if existing.pid != std::process::id() && pid_alive(existing.pid) {
            eprintln!(
                "[phai serve] produção :{port} já tinha PID {}; solicitando encerramento...",
                existing.pid
            );
            signal(existing.pid, "-TERM");
            if !wait_dead(existing.pid, Duration::from_secs(5)).await {
                eprintln!(
                    "[phai serve] PID {} não encerrou após SIGTERM; enviando SIGKILL...",
                    existing.pid
                );
                signal(existing.pid, "-KILL");
                let _ = wait_dead(existing.pid, Duration::from_secs(3)).await;
            }
        }
    }

    write_lock(
        &path,
        &ServeLock {
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: Utc::now().to_rfc3339(),
        },
    )
}

pub(crate) fn release(data_dir: &Path, port: u16) {
    if !should_lock(port) {
        return;
    }
    let path = lock_path(data_dir, port);
    match read_lock(&path) {
        Ok(Some(lock)) if lock.pid == std::process::id() => {
            if let Err(e) = fs::remove_file(&path) {
                eprintln!("[phai serve] falha ao remover lock {}: {e}", path.display());
            }
        }
        Ok(_) => {}
        Err(e) => eprintln!("[phai serve] falha ao ler lock {}: {e:#}", path.display()),
    }
}

fn should_lock(port: u16) -> bool {
    cfg!(unix) && (cfg!(not(debug_assertions)) || cfg!(test)) && is_production_port(port)
}

fn lock_path(data_dir: &Path, port: u16) -> PathBuf {
    data_dir.join(format!("serve-{port}.lock"))
}

fn guard_path(data_dir: &Path, port: u16) -> PathBuf {
    data_dir.join(format!("serve-{port}.lock.acquire"))
}

fn read_lock(path: &Path) -> Result<Option<ServeLock>> {
    match fs::read_to_string(path) {
        Ok(body) => Ok(Some(serde_json::from_str(&body).with_context(|| {
            format!("failed to parse serve lock {}", path.display())
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn write_lock(path: &Path, lock: &ServeLock) -> Result<()> {
    let parent = path.parent().context("lock path has no parent")?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, lock)?;
    tmp.as_file_mut().sync_all()?;
    tmp.persist(path)
        .map(|_| ())
        .map_err(|e| e.error)
        .with_context(|| format!("failed to persist {}", path.display()))
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn signal(pid: u32, sig: &str) {
    let _ = Command::new("kill").args([sig, &pid.to_string()]).status();
}

async fn wait_dead(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    !pid_alive(pid)
}

struct ExclusiveGuard {
    path: PathBuf,
}

/// How long to wait for a stale `.acquire` guard before assuming its owner
/// crashed (SIGKILL, OOM, power loss) without running its `Drop` and removing
/// it ourselves. Without this, a single unclean shutdown would permanently
/// deadlock every future `phai serve` start on this port — exactly the
/// failure mode the takeover lock exists to fix.
const STALE_GUARD_TIMEOUT: Duration = Duration::from_secs(30);

impl ExclusiveGuard {
    async fn acquire(data_dir: &Path, port: u16) -> Result<Self> {
        Self::acquire_with_timeout(data_dir, port, STALE_GUARD_TIMEOUT).await
    }

    async fn acquire_with_timeout(data_dir: &Path, port: u16, timeout: Duration) -> Result<Self> {
        let path = guard_path(data_dir, port);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if tokio::time::Instant::now() >= deadline {
                        eprintln!(
                            "[phai serve] guard {} parado há {}s; assumindo dono anterior morreu sem limpar e removendo...",
                            path.display(),
                            timeout.as_secs()
                        );
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("failed to acquire {}", path.display()))
                }
            }
        }
    }
}

impl Drop for ExclusiveGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn production_port_is_only_80() {
        assert!(is_production_port(80));
        assert!(!is_production_port(4317));
        assert!(!is_production_port(8080));
        assert!(!is_production_port(1));
    }

    #[test]
    fn lock_json_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(dir.path(), 80);
        let lock = ServeLock {
            pid: 123,
            version: "1.2.3".into(),
            started_at: "now".into(),
        };
        write_lock(&path, &lock).unwrap();
        assert_eq!(read_lock(&path).unwrap(), Some(lock));
    }

    #[tokio::test]
    async fn takeover_kills_live_process_and_writes_current_pid() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        write_lock(
            &lock_path(dir.path(), 80),
            &ServeLock {
                pid: child.id(),
                version: "old".into(),
                started_at: "old".into(),
            },
        )
        .unwrap();
        acquire(dir.path(), 80).await.unwrap();
        let _ = child.wait();
        assert!(!pid_alive(child.id()));
        assert_eq!(
            read_lock(&lock_path(dir.path(), 80)).unwrap().unwrap().pid,
            std::process::id()
        );
    }

    #[tokio::test]
    async fn dead_pid_is_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        write_lock(
            &lock_path(dir.path(), 80),
            &ServeLock {
                pid: 999_999,
                version: "old".into(),
                started_at: "old".into(),
            },
        )
        .unwrap();
        acquire(dir.path(), 80).await.unwrap();
        assert_eq!(
            read_lock(&lock_path(dir.path(), 80)).unwrap().unwrap().pid,
            std::process::id()
        );
    }

    #[tokio::test]
    async fn non_production_port_does_not_create_lock() {
        let dir = tempfile::tempdir().unwrap();
        acquire(dir.path(), 4317).await.unwrap();
        assert!(!lock_path(dir.path(), 4317).exists());
    }

    #[test]
    fn release_removes_only_current_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = lock_path(dir.path(), 80);
        write_lock(
            &path,
            &ServeLock {
                pid: std::process::id(),
                version: "v".into(),
                started_at: "t".into(),
            },
        )
        .unwrap();
        release(dir.path(), 80);
        assert!(!path.exists());
        write_lock(
            &path,
            &ServeLock {
                pid: 999_999,
                version: "v".into(),
                started_at: "t".into(),
            },
        )
        .unwrap();
        release(dir.path(), 80);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn stale_guard_is_removed_after_timeout_instead_of_deadlocking() {
        // Simulates a prior owner that created the `.acquire` guard and was
        // SIGKILLed before its Drop could remove it — acquire must self-heal
        // instead of waiting forever.
        let dir = tempfile::tempdir().unwrap();
        let guard = guard_path(dir.path(), 80);
        std::fs::write(&guard, b"").unwrap();
        let acquired =
            ExclusiveGuard::acquire_with_timeout(dir.path(), 80, Duration::from_millis(50))
                .await
                .unwrap();
        assert_eq!(acquired.path, guard);
    }

    #[tokio::test]
    async fn concurrent_acquire_serializes_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = tokio::join!(acquire(dir.path(), 80), acquire(dir.path(), 80));
        a.unwrap();
        b.unwrap();
        assert_eq!(
            read_lock(&lock_path(dir.path(), 80)).unwrap().unwrap().pid,
            std::process::id()
        );
        assert!(!guard_path(dir.path(), 80).exists());
    }
}
