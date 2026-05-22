use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Bigquery,
    #[default]
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub backend: BackendKind,
    pub actor_id: String,
    pub project_id: Option<String>,
    pub dataset_id: Option<String>,
    pub service_account_path: Option<PathBuf>,
    pub local_db_path: Option<PathBuf>,
    pub pluggy_start_date: Option<String>,
    /// LLM provider for enrichment (Phase 2+): "anthropic", "openai",
    /// "deepseek", "ollama". Defaults read at runtime — no logic in
    /// Phase 1.
    #[serde(default)]
    pub llm_provider: Option<String>,
    /// LLM model override. Defaults vary per provider.
    #[serde(default)]
    pub llm_model: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            backend: BackendKind::Local,
            actor_id: "unset-actor".to_string(),
            project_id: None,
            dataset_id: None,
            service_account_path: None,
            local_db_path: None,
            pluggy_start_date: None,
            llm_provider: None,
            llm_model: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub local_db_file: PathBuf,
}

impl ConfigPaths {
    pub fn discover() -> Result<Self> {
        let config_root = std::env::var_os("FINANCE_OS_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(Self::resolve_config_root)
            .context("Não foi possível resolver o diretório de configuração")?;

        let data_root = std::env::var_os("FINANCE_OS_DATA_DIR")
            .map(PathBuf::from)
            .or_else(Self::resolve_data_root)
            .context("Não foi possível resolver o diretório de dados")?;

        Ok(Self {
            config_dir: config_root.clone(),
            data_dir: data_root.clone(),
            config_file: config_root.join("config.toml"),
            local_db_file: data_root.join("finance-os.local.db"),
        })
    }

    /// Resolve the config directory with XDG precedence over platform native.
    ///
    /// Order:
    /// 1. `$XDG_CONFIG_HOME/finance-os` (or `$HOME/.config/finance-os`) if it
    ///    contains a `config.toml` — this matches the production launcher and
    ///    keeps Linux/macOS configs portable.
    /// 2. The platform-native directory from `dirs::config_dir()` (on macOS
    ///    that's `~/Library/Application Support/finance-os`).
    ///
    /// Falling back to native only when XDG has no `config.toml` avoids the
    /// classic macOS trap where a user puts their config under `~/.config`
    /// (the XDG default) but the binary silently reads a stale config from
    /// `~/Library/Application Support`.
    fn resolve_config_root() -> Option<PathBuf> {
        if let Some(xdg) = Self::xdg_config_finance_os() {
            if xdg.join("config.toml").is_file() {
                return Some(xdg);
            }
        }
        dirs::config_dir().map(|dir| dir.join("finance-os"))
    }

    fn resolve_data_root() -> Option<PathBuf> {
        // Mirror the config resolver: when the user has an XDG config that
        // ships a local_db_path, the platform-native data dir is never
        // touched. For cases without an explicit local_db_path, prefer the
        // XDG data dir when it exists.
        if let Some(xdg) = Self::xdg_data_finance_os() {
            if xdg.exists() {
                return Some(xdg);
            }
        }
        dirs::data_dir().map(|dir| dir.join("finance-os"))
    }

    fn xdg_config_finance_os() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|root| root.join("finance-os"))
    }

    fn xdg_data_finance_os() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .map(|root| root.join("finance-os"))
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir)
            .with_context(|| format!("Falha ao criar {}", self.config_dir.display()))?;
        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("Falha ao criar {}", self.data_dir.display()))?;
        Ok(())
    }
}

