//! `.gsb.config.toml` 配置文件的数据模型。
//!
//! ## 设计
//!
//! 每个 `[[item]]` 通过 `ops` + 设备表（`[item.devices.<id>]`）二维表达
//! 「哪些设备执行哪些操作」。相比上一版的 `ignore_collect` /
//! `ignore_restore` / `ignore` / `restore` / `restore_devices` 五个字段，
//! 新模型用一个统一的 `Op` 枚举与设备级覆盖即可表达：
//!
//! - **双向同步**：`ops = ["collect", "restore"]`（缺省）
//! - **仅备份**：`ops = ["collect"]`（不 restore，重要数据保护）
//! - **仅恢复**：`ops = ["restore"]`（首次部署场景）
//! - **完全跳过**：`ops = []`
//!
//! 设备级覆盖可单独覆盖 `source` 与 `ops`，所有 key 均支持别名或原始
//! device hash 混用。

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer};

/// 顶层配置。
#[derive(Debug, Deserialize)]
pub struct Config {
    /// 配置文件版本（仅用于兼容性提示，不强制）。
    #[allow(dead_code)]
    pub version: String,
    /// `gsb sync` 的轮询间隔（秒），缺省 3600。
    #[serde(default = "default_sync_interval")]
    pub sync_interval: u64,
    /// Git 相关配置。
    #[serde(default)]
    pub git: GitConfig,
    /// 设备 ID 的别名表：`alias -> device_id`。
    /// 在 `aliases`、`sources`、`ops` 等位置都可以混用别名与原始 hash。
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// 用户自定义变量，供 `{name}` 路径展开使用。
    #[serde(default)]
    pub vars: HashMap<String, String>,
    /// 所有需要同步的项。
    #[serde(rename = "item", default)]
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GitConfig {
    /// `gsb sync` 使用的远程仓库名（缺省 `origin`）。
    pub remote: Option<String>,
    /// `gsb sync` 使用的分支（缺省 `main`）。
    pub branch: Option<String>,
}

/// 单个同步项。
#[derive(Debug, Deserialize)]
pub struct Item {
    /// 在 gsb 仓库中的相对路径。
    pub path_in_repo: String,
    /// 所有设备的默认 `source` 路径（支持变量展开）。
    /// 若某设备无 `devices.<id>.source` 也无此项，则该设备跳过。
    #[serde(default)]
    pub source: Option<PathBuf>,
    /// 仅当 `source` 是文件时生效：表示仓库内文件与 `source` 是硬链接，
    /// `collect` / `restore` 时跳过该文件。
    #[serde(default)]
    pub is_hardlink: bool,
    /// 缺省操作集合：所有未被 `devices.<id>` 覆盖的设备都遵循此项。
    /// 缺省值为 `["collect", "restore"]`。
    #[serde(default = "default_ops")]
    pub ops: Ops,
    /// 设备级别的覆盖。key 可以是别名或原始 hash。
    #[serde(default, rename = "device")]
    pub devices: HashMap<String, DeviceOverride>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct DeviceOverride {
    /// 该设备的 source 路径，缺省回退到 [`Item::source`]。
    #[serde(default)]
    pub source: Option<PathBuf>,
    /// 该设备的 ops，缺省回退到 [`Item::ops`]。
    #[serde(default)]
    pub ops: Option<Ops>,
}

/// 操作类型：collect（拉到仓库）或 restore（推回本地）。
///
/// 序列化采用 lowercase，且对外支持 `"collect"` / `"restore"` 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Collect,
    Restore,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Op::Collect => f.write_str("collect"),
            Op::Restore => f.write_str("restore"),
        }
    }
}

/// 操作集合封装，便于：
/// 1. 在 TOML 中既接受 `ops = "collect"` 也接受 `ops = ["collect"]`；
/// 2. 在代码中提供 `.contains(op)` 等便捷方法。
///
/// 注意：[`Default`] 返回 `[Collect, Restore]`，与 serde 缺省值一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ops(Vec<Op>);

impl Default for Ops {
    fn default() -> Self {
        Self(vec![Op::Collect, Op::Restore])
    }
}

impl Ops {
    fn default_ops() -> Self {
        Self(vec![Op::Collect, Op::Restore])
    }

    pub fn contains(&self, op: Op) -> bool {
        self.0.contains(&op)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[Op] {
        &self.0
    }

    /// 测试与配置代码中显式构造 [`Ops`]。生产路径上一般来自反序列化。
    pub fn new<I: IntoIterator<Item = Op>>(iter: I) -> Self {
        let v: Vec<Op> = iter.into_iter().collect();
        Self(v)
    }
}

/// 反序列化兼容：单字符串或字符串数组均接受。
impl<'de> Deserialize<'de> for Ops {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StrOrVec {
            Single(Op),
            Multi(Vec<Op>),
        }

        // 先尝试 string / seq of string
        let v = StrOrVec::deserialize(deserializer)?;
        let vec = match v {
            StrOrVec::Single(o) => vec![o],
            StrOrVec::Multi(v) => v,
        };
        // 去重，保持首次出现顺序
        let mut seen = std::collections::HashSet::new();
        let vec: Vec<Op> = vec.into_iter().filter(|o| seen.insert(*o)).collect();
        Ok(Ops(vec))
    }
}

