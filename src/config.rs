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

        if let Some(sources) = &self.sources
            && let Some(path) = sources.get(&actual_device_hash) {
                return Some(path.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_toml_blocks(input: &str) -> Vec<String> {
        input
            .split("```toml") // 先按起始标记分割
            .skip(1) // 跳过第一个部分（起始标记之前的内容）
            .filter_map(|block| {
                block
                    .split("```")
                    .next() // 再按结束标记分割，取第一个部分
                    .map(|s| s.trim().to_string())
            })
            .collect()
    }

    #[test]
    fn test_parse_config_file() {
        let readme = include_str!("../README-zh_CN.md");
        let config_str = extract_toml_blocks(readme).join("\n");
        let config: Config = toml::from_str(&config_str).unwrap();
        dbg!(config);
    }
}
