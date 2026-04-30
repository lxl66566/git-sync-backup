use std::{
    env,
    path::{Path, PathBuf},
};

use home::home_dir;
use log::LevelFilter;

use crate::error::{GsbError, Result};

#[inline]
pub fn log_init() {
    #[cfg(not(debug_assertions))]
    log_init_with_default_level(LevelFilter::Info);
    #[cfg(debug_assertions)]
    log_init_with_default_level(LevelFilter::Debug);
}

#[inline]
pub fn log_init_with_default_level(level: LevelFilter) {
    _ = pretty_env_logger::formatted_builder()
        .filter_level(level)
        .format_timestamp_secs()
        .parse_default_env()
        .try_init();
}

/// 获取当前设备的主机名
pub fn get_current_device_name() -> Result<String> {
    machine_uid::get().map_err(|_| GsbError::DeviceNameError)
}

/// 从当前目录开始向上查找 `.gsb.config.toml` 文件所在的目录，作为仓库根目录
pub fn find_repo_root() -> Result<PathBuf> {
    let current_dir = env::current_dir()?;
    let mut current_path: &Path = current_dir.as_ref();

    loop {
        if current_path.join(".gsb.config.toml").is_file() {
            return Ok(current_path.to_path_buf());
        }

        match current_path.parent() {
            Some(parent) => current_path = parent,
            None => return Err(GsbError::RepoRootNotFound),
        }
    }
}

pub fn expand_tilde(path: PathBuf) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~")
        && let Some(home) = home_dir()
    {
        return home.join(stripped);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        if let Some(home) = home_dir() {
            let input = PathBuf::from("~/Documents");
            let result = expand_tilde(input);
            assert_eq!(result, home.join("Documents"));
        }

        // 不以 ~ 开头 -> 原样返回
        let input = PathBuf::from("/absolute/path");
        let result = expand_tilde(input);
        assert_eq!(result, PathBuf::from("/absolute/path"));

        let input = PathBuf::from("relative/path");
        let result = expand_tilde(input);
        assert_eq!(result, PathBuf::from("relative/path"));
    }

    #[test]
    fn test_get_current_device_name() {
        let result = get_current_device_name();
        assert!(result.is_ok());
        let name = result.unwrap();
        assert!(!name.is_empty());
    }
}