impl Item {
    /// 返回当前设备在该 item 上的有效 ops：
    /// - 若 `devices` 中有该设备的 override，则用 override 的 ops（缺省回退到 item 默认）
    /// - 否则用 item 的 `ops`
    pub fn effective_ops(&self, device_id: &str, aliases: &HashMap<String, String>) -> Ops {
        if let Some(override_entry) = self.find_device_override(device_id, aliases) {
            return override_entry
                .ops
                .clone()
                .unwrap_or_else(|| self.ops.clone());
        }
        self.ops.clone()
    }

    /// 返回当前设备在该 item 上的有效 source 路径（**尚未变量展开**）。
    /// `None` 表示该设备无路径配置，应跳过。
    pub fn effective_source(
        &self,
        device_id: &str,
        aliases: &HashMap<String, String>,
    ) -> Option<PathBuf> {
        if let Some(override_entry) = self.find_device_override(device_id, aliases) {
            return override_entry
                .source
                .clone()
                .or_else(|| self.source.clone());
        }
        self.source.clone()
    }

    /// 在 `devices` 表中查找属于当前设备的 override。
    ///
    /// key 可以是别名也可以是原始 hash，因此需要把每个 key 解析成 hash 后再比较。
    fn find_device_override(
        &self,
        device_id: &str,
        aliases: &HashMap<String, String>,
    ) -> Option<&DeviceOverride> {
        let actual = get_actual_device_hash(device_id, aliases);
        self.devices.iter().find_map(|(k, v)| {
            if get_actual_device_hash(k, aliases) == actual {
                Some(v)
            } else {
                None
            }
        })
    }

    /// 当 `source` 为相对路径时，以仓库根目录解析。
    pub fn resolve_repo_relative(&self, repo_root: &Path) -> PathBuf {
        let p = Path::new(&self.path_in_repo);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            repo_root.join(p)
        }
    }
}

fn default_sync_interval() -> u64 {
    3600
}

fn default_ops() -> Ops {
    Ops::default_ops()
}

/// 输入 device name 或其 alias，解析为实际的设备 hash。
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

    #[test]
    fn deserialize_single_op_string() {
        let toml = r#"
            version = "0.5"
            [[item]]
            path_in_repo = "a"
            ops = "collect"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.items[0].ops.contains(Op::Collect));
        assert!(!cfg.items[0].ops.contains(Op::Restore));
    }

    #[test]
    fn deserialize_ops_array() {
        let toml = r#"
            version = "0.5"
            [[item]]
            path_in_repo = "a"
            ops = ["collect", "restore"]
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.items[0].ops.contains(Op::Collect));
        assert!(cfg.items[0].ops.contains(Op::Restore));
    }

    #[test]
    fn deserialize_ops_default_is_both() {
        let toml = r#"
            version = "0.5"
            [[item]]
            path_in_repo = "a"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.items[0].ops.contains(Op::Collect));
        assert!(cfg.items[0].ops.contains(Op::Restore));
    }

    #[test]
    fn deserialize_device_override() {
        let toml = r#"
            version = "0.5"
            [aliases]
            work = "hash-bbb"

            [[item]]
            path_in_repo = "a"
            source = "/default"
            ops = ["collect", "restore"]

            [item.device.work]
            source = "/work"
            ops = ["collect"]

            [item.device."hash-aaa"]
            ops = []
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let item = &cfg.items[0];

        // work (= hash-bbb)：source override + ops override
        assert_eq!(
            item.effective_source("hash-bbb", &cfg.aliases),
            Some(PathBuf::from("/work"))
        );
        assert!(
            item.effective_ops("hash-bbb", &cfg.aliases)
                .contains(Op::Collect)
        );
        assert!(
            !item
                .effective_ops("hash-bbb", &cfg.aliases)
                .contains(Op::Restore)
        );

        // hash-aaa：ops = []
        assert_eq!(
            item.effective_source("hash-aaa", &cfg.aliases),
            Some(PathBuf::from("/default"))
        );
        assert!(item.effective_ops("hash-aaa", &cfg.aliases).is_empty());

        // 未配置的设备：回退到默认
        assert_eq!(
            item.effective_source("unknown", &cfg.aliases),
            Some(PathBuf::from("/default"))
        );
        assert!(
            item.effective_ops("unknown", &cfg.aliases)
                .contains(Op::Collect)
        );
        assert!(
            item.effective_ops("unknown", &cfg.aliases)
                .contains(Op::Restore)
        );
    }

    #[test]
    fn deserialize_ops_empty_array() {
        let toml = r#"
            version = "0.5"
            [[item]]
            path_in_repo = "a"
            ops = []
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert!(cfg.items[0].ops.is_empty());
    }

    #[test]
    fn deserialize_with_vars() {
        let toml = r#"
            version = "0.5"
            [vars]
            my_root = "/x"

            [[item]]
            path_in_repo = "a"
            source = "{my_root}/a"
        "#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.vars.get("my_root").map(String::as_str), Some("/x"));
        assert_eq!(
            cfg.items[0].source.as_deref(),
            Some(std::path::Path::new("{my_root}/a"))
        );
    }
}
