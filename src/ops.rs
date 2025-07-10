use std::{
    fs,
    io::{self, BufReader, Read},
    path::Path,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fuck_backslash::FuckBackslash;
use rayon::prelude::*;

use crate::{
    config::{Config, get_actual_device_hash},
    error::{GsbError, Result},
    git::GsbRepo,
    utils::{self, expand_tilde},
};

/// 逐字节比较两个文件的内容是否相等。
///
/// 仅在文件大小相同但修改时间不可靠时作为备用检查方法。
/// 为了提高效率，使用了带缓冲的读取器。
///
/// # Arguments
///
/// * `path1` - 第一个文件的路径
/// * `path2` - 第二个文件的路径
///
/// # Returns
///
/// 如果文件内容完全相同，返回 `Ok(true)`，否则返回 `Ok(false)`。
/// 如果发生 I/O 错误，则返回 `Err`。
fn are_contents_equal(path1: &Path, path2: &Path) -> io::Result<bool> {
    // 使用带缓冲的读取器以获得更好的性能
    let mut f1 = BufReader::new(fs::File::open(path1)?);
    let mut f2 = BufReader::new(fs::File::open(path2)?);

    let mut buf1 = [0; 8192]; // 8KB 缓冲区
    let mut buf2 = [0; 8192];

    loop {
        let bytes_read1 = f1.read(&mut buf1)?;
        let bytes_read2 = f2.read(&mut buf2)?;

        // 如果读取的字节数不同，说明文件不同（理论上在大小相同时不应发生）
        if bytes_read1 != bytes_read2 {
            return Ok(false);
        }

        // 如果都读取到了文件末尾（读取字节数为0），则说明文件内容相同
        if bytes_read1 == 0 {
            return Ok(true);
        }

        // 比较当前缓冲区的内容
        if buf1[..bytes_read1] != buf2[..bytes_read2] {
            return Ok(false);
        }
    }
}

/// 统一的文件/文件夹智能拷贝函数
///
/// 该函数会比较源和目标，只在必要时执行 I/O 操作，以最小化磁盘写入。
///
/// - 如果源是文件：
///   1. 优先比较文件大小和修改时间。
///   2. 如果修改时间不可用，则回退到逐字节的内容比较，确保拷贝的准确性。
/// - 如果源是目录：
///   - 递归地对目录内容应用相同的智能拷贝逻辑。
fn copy_item(from: &Path, to: &Path) -> Result<()> {
    if !from.exists() {
        log::warn!("Source path does not exist, skipping copy: {from:?}");
        return Ok(());
    }

    // 如果目标路径的父目录不存在，则创建它
    if let Some(parent) = to.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    if from.is_dir() {
        // --- 目录拷贝逻辑 ---
        if !to.exists() {
            fs::create_dir(to)?;
        }

        // 递归拷贝目录内容
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let source_path = entry.path();
            let dest_path = to.join(entry.file_name());
            copy_item(&source_path, &dest_path)?; // 递归调用
        }
        return Ok(());
    }

    // --- 文件拷贝逻辑 ---
    let mut should_copy = true;
    if to.exists() {
        let from_meta = fs::metadata(from)?;
        let to_meta = fs::metadata(to)?;

        // 1. 快速检查：比较文件大小。如果大小不同，必须复制。
        if from_meta.len() == to_meta.len() {
            // 大小相同，继续检查。
            // 2. 尝试通过修改时间进行检查（快速且常用）。
            if let (Ok(from_time), Ok(to_time)) = (from_meta.modified(), to_meta.modified()) {
                // 修改时间可用，进行比较。
                if from_time.duration_since(UNIX_EPOCH).unwrap().as_secs()
                    == to_time.duration_since(UNIX_EPOCH).unwrap().as_secs()
                {
                    should_copy = false; // 大小和修改时间都相同，跳过复制。
                }
            } else {
                // 3. 备用方案：修改时间不可用，回退到更可靠但较慢的逐字节比较。
                log::warn!(
                    "Could not read modification time for {from:?} or {to:?}. Falling back to byte-by-byte comparison."
                );
                if are_contents_equal(from, to)? {
                    should_copy = false; // 文件内容相同，跳过复制。
                }
            }
        }
    }

    if should_copy {
        log::debug!("Copying file: {from:?} -> {to:?}");
        fs::copy(from, to)?;
    } else {
        log::trace!("Skipping unchanged file: {from:?}");
    }

    Ok(())
}

