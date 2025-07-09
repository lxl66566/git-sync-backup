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
        match repo.pull(
            config.git.remote.as_ref().unwrap_or(&"origin".to_string()),
            config.git.branch.as_ref().unwrap_or(&"main".to_string()),
        ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GitConfig, Item};

    use std::collections::HashMap;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::tempdir;

    // 辅助函数：创建并初始化一个临时 Git 仓库和工作目录
    fn setup_test_env() -> (tempfile::TempDir, tempfile::TempDir, Config) {
        let repo_dir = tempdir().expect("无法创建临时仓库目录");
        let work_dir = tempdir().expect("无法创建临时工作目录");

        // 初始化 Git 仓库
        let repo = git2::Repository::init(repo_dir.path()).expect("无法初始化 Git 仓库");
        // 首次提交，以便后续可以添加和提交新文件
        let mut index = repo.index().unwrap();
        let oid = index.write_tree().unwrap();
        let signature = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &repo.find_tree(oid).unwrap(),
            &[],
        )
        .unwrap();

        let config = Config {
            version: "0.1.0".to_string(),
            sync_interval: 3600,
            git: GitConfig {
                remote: None,
                branch: None,
            },
            items: vec![
                Item {
                    path_in_repo: "file1.txt".to_string(),
                    default_source: Some(work_dir.path().join("file1.txt")),
                    is_hardlink: false,
                    sources: None, // 使用 None 确保使用 default_source
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                },
                Item {
                    path_in_repo: "dir1".to_string(),
                    default_source: Some(work_dir.path().join("dir1")),
                    is_hardlink: false,
                    sources: None,
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                },
                Item {
                    path_in_repo: "hardlink_file.txt".to_string(),
                    default_source: Some(work_dir.path().join("hardlink_source.txt")),
                    is_hardlink: true,
                    sources: None,
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                },
                Item {
                    path_in_repo: "ignored_file.txt".to_string(),
                    default_source: Some(work_dir.path().join("ignored_source.txt")),
                    is_hardlink: false,
                    sources: Some(HashMap::from([(
                        utils::get_current_device_name().unwrap(),
                        work_dir.path().join("ignored_source.txt"),
                    )])),
                    ignore_collect: vec![utils::get_current_device_name().unwrap()], // 忽略当前设备的收集
                    ignore_restore: vec![],
                },
            ],
        };

        (repo_dir, work_dir, config)
    }

    #[test]
    fn test_handle_collect() {
        let (repo_dir, work_dir, config) = setup_test_env();
        let repo_root = repo_dir.path();
        let work_root = work_dir.path();

        // 1. 准备源文件和目录
        // file1.txt
        let source_file1_path = work_root.join("file1.txt");
        File::create(&source_file1_path)
            .unwrap()
            .write_all(b"content of file1 in work dir")
            .unwrap();

        // dir1
        let source_dir1_path = work_root.join("dir1");
        fs::create_dir(&source_dir1_path).unwrap();
        File::create(source_dir1_path.join("file_in_dir1.txt"))
            .unwrap()
            .write_all(b"content of file_in_dir1 in work dir")
            .unwrap();

        // hardlink_source.txt
        let hardlink_source_path = work_root.join("hardlink_source.txt");
        File::create(&hardlink_source_path)
            .unwrap()
            .write_all(b"content for hardlink")
            .unwrap();

        // ignored_source.txt
        let ignored_source_path = work_root.join("ignored_source.txt");
        File::create(&ignored_source_path)
            .unwrap()
            .write_all(b"content for ignored file")
            .unwrap();

        // 2. 运行 collect
        handle_collect(&config, repo_root).unwrap();

        // 3. 验证结果

        // 验证 file1.txt 是否被收集
        let collected_file1_path = repo_root.join("file1.txt");
        assert!(collected_file1_path.exists());
        assert_eq!(
            fs::read_to_string(&collected_file1_path).unwrap(),
            "content of file1 in work dir"
        );

        // 验证 dir1 是否被收集
        let collected_dir1_path = repo_root.join("dir1");
        assert!(collected_dir1_path.exists());
        assert!(collected_dir1_path.join("file_in_dir1.txt").exists());
        assert_eq!(
            fs::read_to_string(collected_dir1_path.join("file_in_dir1.txt")).unwrap(),
            "content of file_in_dir1 in work dir"
        );

        // 验证 hardlink_file.txt 是否被收集并是硬链接
        let collected_hardlink_path = repo_root.join("hardlink_file.txt");
        assert!(collected_hardlink_path.exists());
        assert_eq!(
            fs::read_to_string(&collected_hardlink_path).unwrap(),
            "content for hardlink"
        );
        // 验证是否是硬链接 (通过 inode 数量)
        #[cfg(unix)] // 硬链接检查在 Unix-like 系统上更可靠
        {
            use std::os::unix::fs::MetadataExt;
            let source_metadata = fs::metadata(&hardlink_source_path).unwrap();
            let collected_metadata = fs::metadata(&collected_hardlink_path).unwrap();
            assert_eq!(source_metadata.ino(), collected_metadata.ino());
            assert_eq!(source_metadata.nlink(), 2); // 原始文件和硬链接
        }

        #[cfg(windows)] // Windows 上硬链接的验证可能需要不同的方法，这里只检查内容
        {
            // Windows 上的硬链接检查更复杂，暂时忽略
        }

        // 验证 ignored_file.txt 是否被忽略 (不应该存在于仓库中)
        let collected_ignored_file_path = repo_root.join("ignored_file.txt");
        assert!(!collected_ignored_file_path.exists());

        // 验证 Git 提交
        let repo = git2::Repository::open(repo_root).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let commit_message = head.message().unwrap();
        assert!(commit_message.contains("gsb collect on"));
        assert!(commit_message.contains(&utils::get_current_device_name().unwrap()));

        // 清理临时目录
        repo_dir.close().unwrap();
        work_dir.close().unwrap();
    }

    #[test]
    fn test_handle_restore() {
        let (repo_dir, work_dir, config) = setup_test_env();
        let repo_root = repo_dir.path();
        let work_root = work_dir.path();

        // 在仓库中创建文件和目录
        let repo_file1_path = repo_root.join("file1.txt");
        File::create(&repo_file1_path)
            .unwrap()
            .write_all(b"content of file1 in repo")
            .unwrap();
        let repo_dir1_path = repo_root.join("dir1");
        fs::create_dir(&repo_dir1_path).unwrap();
        File::create(repo_dir1_path.join("file_in_dir1.txt"))
            .unwrap()
            .write_all(b"content of file_in_dir1 in repo")
            .unwrap();

        // 运行 restore
        handle_restore(&config, repo_root).unwrap();

        // 验证文件是否已恢复到工作目录
        let work_file1_path = work_root.join("file1.txt");
        assert!(work_file1_path.exists());
        assert_eq!(
            fs::read_to_string(&work_file1_path).unwrap(),
            "content of file1 in repo"
        );

        let work_dir1_path = work_root.join("dir1");
        assert!(work_dir1_path.exists());
        assert!(work_dir1_path.join("file_in_dir1.txt").exists());
        assert_eq!(
            fs::read_to_string(work_dir1_path.join("file_in_dir1.txt")).unwrap(),
            "content of file_in_dir1 in repo"
        );

        // 清理临时目录
        repo_dir.close().unwrap();
        work_dir.close().unwrap();
    }
}
