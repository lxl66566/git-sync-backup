use crate::error::{GsbError, Result};
use std::env;
use std::path::{Path, PathBuf};

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
