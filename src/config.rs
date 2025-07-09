use std::{collections::HashMap, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[allow(dead_code)]
    pub version: String,
    #[serde(default = "default_sync_interval")]
    pub sync_interval: u64,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(rename = "item")]
    pub items: Vec<Item>,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GitConfig {
    pub remote: Option<String>,
    pub branch: Option<String>,
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

impl Item {
    /// 根据当前设备名或别名获取源路径
    pub fn get_source_for_device(
        &self,
        device_identifier: &str,
        aliases: &HashMap<String, String>,
    ) -> Option<PathBuf> {
        let actual_device_hash = get_actual_device_hash(device_identifier, aliases);

        if let Some(sources) = &self.sources {
            if let Some(path) = sources.get(&actual_device_hash) {
                return Some(path.clone());
            }
        }
        self.default_source.clone()
    }
}

fn default_sync_interval() -> u64 {
    3600
}

/// 输入 device name 或其 alias，解析为实际的设备哈希
#[inline]
pub fn get_actual_device_hash(
    device_identifier: &str,
    aliases: &HashMap<String, String>,
) -> String {
    aliases
        .get(device_identifier)
        .map_or(device_identifier.to_string(), |alias_hash| {
            alias_hash.to_string()
        })
}