impl AppConfig {
    pub fn load(paths: &ConfigPaths) -> Result<Self> {
        if !paths.config_file.exists() {
            return Ok(Self::default().with_default_paths(paths));
        }
        let content = fs::read_to_string(&paths.config_file)
            .with_context(|| format!("Falha ao ler {}", paths.config_file.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Falha ao parsear {}", paths.config_file.display()))?;
        Ok(config.with_default_paths(paths))
    }

    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure()?;
        let serialized = toml::to_string_pretty(self).context("Falha ao serializar config")?;
        fs::write(&paths.config_file, &serialized)
            .with_context(|| format!("Falha ao gravar {}", paths.config_file.display()))?;
        #[cfg(unix)]
        {
            let perms = std::fs::Permissions::from_mode(0o600);
            fs::set_permissions(&paths.config_file, perms).with_context(|| {
                format!(
                    "Falha ao definir permissões em {}",
                    paths.config_file.display()
                )
            })?;
        }
        Ok(())
    }

    pub fn with_default_paths(mut self, paths: &ConfigPaths) -> Self {
        if self.local_db_path.is_none() {
            self.local_db_path = Some(paths.local_db_file.clone());
        }
        self
    }

    pub fn effective_backend(&self) -> BackendKind {
        match self.backend {
            BackendKind::Local => BackendKind::Local,
            BackendKind::Bigquery => {
                if self
                    .service_account_path
                    .as_ref()
                    .is_some_and(|path| path.exists())
                    && self.project_id.as_deref().is_some()
                    && self.dataset_id.as_deref().is_some()
                {
                    BackendKind::Bigquery
                } else {
                    eprintln!(
                        "aviso: backend configurado como bigquery mas credenciais ausentes — usando local como fallback"
                    );
                    BackendKind::Local
                }
            }
        }
    }

    pub fn project_id(&self) -> Result<&str> {
        self.project_id
            .as_deref()
            .context("project_id não configurado")
    }

    pub fn dataset_id(&self) -> Result<&str> {
        self.dataset_id
            .as_deref()
            .context("dataset_id não configurado")
    }

    pub fn service_account_path(&self) -> Result<&Path> {
        self.service_account_path
            .as_deref()
            .context("service_account_path não configurado")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    fn setup_env(home: &Path, xdg_config: Option<&Path>, xdg_data: Option<&Path>) {
        unsafe {
            std::env::set_var("HOME", home);
            match xdg_config {
                Some(p) => std::env::set_var("XDG_CONFIG_HOME", p),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match xdg_data {
                Some(p) => std::env::set_var("XDG_DATA_HOME", p),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            std::env::remove_var("FINANCE_OS_CONFIG_DIR");
            std::env::remove_var("FINANCE_OS_DATA_DIR");
        }
    }

    #[test]
    #[serial]
    fn discover_prefers_xdg_config_when_config_toml_exists() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        let xdg = home.join(".config").join("finance-os");
        fs::create_dir_all(&xdg).unwrap();
        fs::write(xdg.join("config.toml"), "backend = \"bigquery\"").unwrap();

        setup_env(home, None, None);
        let paths = ConfigPaths::discover().unwrap();
        assert_eq!(paths.config_dir, xdg);
    }

    #[test]
    #[serial]
    fn discover_falls_back_to_native_when_xdg_has_no_config() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        setup_env(home, None, None);

        let paths = ConfigPaths::discover().unwrap();
        // The fallback must come from `dirs::config_dir()`. On Linux that's
        // `$XDG_CONFIG_HOME/finance-os` (so it coincides with the XDG path
        // we'd otherwise prefer); on macOS it's
        // `~/Library/Application Support/finance-os`. Either way the path
        // must be under HOME — that's all this test can portably assert.
        assert!(
            paths.config_dir.starts_with(home),
            "expected fallback under HOME, got {:?}",
            paths.config_dir
        );
        // And critically: on macOS specifically, fallback must NOT equal
        // the XDG path, because that's the whole reason for the resolver.
        #[cfg(target_os = "macos")]
        {
            let xdg_path = home.join(".config").join("finance-os");
            assert_ne!(paths.config_dir, xdg_path);
        }
    }

    #[test]
    #[serial]
    fn discover_honours_explicit_env_var_override() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        let custom = home.join("custom-cfg");
        fs::create_dir_all(&custom).unwrap();
        setup_env(home, None, None);
        unsafe {
            std::env::set_var("FINANCE_OS_CONFIG_DIR", &custom);
        }

        let paths = ConfigPaths::discover().unwrap();
        assert_eq!(paths.config_dir, custom);

        unsafe {
            std::env::remove_var("FINANCE_OS_CONFIG_DIR");
        }
    }
}
