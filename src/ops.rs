use crate::config::Config;
use crate::error::{GsbError, Result};
use crate::git::GsbRepo;
use crate::utils::{self, expand_tilde}; // <-- MODIFIED
use fs_extra::dir::{copy as copy_dir, CopyOptions};
use fs_extra::file::{copy as copy_file, CopyOptions as FileCopyOptions};
use rayon::prelude::*; // <-- ADDED
use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 处理 `collect` 命令 (并行版本)
pub fn handle_collect(config: &Config, repo_root: &Path) -> Result<()> {
    log::info!("Starting parallel collection process...");
    let device_name = utils::get_current_device_name()?;
    let repo = GsbRepo::open(repo_root)?;

    // Use Rayon for parallel processing
    config.items.par_iter().try_for_each(|item| -> Result<()> {
        if item.ignore_collect.contains(&device_name) {
            log::info!(
                "Skipping collect for '{}' on this device.",
                item.path_in_repo
            );
            return Ok(());
        }

        let source_path = item.get_source_for_device(&device_name).ok_or_else(|| {
            GsbError::SourcePathNotFound(item.path_in_repo.clone(), device_name.clone())
        })?;

        // Expand tilde in path
        let source_path = expand_tilde(source_path);

        let dest_path = repo_root.join(&item.path_in_repo);

        if !source_path.exists() {
            log::warn!("Source path does not exist, skipping: {:?}", source_path);
            return Ok(());
        }

        // 确保目标文件夹存在
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Handle hardlinks
        if item.is_hardlink {
            log::info!(
                "Linking (hardlink) '{:?}' -> '{:?}'",
                source_path,
                dest_path
            );
            // 如果目标已存在，先删除，以确保可以创建新的硬链接
            if dest_path.exists() {
                if dest_path.is_dir() {
                    fs::remove_dir_all(&dest_path)?;
                } else {
                    fs::remove_file(&dest_path)?;
                }
            }
            fs::hard_link(&source_path, &dest_path)
                .map_err(|_| GsbError::HardlinkFailed(source_path.clone(), dest_path.clone()))?;
        } else {
            log::info!("Collecting '{:?}' -> '{:?}'", source_path, dest_path);
            // 如果目标已存在，先删除，避免合并问题
            if dest_path.exists() {
                if dest_path.is_dir() {
                    fs::remove_dir_all(&dest_path)?;
                } else {
                    fs::remove_file(&dest_path)?;
                }
            }
            copy_item(&source_path, &dest_path)?;
        }
        Ok(())
    })?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let commit_message = format!("gsb collect on {} at {}", device_name, timestamp);
    repo.add_and_commit(&commit_message)?;

    log::info!("Collection process finished.");
    Ok(())
}

/// 处理 `restore` 命令 (并行版本)
pub fn handle_restore(config: &Config, repo_root: &Path) -> Result<()> {
    log::info!("Starting parallel restore process...");
    let device_name = utils::get_current_device_name()?;

    // Use Rayon for parallel processing
    config.items.par_iter().try_for_each(|item| -> Result<()> {
        if item.ignore_restore.contains(&device_name) {
            log::info!(
                "Skipping restore for '{}' on this device.",
                item.path_in_repo
            );
            return Ok(());
        }

        let source_path = repo_root.join(&item.path_in_repo);
        let dest_path = item.get_source_for_device(&device_name).ok_or_else(|| {
            GsbError::SourcePathNotFound(item.path_in_repo.clone(), device_name.clone())
        })?;

        // Expand tilde in path
        let dest_path = expand_tilde(dest_path);

        if !source_path.exists() {
            log::warn!(
                "Source path in repo does not exist, skipping: {:?}",
                source_path
            );
            return Ok(());
        }

        // 确保目标文件夹存在
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Handle hardlinks
        if item.is_hardlink {
            log::info!(
                "Linking (hardlink) '{:?}' -> '{:?}'",
                source_path,
                dest_path
            );
            if dest_path.exists() {
                if dest_path.is_dir() {
                    fs::remove_dir_all(&dest_path)?;
                } else {
                    fs::remove_file(&dest_path)?;
                }
            }
            fs::hard_link(&source_path, &dest_path)
                .map_err(|_| GsbError::HardlinkFailed(source_path.clone(), dest_path.clone()))?;
        } else {
            log::info!("Restoring '{:?}' -> '{:?}'", source_path, dest_path);
            copy_item(&source_path, &dest_path)?;
        }
        Ok(())
    })?;

    log::info!("Restore process finished.");
    Ok(())
}

/// 处理 `sync` 命令
pub fn handle_sync(config: &Config, repo_root: &Path) -> Result<()> {
    log::info!(
        "Starting sync process. Interval: {} seconds.",
        config.sync_interval
    );
    let repo = GsbRepo::open(repo_root)?;
    let sleep_duration = Duration::from_secs(config.sync_interval);

    loop {
        log::info!("Running sync cycle...");
        match repo.pull(&config.git.remote, &config.git.branch) {
            Ok(_) => {
                log::info!("Pull successful, now restoring files...");
                if let Err(e) = handle_restore(config, repo_root) {
                    log::error!("Failed to restore after pull: {}", e);
                }
            }
            Err(e) => {
                log::error!("Failed to pull from remote: {}", e);
            }
        }

        log::info!("Sync cycle finished. Sleeping for {:?}...", sleep_duration);
        thread::sleep(sleep_duration);
    }
}

/// 统一的文件/文件夹拷贝函数
fn copy_item(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        let mut options = CopyOptions::new();
        options.overwrite = true;
        options.copy_inside = true;
        copy_dir(from, to, &options)?;
    } else {
        let mut options = FileCopyOptions::new();
        options.overwrite = true;
        copy_file(from, to, &options)?;
    }
    Ok(())
}
