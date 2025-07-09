use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GsbError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config file format error: {0}")]
    ConfigFormat(#[from] config_file2::error::Error),

    #[error("Config file '.gsb.config.toml' not found in current or parent directories.")]
    ConfigNotFound,

    #[error("Git Error: {0}")]
    Git(#[from] git2::Error),

    #[error("Filesystem Extra Error: {0}")]
    FsExtra(#[from] fs_extra::error::Error),

    #[error("Could not determine repository root.")]
    RepoRootNotFound,

    #[error("Could not determine current device name.")]
    DeviceNameError,

    #[error("Source path not found for item '{0}' on device '{1}'.")]
    SourcePathNotFound(String, String),

    #[error("Failed to create hardlink from {0:?} to {1:?}. This can happen on different filesystems/partitions.")]
    HardlinkFailed(PathBuf, PathBuf),
}

pub type Result<T> = std::result::Result<T, GsbError>;
