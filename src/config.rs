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

/// 单个 item 的 restore 策略。
///
/// 该枚举实现了「白名单优先」的安全语义：新增设备时，若用户未显式将该设备
/// 加入允许列表，则默认不会执行 restore，避免意外覆盖本地数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RestorePolicy {
    /// 所有未被 `ignore_restore` / `ignore` 排除的设备都会 restore（缺省值，
    /// 向后兼容）。
    #[default]
    All,
    /// 白名单模式：仅 `restore_devices` 中列出的设备才会 restore。
    /// 新设备默认不 restore，必须显式配置才生效。
    Explicit,
    /// 任何设备都不 restore，纯备份用途。
    Off,
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
    /// Restore 策略，参见 [`RestorePolicy`]。
    #[serde(default)]
    pub restore: RestorePolicy,
    /// 当 `restore = "explicit"` 时，允许 restore 的设备标识或别名列表。
    #[serde(default)]
    pub restore_devices: Vec<String>,
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

    /// 当前设备是否应忽略 restore 操作。
    ///
    /// 判定优先级（从高到低）：
    /// 1. `ignore` / `ignore_restore` 黑名单 → 总是忽略
    /// 2. [`RestorePolicy::Off`] → 总是忽略
    /// 3. [`RestorePolicy::Explicit`] → 仅当设备不在 `restore_devices` 白名单
    ///    中时忽略
    /// 4. [`RestorePolicy::All`] → 不忽略
    pub fn is_ignored_for_restore(
        &self,
        device_name: &str,
        aliases: &HashMap<String, String>,
    ) -> bool {
        if is_device_in_list(device_name, &self.ignore_restore, aliases)
            || is_device_in_list(device_name, &self.ignore, aliases)
        {
            return true;
        }
        match self.restore {
            RestorePolicy::All => false,
            RestorePolicy::Off => true,
            RestorePolicy::Explicit => {
                !is_device_in_list(device_name, &self.restore_devices, aliases)
            }
        }
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
        .map_or_else(|| device_identifier.to_string(), String::clone)
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
            restore: RestorePolicy::All,
            restore_devices: vec![],
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
            restore: RestorePolicy::All,
            restore_devices: vec![],
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
            restore: RestorePolicy::All,
            restore_devices: vec![],
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
            restore: RestorePolicy::All,
            restore_devices: vec![],
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
            restore: RestorePolicy::All,
            restore_devices: vec![],
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
            restore: RestorePolicy::All,
            restore_devices: vec![],
        };

        assert!(item.is_ignored_for_restore("device-x", &aliases));
        assert!(!item.is_ignored_for_restore("device-y", &aliases));
    }

    #[test]
    fn test_restore_policy_off_skips_all_devices() {
        let aliases = HashMap::new();
        let item = Item {
            path_in_repo: "backup_only".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Off,
            restore_devices: vec![],
        };

        // 任何设备都被跳过
        assert!(item.is_ignored_for_restore("device-a", &aliases));
        assert!(item.is_ignored_for_restore("device-b", &aliases));
    }

    #[test]
    fn test_restore_policy_explicit_whitelist() {
        let aliases = HashMap::from([("main".to_string(), "hash-aaa".to_string())]);

        let item = Item {
            path_in_repo: "work_docs".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Explicit,
            restore_devices: vec!["main".to_string()],
        };

        // 白名单内的设备（通过别名或 hash）可以 restore
        assert!(!item.is_ignored_for_restore("main", &aliases));
        assert!(!item.is_ignored_for_restore("hash-aaa", &aliases));
        // 白名单外的设备被跳过 —— 这是「新增设备默认安全」的关键
        assert!(item.is_ignored_for_restore("hash-bbb", &aliases));
        assert!(item.is_ignored_for_restore("new-device", &aliases));
    }

    #[test]
    fn test_restore_policy_all_default_behavior() {
        let aliases = HashMap::new();
        let item = Item {
            path_in_repo: "shared".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        };

        // 缺省策略：所有设备都 restore
        assert!(!item.is_ignored_for_restore("any-device", &aliases));
    }

    #[test]
    fn test_restore_ignore_blacklist_overrides_policy() {
        // ignore_restore 黑名单优先级高于 restore 策略
        let aliases = HashMap::new();
        let item = Item {
            path_in_repo: "test".to_string(),
            default_source: None,
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec!["banned".to_string()],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        };

        assert!(item.is_ignored_for_restore("banned", &aliases));
    }

    #[test]
    fn test_deserialize_restore_policy() {
        let toml_str = r#"
            version = "0.3.0"
            [[item]]
            path_in_repo = "a"
            restore = "off"

            [[item]]
            path_in_repo = "b"
            restore = "explicit"
            restore_devices = ["main"]

            [[item]]
            path_in_repo = "c"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.items[0].restore, RestorePolicy::Off);
        assert_eq!(config.items[1].restore, RestorePolicy::Explicit);
        assert_eq!(config.items[1].restore_devices, vec!["main".to_string()]);
        // 缺省 = All
        assert_eq!(config.items[2].restore, RestorePolicy::All);
    }
}
