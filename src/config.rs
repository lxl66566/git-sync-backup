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
    #[serde(default)]
    pub ignore: Vec<String>,
}

impl Item {
    /// 根据当前设备名或别名获取源路径
    ///
    /// `sources` 的 key 可以是设备 hash 也可以是别名，因此需要将每个 key
    /// 都通过别名表解析后再与当前设备 hash 比较。
    pub fn get_source_for_device(
        &self,
        device_identifier: &str,
        aliases: &HashMap<String, String>,
    ) -> Option<PathBuf> {
        let actual_device_hash = get_actual_device_hash(device_identifier, aliases);

        if let Some(sources) = &self.sources {
            for (key, path) in sources {
                if get_actual_device_hash(key, aliases) == actual_device_hash {
                    return Some(path.clone());
                }
            }
        }
        self.default_source.clone()
    }

    /// 当前设备是否应忽略 collect 操作
    pub fn is_ignored_for_collect(
        &self,
        device_name: &str,
        aliases: &HashMap<String, String>,
    ) -> bool {
        is_device_in_list(device_name, &self.ignore_collect, aliases)
            || is_device_in_list(device_name, &self.ignore, aliases)
    }

    /// 当前设备是否应忽略 restore 操作
    pub fn is_ignored_for_restore(
        &self,
        device_name: &str,
        aliases: &HashMap<String, String>,
    ) -> bool {
        is_device_in_list(device_name, &self.ignore_restore, aliases)
            || is_device_in_list(device_name, &self.ignore, aliases)
    }
}

/// 检查设备标识（原始 hash 或别名）是否出现在给定列表中
fn is_device_in_list(
    device_name: &str,
    list: &[String],
    aliases: &HashMap<String, String>,
) -> bool {
    let device_hash = get_actual_device_hash(device_name, aliases);
    list.iter()
        .any(|x| get_actual_device_hash(x, aliases) == device_hash)
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
    use std::path::PathBuf;

    use super::*;

    fn extract_toml_blocks(input: &str) -> Vec<String> {
        input
            .split("```toml")
            .skip(1)
            .filter_map(|block| block.split("```").next().map(|s| s.trim().to_string()))
            .collect()
    }

    #[test]
    fn test_parse_config_file() {
        let readme = include_str!("../README.md");
        let config_str = extract_toml_blocks(readme).join("\n");
        let config: Config = toml::from_str(&config_str).unwrap();
        dbg!(config);
    }

    #[test]
    fn test_get_actual_device_hash() {
        let aliases = HashMap::from([
            ("main".to_string(), "hash-aaa".to_string()),
            ("work".to_string(), "hash-bbb".to_string()),
        ]);

        // 传入别名 -> 解析为 hash
        assert_eq!(get_actual_device_hash("main", &aliases), "hash-aaa");
        assert_eq!(get_actual_device_hash("work", &aliases), "hash-bbb");

        // 传入原始 hash -> 原样返回
        assert_eq!(get_actual_device_hash("hash-ccc", &aliases), "hash-ccc");

        // 空别名表 -> 原样返回
        let empty: HashMap<String, String> = HashMap::new();
        assert_eq!(get_actual_device_hash("hash-aaa", &empty), "hash-aaa");
    }

    #[test]
    fn test_get_source_for_device_with_alias_key() {
        // 模拟 aliases: main -> hash-aaa
        let aliases = HashMap::from([
            ("main".to_string(), "hash-aaa".to_string()),
            ("work".to_string(), "hash-bbb".to_string()),
        ]);

        // sources 中使用别名 "work" 作为 key
        let item = Item {
            path_in_repo: "test_item".to_string(),
            default_source: Some(PathBuf::from("/default/path")),
            is_hardlink: false,
            sources: Some(HashMap::from([(
                "work".to_string(),
                PathBuf::from("/work/path"),
            )])),
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
        };

        // 用原始 hash 查找，应该能通过别名解析到 sources 中的路径
        let result = item.get_source_for_device("hash-bbb", &aliases);
        assert_eq!(result, Some(PathBuf::from("/work/path")));

        // 用别名查找也应该工作
        let result = item.get_source_for_device("work", &aliases);
        assert_eq!(result, Some(PathBuf::from("/work/path")));

        // 不匹配的设备 -> 回退到 default_source
        let result = item.get_source_for_device("hash-xxx", &aliases);
        assert_eq!(result, Some(PathBuf::from("/default/path")));
    }

    #[test]
    fn test_get_source_for_device_with_hash_key() {
        let aliases = HashMap::from([("main".to_string(), "hash-aaa".to_string())]);

        // sources 中直接使用设备 hash 作为 key
        let item = Item {
            path_in_repo: "test_item".to_string(),
            default_source: Some(PathBuf::from("/default/path")),
            is_hardlink: false,
            sources: Some(HashMap::from([(
                "hash-aaa".to_string(),
                PathBuf::from("/main/path"),
            )])),
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
        };

        let result = item.get_source_for_device("hash-aaa", &aliases);
        assert_eq!(result, Some(PathBuf::from("/main/path")));

        // 通过别名查找也能匹配
        let result = item.get_source_for_device("main", &aliases);
        assert_eq!(result, Some(PathBuf::from("/main/path")));
    }

    #[test]
    fn test_get_source_for_device_no_sources() {
        let aliases = HashMap::new();
        let item = Item {
            path_in_repo: "test_item".to_string(),
            default_source: Some(PathBuf::from("/default/path")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
        };

        let result = item.get_source_for_device("any-device", &aliases);
        assert_eq!(result, Some(PathBuf::from("/default/path")));
    }

    #[test]
    fn test_get_source_for_device_no_default() {
        let aliases = HashMap::new();
        let item = Item {
            path_in_repo: "test_item".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
        };

        let result = item.get_source_for_device("any-device", &aliases);
        assert_eq!(result, None);
    }

    #[test]
    fn test_is_ignored_for_collect_with_alias() {
        let aliases = HashMap::from([("main".to_string(), "hash-aaa".to_string())]);

        let item = Item {
            path_in_repo: "test".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec!["main".to_string()],
            ignore_restore: vec![],
            ignore: vec![],
        };

        // 用原始 hash 检查，应该通过别名匹配
        assert!(item.is_ignored_for_collect("hash-aaa", &aliases));
        // 用别名检查也应该匹配
        assert!(item.is_ignored_for_collect("main", &aliases));
        // 不匹配的设备
        assert!(!item.is_ignored_for_collect("hash-xxx", &aliases));
    }

    #[test]
    fn test_is_ignored_for_restore_with_ignore_field() {
        let aliases = HashMap::new();
        let item = Item {
            path_in_repo: "test".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec!["device-x".to_string()],
        };

        assert!(item.is_ignored_for_restore("device-x", &aliases));
        assert!(!item.is_ignored_for_restore("device-y", &aliases));
    }
}
