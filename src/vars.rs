//! 路径变量展开
//!
//! 支持在 `source` 字段中使用 `{NAME}` 形式的变量（基于 `easy_strfmt`），
//! 方便跨设备使用同一份配置。变量来源（优先级从高到低）：
//! 1. **内置变量**（不可被覆盖）：
//!    - `{HOME}` —— 用户主目录（来自 `home` crate）
//!    - `{DEVICE}` —— 当前设备 ID（`machine-uid`）
//!    - `{DEVICE_ALIAS}` —— 当前设备在 `[aliases]` 中对应的别名（未配置时
//!      视为未定义，返回错误）
//!    - `{REPO}` —— gsb 仓库根目录的绝对路径
//! 2. **用户自定义变量**：在配置文件 `[vars]` 表中声明，例如： ```toml [vars]
//!    my_root = "/some/root" ``` 则 `{my_root}` 即可被解析为 `/some/root`。
//!
//! 使用 `{{` 与 `}}` 转义字面量花括号。
//!
//! 跨设备的差异化路径应通过 `[item.device.<id>]` 单独配置，而不是依赖
//! 每台设备上不同的 `[vars]`。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use home::home_dir;

use crate::{
    config::{Config, get_actual_device_hash},
    error::{Context, Result},
    utils,
};

/// 一次 collect/restore 流程的变量解析上下文。
///
/// 预先计算好所有内置变量，避免对每个 item 重复 syscall。
#[derive(Debug, Clone)]
pub struct Vars {
    map: HashMap<String, String>,
    device: String,
}

impl Vars {
    pub fn build(config: &Config, device: String, repo_root: &Path) -> Result<Self> {
        let mut map: HashMap<String, String> = HashMap::new();

        for (k, v) in &config.vars {
            map.insert(k.clone(), v.clone());
        }

        if let Some(home) = home_dir() {
            map.insert("HOME".into(), home.to_string_lossy().into_owned());
        }
        map.insert("DEVICE".into(), device.clone());
        map.insert("REPO".into(), repo_root.to_string_lossy().into_owned());

        if let Some(alias) = config.aliases.iter().find_map(|(alias, hash)| {
            if get_actual_device_hash(hash, &config.aliases) == device {
                Some(alias.clone())
            } else {
                None
            }
        }) {
            map.insert("DEVICE_ALIAS".into(), alias);
        }

        Ok(Self { map, device })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// 在 `path` 上展开所有形如 `{NAME}` 的变量。
    ///
    /// 未识别的变量返回错误，未闭合的 `{` 也视为错误。
    /// 使用 `{{` / `}}` 转义字面量花括号。
    pub fn expand(&self, path: &str) -> Result<PathBuf> {
        easy_strfmt::strfmt(path, &self.map)
            .map(PathBuf::from)
            .map_err(|e| match e {
                easy_strfmt::Error::KeyNotFound(key) => {
                    anyhow::anyhow!("Unknown variable: {{{key}}}")
                }
                easy_strfmt::Error::UnmatchedOpenBrace => {
                    anyhow::anyhow!("Unclosed variable reference: '{{' without matching '}}'")
                }
                easy_strfmt::Error::UnmatchedCloseBrace => {
                    anyhow::anyhow!("Extra '}}'")
                }
                easy_strfmt::Error::WriteError(_) => {
                    anyhow::anyhow!("Format error")
                }
            })
            .with_context(|| format!("Failed to expand path: {path:?}"))
    }

    /// 便捷方法：展开 [`Path`] 形式的输入。
    pub fn expand_path(&self, path: &Path) -> Result<PathBuf> {
        self.expand(&path.to_string_lossy())
    }
}

/// 解析当前设备的 ID（machine-uid），用于变量上下文构建。
pub fn current_device() -> Result<String> {
    utils::get_current_device_name().context("Could not get current device ID")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_vars(map: &[(&str, &str)], device: &str) -> Vars {
        let mut m = HashMap::new();
        for (k, v) in map {
            m.insert((*k).to_string(), (*v).to_string());
        }
        Vars {
            map: m,
            device: device.to_string(),
        }
    }

    #[test]
    fn expand_known_var() {
        let v = build_vars(&[("HOME", "/u/home"), ("my", "/x")], "dev-1");
        assert_eq!(
            v.expand("{HOME}/docs").unwrap(),
            PathBuf::from("/u/home/docs")
        );
        assert_eq!(v.expand("{my}/y").unwrap(), PathBuf::from("/x/y"));
    }

    #[test]
    fn expand_unknown_var_errors() {
        let v = build_vars(&[("HOME", "/h")], "dev");
        let err = v.expand("{UNKNOWN}/x").unwrap_err();
        assert!(format!("{err}").contains("UNKNOWN"));
    }

    #[test]
    fn expand_unclosed_brace_errors() {
        let v = build_vars(&[("HOME", "/h")], "dev");
        assert!(v.expand("/abc{HOME").is_err());
    }

    #[test]
    fn expand_no_var() {
        let v = build_vars(&[("HOME", "/h")], "dev");
        assert_eq!(v.expand("/abs/path").unwrap(), PathBuf::from("/abs/path"));
    }

    #[test]
    fn expand_multiple_vars() {
        let v = build_vars(&[("A", "/a"), ("B", "/b")], "dev");
        assert_eq!(v.expand("{A}/{B}/x").unwrap(), PathBuf::from("/a//b/x"));
    }

    #[test]
    fn expand_empty_var_errors() {
        let v = build_vars(&[], "dev");
        assert!(v.expand("{}").is_err());
    }

    #[test]
    fn expand_extra_close_brace_errors() {
        let v = build_vars(&[("A", "x")], "dev");
        assert!(v.expand("}").is_err());
    }

    #[test]
    fn expand_escaped_braces() {
        let v = build_vars(&[("name", "world")], "dev");
        assert_eq!(
            v.expand("{{hello {name}}}").unwrap(),
            PathBuf::from("{hello world}")
        );
    }
}
