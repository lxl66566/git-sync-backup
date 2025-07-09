use crate::error::{GsbError, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    #[allow(dead_code)]
    pub version: String,
    #[serde(default = "default_sync_interval")]
    pub sync_interval: u64,
    pub git: GitConfig,
    #[serde(rename = "item")]
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
pub struct GitConfig {
    pub remote: String,
    pub branch: String,
}

#[derive(Debug, Deserialize)]
pub struct Item {
    pub path_in_repo: String,
    pub default_source: Option<PathBuf>,
    #[serde(default)]
    pub is_hardlink: bool,
    pub sources: Option<HashMap<String, PathBuf>>,
    #[serde(default)]
    pub ignore_collect: Vec<String>,
    #[serde(default)]
    pub ignore_restore: Vec<String>,
}

impl Config {
    /// 加载并解析配置文件
    pub fn load(repo_root: &Path) -> Result<Self> {
        let config_path = repo_root.join(".gsb.config.toml");
        if !config_path.exists() {
            return Err(GsbError::ConfigNotFound);
        }
        let content = fs::read_to_string(config_path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}

impl Item {
    /// 根据当前设备名获取源路径
    pub fn get_source_for_device(&self, device_name: &str) -> Option<PathBuf> {
        if let Some(sources) = &self.sources {
            if let Some(path) = sources.get(device_name) {
                return Some(path.clone());
            }
        }
        self.default_source.clone()
    }
}

fn default_sync_interval() -> u64 {
    3600
}