/// 处理 `collect` 命令
pub fn handle_collect(config: &Config, repo_root: &Path) -> Result<()> {
    log::info!("Starting collection process...");
    let device_name = utils::get_current_device_name()?;
    let repo = GsbRepo::open(repo_root)?;

    // Use Rayon for parallel processing
    config.items.par_iter().try_for_each(|item| -> Result<()> {
        // ignore_collect 内可以填写原始 device name 或其 alias，因此两种都要检查
        let mut mapped = item
            .ignore_collect
            .iter()
            .map(|x| get_actual_device_hash(x, &config.aliases));
        if item.ignore_collect.iter().any(|x| x == &device_name) && mapped.any(|x| x == device_name)
        {
            log::info!(
                "Skip    collect for '{}' on this device: ignored.",
                item.path_in_repo
            );
            return Ok(());
        }

        let source_path = item
            .get_source_for_device(&device_name, &config.aliases)
            .ok_or_else(|| {
                GsbError::SourcePathNotFound(item.path_in_repo.clone(), device_name.clone())
            })?;

        // Expand tilde in path
        let source_path = expand_tilde(source_path).fuck_backslash();

        let dest_path = repo_root.join(&item.path_in_repo).fuck_backslash();

        if !source_path.exists() {
            log::error!("Source path does not exist, skipping: {source_path:?}");
            return Ok(());
        }

        // Handle hardlinks
        if item.is_hardlink {
            log::info!(
                "Skip    collect for hardlink item '{}' as it should be kept in sync manually.",
                item.path_in_repo
            );
            return Ok(());
        } else {
            log::info!("Collecting {source_path:?} -> {dest_path:?}");
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
    let commit_message = format!("gsb collect on {device_name} at {timestamp}");
    repo.add_and_commit(&commit_message)?;

    log::info!("Collection process finished.");
    Ok(())
}

/// 处理 `restore` 命令
pub fn handle_restore(config: &Config, repo_root: &Path) -> Result<()> {
    log::info!("Starting restore process...");
    let device_name = utils::get_current_device_name()?;

    // Use Rayon for parallel processing
    config.items.par_iter().try_for_each(|item| -> Result<()> {
        // ignore_restore 内可以填写原始 device name 或其 alias，因此两种都要检查
        let mut mapped = item
            .ignore_restore
            .iter()
            .map(|x| get_actual_device_hash(x, &config.aliases));
        if item.ignore_restore.iter().any(|x| x == &device_name) && mapped.any(|x| x == device_name)
        {
            log::info!(
                "Skip    restore for '{}' on this device: ignored.",
                item.path_in_repo
            );
            return Ok(());
        }

        let source_path = repo_root.join(&item.path_in_repo);
        let dest_path = item
            .get_source_for_device(&device_name, &config.aliases)
            .ok_or_else(|| {
                GsbError::SourcePathNotFound(item.path_in_repo.clone(), device_name.clone())
            })?;

        // Expand tilde in path
        let dest_path = expand_tilde(dest_path);

        if !source_path.exists() {
            log::error!("Source path in repo does not exist, skipping: {source_path:?}");
            return Ok(());
        }

        // Handle hardlinks
        if item.is_hardlink {
            log::info!(
                "Skip    restore for hardlink item '{}' as it should be kept in sync manually.",
                item.path_in_repo
            );
            return Ok(());
        } else {
            log::info!("Restore {source_path:?} -> {dest_path:?}");
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
                    log::error!("Failed to restore after pull: {e}");
                }
            }
            Err(e) => {
                log::error!("Failed to pull from remote: {e}");
            }
        }

        log::info!("Sync cycle finished. Sleeping for {sleep_duration:?}...");
        thread::sleep(sleep_duration);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs::{self, File},
        io::Write,
    };

    use tempfile::tempdir;

    use super::*;
    use crate::config::{Config, GitConfig, Item};

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
            aliases: HashMap::from([(
                "alias1".to_string(),
                utils::get_current_device_name().unwrap(),
            )]), // test alias
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
                        "alias1".to_string(),
                        work_dir.path().join("ignored_source.txt"),
                    )])),
                    ignore_collect: vec![utils::get_current_device_name().unwrap()], /* 忽略当前设备的收集 */
                    ignore_restore: vec![],
                },
            ],
        };

        (repo_dir, work_dir, config)
    }

    #[test]
    fn test_copy_item() {
        let temp_dir = tempdir().expect("无法创建临时目录");
        let root = temp_dir.path();

        // --- 测试文件拷贝 ---

        // 1. 源文件不存在
        let non_existent_source = root.join("non_existent.txt");
        let dest_path = root.join("dest.txt");
        assert!(copy_item(&non_existent_source, &dest_path).is_ok()); // 应该返回 Ok(()) 且不报错
        assert!(!dest_path.exists());

        // 2. 拷贝新文件
        let source_file1 = root.join("source1.txt");
        File::create(&source_file1)
            .unwrap()
            .write_all(b"content1")
            .unwrap();
        let dest_file1 = root.join("dest1.txt");
        copy_item(&source_file1, &dest_file1).unwrap();
        assert!(dest_file1.exists());
        assert_eq!(fs::read_to_string(&dest_file1).unwrap(), "content1");

        // 3. 拷贝文件，目标已存在且内容相同（通过大小和修改时间）
        let source_file2 = root.join("source2.txt");
        let dest_file2 = root.join("dest2.txt");
        File::create(&source_file2)
            .unwrap()
            .write_all(b"content2")
            .unwrap();
        File::create(&dest_file2)
            .unwrap()
            .write_all(b"content2")
            .unwrap();
        // 确保修改时间一致，以便跳过拷贝
        let now = SystemTime::now();
        filetime::set_file_mtime(&source_file2, filetime::FileTime::from_system_time(now)).unwrap();
        filetime::set_file_mtime(&dest_file2, filetime::FileTime::from_system_time(now)).unwrap();

        let dest_file2_meta_before = fs::metadata(&dest_file2).unwrap();
        copy_item(&source_file2, &dest_file2).unwrap();
        let dest_file2_meta_after = fs::metadata(&dest_file2).unwrap();
        assert_eq!(
            dest_file2_meta_before.modified().unwrap(),
            dest_file2_meta_after.modified().unwrap()
        ); // 确认没有被修改

        // 4. 拷贝文件，目标已存在但内容不同（大小不同）
        let source_file3 = root.join("source3.txt");
        let dest_file3 = root.join("dest3.txt");
        File::create(&source_file3)
            .unwrap()
            .write_all(b"new content3")
            .unwrap();
        File::create(&dest_file3)
            .unwrap()
            .write_all(b"old")
            .unwrap();
        copy_item(&source_file3, &dest_file3).unwrap();
        assert_eq!(fs::read_to_string(&dest_file3).unwrap(), "new content3");

        // --- 测试目录拷贝 ---

        // 5. 拷贝新目录
        let source_dir1 = root.join("source_dir1");
        fs::create_dir(&source_dir1).unwrap();
        File::create(source_dir1.join("file_in_dir1.txt"))
            .unwrap()
            .write_all(b"dir content")
            .unwrap();
        let dest_dir1 = root.join("dest_dir1");
        copy_item(&source_dir1, &dest_dir1).unwrap();
        assert!(dest_dir1.exists());
        assert!(dest_dir1.join("file_in_dir1.txt").exists());
        assert_eq!(
            fs::read_to_string(dest_dir1.join("file_in_dir1.txt")).unwrap(),
            "dir content"
        );

        // 6. 拷贝目录，目标目录已存在且包含内容
        let source_dir2 = root.join("source_dir2");
        fs::create_dir(&source_dir2).unwrap();
        File::create(source_dir2.join("file_a.txt"))
            .unwrap()
            .write_all(b"content A")
            .unwrap();
        fs::create_dir(source_dir2.join("subdir")).unwrap();
        File::create(source_dir2.join("subdir").join("file_b.txt"))
            .unwrap()
            .write_all(b"content B")
            .unwrap();

        let dest_dir2 = root.join("dest_dir2");
        fs::create_dir(&dest_dir2).unwrap();
        File::create(dest_dir2.join("old_file.txt"))
            .unwrap()
            .write_all(b"old content")
            .unwrap();

        copy_item(&source_dir2, &dest_dir2).unwrap();
        assert!(dest_dir2.exists());
        assert!(dest_dir2.join("file_a.txt").exists());
        assert_eq!(
            fs::read_to_string(dest_dir2.join("file_a.txt")).unwrap(),
            "content A"
        );
        assert!(dest_dir2.join("subdir").exists());
        assert!(dest_dir2.join("subdir").join("file_b.txt").exists());
        assert_eq!(
            fs::read_to_string(dest_dir2.join("subdir").join("file_b.txt")).unwrap(),
            "content B"
        );
        // 确认旧文件仍然存在
        assert!(dest_dir2.join("old_file.txt").exists());
        assert_eq!(
            fs::read_to_string(dest_dir2.join("old_file.txt")).unwrap(),
            "old content"
        );
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
        assert!(!collected_hardlink_path.exists()); // 硬链接现在应该被跳过，不应存在于仓库中

        // 验证 ignored_file.txt 是否被忽略 (不应该存在于仓库中)
        let collected_ignored_file_path = repo_root.join("ignored_file.txt");
        assert!(!collected_ignored_file_path.exists());

        // 验证 Git 提交
        let repo = git2::Repository::open(repo_root).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let commit_message = head.message().unwrap();
        assert!(commit_message.contains("gsb collect on"));
        assert!(commit_message.contains(&utils::get_current_device_name().unwrap()));
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
    }
}
