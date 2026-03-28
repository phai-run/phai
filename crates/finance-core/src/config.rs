use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Bigquery,
    Local,
}

impl Default for BackendKind {
    fn default() -> Self {
        Self::Local
    }
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
            pluggy_start_date: Some("2025-12-01".to_string()),
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
            .or_else(|| dirs::config_dir().map(|dir| dir.join("finance-os")))
            .context("Não foi possível resolver o diretório de configuração")?;

        let data_root = std::env::var_os("FINANCE_OS_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|dir| dir.join("finance-os")))
            .context("Não foi possível resolver o diretório de dados")?;

        Ok(Self {
            config_dir: config_root.clone(),
            data_dir: data_root.clone(),
            config_file: config_root.join("config.toml"),
            local_db_file: data_root.join("finance-os.local.db"),
        })
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
        fs::write(&paths.config_file, serialized)
            .with_context(|| format!("Falha ao gravar {}", paths.config_file.display()))?;
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
