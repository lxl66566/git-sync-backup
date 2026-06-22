use std::{
    collections::HashSet,
    fs,
    io::{self, BufReader, Read, Write},
    path::Path,
    thread,
    time::{Duration, UNIX_EPOCH},
};

use fuck_backslash::FuckBackslash;
use log::{debug, error, info, trace, warn};
use rayon::prelude::*;
use same_file::is_same_file;

use crate::{
    config::Config,
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

        // 递归拷贝目录内容，同时收集源文件名用于后续删除
        let mut src_names = HashSet::new();
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let file_name = entry.file_name();
            src_names.insert(file_name.clone());
            let dest_path = to.join(&file_name).fuck_backslash();
            copy_item(&entry.path(), &dest_path)?; // 递归调用
        }

        // 删除目标目录中源已不存在的文件/目录（用 HashSet 避免逐文件 syscall）
        for entry in fs::read_dir(to)? {
            let entry = entry?;
            let file_name = entry.file_name();
            if !src_names.contains(&file_name) {
                let dest_path = entry.path();
                if dest_path.is_dir() {
                    fs::remove_dir_all(&dest_path)?;
                } else {
                    fs::remove_file(&dest_path)?;
                }
                debug!("Removed {dest_path:?} (no longer exists in source)");
            }
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
                warn!(
                    "Could not read modification time for {from:?} or {to:?}. Falling back to byte-by-byte comparison."
                );
                if are_contents_equal(from, to)? {
                    should_copy = false; // 文件内容相同，跳过复制。
                }
            }
        }
    }

    if should_copy {
        debug!("Copying  file: {from:?} -> {to:?}");
        fs::copy(from, to)?;
    } else {
        trace!("Skipping unchanged file: {from:?}");
    }

    Ok(())
}

/// 智能复制文件或目录，支持硬链接和目标路径处理。
///
/// 该函数根据 `is_hardlink` 参数决定是创建硬链接还是执行智能文件/目录拷贝。
/// 它会处理源路径不存在、尝试对非文件路径创建硬链接以及目标路径已存在的情况。
///
/// # Arguments
///
/// * `from` - 源文件或目录的路径。
/// * `to` - 目标文件或目录的路径。
/// * `is_hardlink` - 一个布尔值，如果为 `true`，则尝试创建硬链接； 如果为
///   `false`，则调用 `copy_item` 进行智能拷贝。
///
/// # Behavior
///
/// 1. **源路径检查**: 如果 `from` 路径不存在，则跳过操作并记录错误。
/// 2. **硬链接条件检查**: 如果 `is_hardlink` 为 `true` 但 `from` 不是一个文件，
///    则跳过操作并记录错误（硬链接只能用于文件）。
/// 3. **硬链接处理**:
///     * 如果 `to` 路径已存在且与 `from` 是同一个文件（通过 `is_same_file`
///       判断）， 则跳过硬链接创建，因为目标已是源的硬链接。
///     * 否则，如果 `to`
///       路径存在，会尝试删除它（无论是文件还是目录，但硬链接只对文件有效），
///       然后创建从 `from` 到 `to` 的硬链接。
/// 4. **非硬链接处理**: 如果 `is_hardlink` 为 `false`，则调用 `copy_item`
///    函数， 该函数会智能地比较源和目标，只在必要时执行实际的 I/O 拷贝操作。
///
/// # Returns
///
/// 如果操作成功，返回 `Ok(())`。如果在文件系统操作中发生错误，则返回
/// `Err(GsbError)`。
fn copy_item_all(from: &Path, to: &Path, is_hardlink: bool) -> Result<()> {
    if !from.exists() {
        error!("Source path does not exist, skipping copy: {from:?}");
        return Ok(());
    }
    if is_hardlink && !from.is_file() {
        error!("Source path is not a file, skipping hardlink: {from:?}");
        return Ok(());
    }
    if is_hardlink {
        if to.exists() && is_same_file(from, to)? {
            warn!("Skipping hardlink copy: {from:?} -> {to:?}");
            return Ok(());
        }
        info!("Hardlink {from:?} -> {to:?}");
        _ = fs::remove_file(to); // 尝试删除目标文件，忽略错误
        fs::hard_link(from, to)?;
    } else {
        copy_item(from, to)?;
    }
    Ok(())
}

/// 处理 `collect` 命令
pub fn handle_collect(config: &Config, repo_root: &Path, autocommit: bool) -> Result<()> {
    info!("Starting collection process...");
    let device_name = utils::get_current_device_name()?;
    let repo = GsbRepo::open(repo_root)?;

    // 构建加密计划（若 git_simple_encrypt.toml 存在，密钥必须已配置）
    #[cfg(feature = "encrypt")]
    let active_paths: Vec<&str> = config
        .items
        .iter()
        .filter(|item| !item.is_ignored_for_collect(&device_name, &config.aliases))
        .map(|item| item.path_in_repo.as_str())
        .collect();
    #[cfg(feature = "encrypt")]
    let crypt_plan = CryptPlan::build(repo_root, &active_paths)?;
    #[cfg(feature = "encrypt")]
    let encrypted_paths: Vec<String> = crypt_plan
        .as_ref()
        .map(|p| {
            p.paths
                .iter()
                .map(|pb| pb.to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(not(feature = "encrypt"))]
    let encrypted_paths: Vec<String> = Vec::new();

    config.items.par_iter().try_for_each(|item| -> Result<()> {
        if item.is_ignored_for_collect(&device_name, &config.aliases) {
            warn!(
                "Skip     collect for '{}' on this device: ignored.",
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

        // 加密项不允许硬链接（加密后的密文与源文件内容不同）
        let use_hardlink = if encrypted_paths.contains(&item.path_in_repo) {
            false
        } else {
            item.is_hardlink
        };

        copy_item_all(&source_path, &dest_path, use_hardlink)?;

        Ok(())
    })?;

    // 加密仓库中匹配的文件（原地加密，幂等）
    #[cfg(feature = "encrypt")]
    if let Some(plan) = &crypt_plan {
        info!("Encrypting collected files...");
        plan.encrypt()?;
    }

    info!("Collection process finished.");
    if autocommit {
        let timestamp = chrono::Local::now();
        let commit_message = format!(
            "gsb collect on {device_name} at {}",
            timestamp.format("%Y-%m-%d %H:%M:%S")
        );
        repo.add_and_commit(&commit_message)?;
    }
    Ok(())
}

/// 处理 `restore` 命令
///
/// `yes` 为 `true` 时跳过 dry-run 确认提示，直接执行 restore。适用于
/// `gsb sync` 后台模式或 CI 脚本。
pub fn handle_restore(config: &Config, repo_root: &Path, yes: bool) -> Result<()> {
    info!("Starting restore process...");
    let device_name = utils::get_current_device_name()?;

    // --- dry-run 摘要 ---
    // 在真正写入之前，列出将要被 restore 的 item 及目标路径，
    // 让用户有机会检查是否有意外覆盖。
    let pending: Vec<(&crate::config::Item, std::path::PathBuf)> = config
        .items
        .iter()
        .filter(|item| !item.is_ignored_for_restore(&device_name, &config.aliases))
        .filter_map(|item| {
            item.get_source_for_device(&device_name, &config.aliases)
                .map(|p| (item, expand_tilde(p).fuck_backslash()))
        })
        .collect();

    if pending.is_empty() {
        info!("No items to restore on this device.");
        return Ok(());
    }

    println!(
        "The following {} item(s) will be restored on this device:",
        pending.len()
    );
    for (item, dest) in &pending {
        println!("  {}  ->  {}", item.path_in_repo, dest.display());
    }

    // 同时列出被跳过的 item（帮助用户发现配置遗漏）
    let skipped: Vec<&crate::config::Item> = config
        .items
        .iter()
        .filter(|item| item.is_ignored_for_restore(&device_name, &config.aliases))
        .collect();
    if !skipped.is_empty() {
        println!("\nSkipped ({} item(s)):", skipped.len());
        for item in &skipped {
            let reason = match item.restore {
                crate::config::RestorePolicy::Off => "restore = off".to_string(),
                crate::config::RestorePolicy::Explicit => "not in restore_devices".to_string(),
                crate::config::RestorePolicy::All => "ignored".to_string(),
            };
            println!("  {}  ({})", item.path_in_repo, reason);
        }
    }

    if !yes {
        print!("\nProceed? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            info!("Restore cancelled by user.");
            return Ok(());
        }
    }

    // 构建加密计划
    #[cfg(feature = "encrypt")]
    let active_paths: Vec<&str> = pending
        .iter()
        .map(|(item, _)| item.path_in_repo.as_str())
        .collect();
    #[cfg(feature = "encrypt")]
    let crypt_plan = CryptPlan::build(repo_root, &active_paths)?;

    // 有加密计划时，对可能含加密文件的项目使用 smart_restore_item
    // （逐个文件检查并决定解密或拷贝），其余项目普通拷贝。
    // 无加密计划时，所有项目普通拷贝。
    #[cfg(feature = "encrypt")]
    if let Some(plan) = crypt_plan {
        let key = plan.key.as_bytes().to_vec();
        pending
            .par_iter()
            .try_for_each(|(item, dest_path)| -> Result<()> {
                let source_path = repo_root.join(&item.path_in_repo).fuck_backslash();
                if crypt_plan::path_intersects_crypt_list(&item.path_in_repo, &plan.crypt_list) {
                    // 可能包含加密文件：智能解密/拷贝，忽略硬链接
                    smart_restore_item(&source_path, dest_path, &key)?;
                } else {
                    // 不含加密文件：普通拷贝
                    copy_item_all(&source_path, dest_path, item.is_hardlink)?;
                }
                Ok(())
            })?;
        info!("Restore  process finished.");
        return Ok(());
    }

    // 无加密计划：普通拷贝
    pending
        .par_iter()
        .try_for_each(|(item, dest_path)| -> Result<()> {
            let source_path = repo_root.join(&item.path_in_repo).fuck_backslash();
            copy_item_all(&source_path, dest_path, item.is_hardlink)?;
            Ok(())
        })?;

    info!("Restore  process finished.");
    Ok(())
}

/// 处理 `sync` 命令
pub fn handle_sync(config: &Config, repo_root: &Path) -> Result<()> {
    info!(
        "Starting sync process. Interval: {} seconds.",
        config.sync_interval
    );
    let repo = GsbRepo::open(repo_root)?;
    let sleep_duration = Duration::from_secs(config.sync_interval);

    loop {
        info!("Running sync cycle...");
        match repo.pull(
            config.git.remote.as_deref().unwrap_or("origin"),
            config.git.branch.as_deref().unwrap_or("main"),
        ) {
            Ok(()) => {
                info!("Pull successful, now restoring files...");
                if let Err(e) = handle_restore(config, repo_root, true) {
                    error!("Failed to restore after pull: {e}");
                }
            }
            Err(e) => {
                error!("Failed to pull from remote: {e}");
            }
        }

        info!("Sync cycle finished. Sleeping for {sleep_duration:?}...");
        thread::sleep(sleep_duration);
    }
}

// =========================================================================
// 加密支持（feature = "encrypt"）
// =========================================================================
// 当启用 encrypt feature 时，collect 完成后自动加密仓库中的文件，
// restore 开始前自动解密、完成后重新加密，保证仓库始终存储密文。
// 加密列表复用 git_simple_encrypt.toml 中的 crypt_list，与 git-se CLI
// 完全兼容。仅处理同时在 gsb items 和 crypt_list 中的 path_in_repo。

#[cfg(feature = "encrypt")]
mod crypt_plan {
    use std::path::{Path, PathBuf};

    use git_simple_encrypt::{crypt, repo::Repo};

    use crate::error::Result;

    /// 一次 collect/restore 的加解密计划。
    ///
    /// 持有 git-simple-encrypt 的 [`Repo`] 句柄、完整 `crypt_list、本次需要`
    /// 处理的仓库相对路径列表（已与 `crypt_list` 取交集）以及主密钥。
    pub struct CryptPlan {
        pub repo: Repo,
        pub paths: Vec<PathBuf>,
        pub crypt_list: Vec<String>,
        pub key: String,
    }

    impl CryptPlan {
        /// 根据本次活跃的 `path_in_repo` 列表构建加解密计划。
        ///
        /// 返回 `Ok(None)` 的情况：
        /// - `git_simple_encrypt.toml` 不存在或 `crypt_list` 为空
        /// - 活跃路径中没有与 `crypt_list` 匹配的项
        ///
        /// 当密钥缺失且交集非空时，将返回错误而非静默跳过，防止 collect
        /// 时未加密或 restore 时将密文原样写入本地。
        pub fn build(repo_root: &Path, active_paths: &[&str]) -> Result<Option<Self>> {
            let gse_repo = match Repo::open(repo_root) {
                Ok(r) => r,
                Err(e) => {
                    log::debug!("git-simple-encrypt repo open failed: {e}");
                    return Ok(None);
                }
            };

            if gse_repo.conf.crypt_list.is_empty() {
                return Ok(None);
            }

            // 取交集：active_paths ∩ crypt_list
            let paths: Vec<PathBuf> = active_paths
                .iter()
                .filter(|p| path_matches_crypt_list(p, &gse_repo.conf.crypt_list))
                .map(PathBuf::from)
                .collect();

            if paths.is_empty() {
                return Ok(None);
            }

            // 密钥必须存在，否则中断操作
            let key = gse_repo.get_key()?;

            log::info!("{} item(s) will be encrypted/decrypted.", paths.len());
            let crypt_list = paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            Ok(Some(Self {
                repo: gse_repo,
                paths,
                crypt_list,
                key,
            }))
        }

        /// 检查某一路径是否在此加解密计划中。
        pub fn contains(&self, path_in_repo: &str) -> bool {
            let path = Path::new(path_in_repo);
            self.paths.iter().any(|p| p == path)
        }

        /// 加密仓库中的文件（原地，幂等）。
        pub fn encrypt(&self) -> Result<()> {
            crypt::encrypt_repo(&self.repo, &self.paths)?;
            Ok(())
        }
    }

    /// 检查 `path_in_repo` 是否匹配 `crypt_list`。
    ///
    /// 匹配规则：
    /// - `path_in_repo` 直接等于 `crypt_list` 中的条目
    /// - `path_in_repo` 是 `crypt_list` 中某个目录条目的子路径
    ///
    /// 支持正斜杠 / 反斜杠。
    pub(crate) fn path_matches_crypt_list(path_in_repo: &str, crypt_list: &[String]) -> bool {
        crypt_list.iter().any(|c| {
            c == path_in_repo
                || path_in_repo.starts_with(&format!("{c}/"))
                || path_in_repo.starts_with(&format!("{c}\\"))
        })
    }

    /// 检查 `path_in_repo` 是否可能包含加密文件。
    ///
    /// 相比 [`path_matches_crypt_list`] 增加了反向匹配：如果
    /// `crypt_list` 中的条目在 `path_in_repo` 的子树中，该路径下
    /// 也可能存在加密文件。
    pub(crate) fn path_intersects_crypt_list(path_in_repo: &str, crypt_list: &[String]) -> bool {
        crypt_list.iter().any(|c| {
            c == path_in_repo
                || path_in_repo.starts_with(&format!("{c}/"))
                || path_in_repo.starts_with(&format!("{c}\\"))
                || c.starts_with(&format!("{path_in_repo}/"))
                || c.starts_with(&format!("{path_in_repo}\\"))
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_path_matches_exact() {
            assert!(path_matches_crypt_list("secrets", &["secrets".to_string()]));
            assert!(path_matches_crypt_list(
                "secrets/file.txt",
                &["secrets".to_string()]
            ));
            assert!(!path_matches_crypt_list(
                "secrets2",
                &["secrets".to_string()]
            ));
        }

        #[test]
        fn test_path_matches_backslash() {
            assert!(path_matches_crypt_list(
                "secrets\\file.txt",
                &["secrets".to_string()]
            ));
        }

        #[test]
        fn test_path_matches_empty_list() {
            assert!(!path_matches_crypt_list("secrets", &[]));
        }
    }
}

/// 智能恢复单个加密项（文件或目录）。
///
/// - 文件：若已加密则直接解密到目标路径，否则普通拷贝。
/// - 目录：递归处理子项，并在完成后清理目标目录中的孤儿文件。
///
/// 该函数替代了旧的 `decrypt_repo → copy → encrypt_repo` roundtrip，
/// 不再修改仓库中的文件。
#[cfg(feature = "encrypt")]
fn smart_restore_item(src: &Path, dst: &Path, key: &[u8]) -> Result<()> {
    use git_simple_encrypt::crypt::decrypt_file_to;

    if src.is_file() {
        if decrypt_file_to(src, dst, key)?.is_none() {
            // 未加密文件，普通拷贝
            copy_item(src, dst)?;
        }
    } else if src.is_dir() {
        if !dst.exists() {
            fs::create_dir(dst)?;
        }
        let mut src_names = HashSet::new();
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let name = entry.file_name();
            src_names.insert(name.clone());
            smart_restore_item(&entry.path(), &dst.join(&name), key)?;
        }
        // 清理目标目录中的孤儿文件
        for entry in fs::read_dir(dst)? {
            let entry = entry?;
            let name = entry.file_name();
            if !src_names.contains(&name) {
                let path = entry.path();
                if path.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
                debug!("Removed orphan {path:?} (no longer exists in repo)");
            }
        }
    }
    Ok(())
}

#[cfg(feature = "encrypt")]
pub use crypt_plan::CryptPlan;

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
            version: "0.2.1".to_string(),
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
                    ignore: vec![],
                    restore: crate::config::RestorePolicy::All,
                    restore_devices: vec![],
                },
                Item {
                    path_in_repo: "dir1".to_string(),
                    default_source: Some(work_dir.path().join("dir1")),
                    is_hardlink: false,
                    sources: None,
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                    ignore: vec![],
                    restore: crate::config::RestorePolicy::All,
                    restore_devices: vec![],
                },
                Item {
                    path_in_repo: "hardlink_file.txt".to_string(),
                    default_source: Some(work_dir.path().join("hardlink_source.txt")),
                    is_hardlink: true,
                    sources: None,
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                    ignore: vec![],
                    restore: crate::config::RestorePolicy::All,
                    restore_devices: vec![],
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
                    ignore: vec![],
                    restore: crate::config::RestorePolicy::All,
                    restore_devices: vec![],
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
        handle_collect(&config, repo_root, true).unwrap();

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
        assert!(is_same_file(&hardlink_source_path, &collected_hardlink_path).unwrap());
        assert_eq!(
            fs::read_to_string(&collected_hardlink_path).unwrap(),
            "content for hardlink"
        );

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

        // 在仓库中创建硬链接源文件
        let repo_hardlink_source_path = repo_root.join("hardlink_file.txt");
        File::create(&repo_hardlink_source_path)
            .unwrap()
            .write_all(b"content for hardlink in repo")
            .unwrap();

        // 运行 restore
        handle_restore(&config, repo_root, true).unwrap();

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

        // 验证硬链接文件是否已恢复到工作目录并是硬链接
        let work_hardlink_path = work_root.join("hardlink_source.txt"); // default_source
        assert!(work_hardlink_path.exists());
        assert!(is_same_file(&repo_hardlink_source_path, &work_hardlink_path).unwrap());
        assert_eq!(
            fs::read_to_string(&work_hardlink_path).unwrap(),
            "content for hardlink in repo"
        );
    }

    // 新增的 copy_item 测试
    #[test]
    fn test_copy_item() -> Result<()> {
        let temp_dir = tempdir()?;
        let from_path = temp_dir.path().join("source");
        let to_path = temp_dir.path().join("destination");

        // 场景 1: 拷贝文件 (目标不存在)
        let source_file_path = from_path.join("file.txt");
        let dest_file_path = to_path.join("file.txt");
        fs::create_dir_all(&from_path)?;
        File::create(&source_file_path)?.write_all(b"hello world")?;

        copy_item(&source_file_path, &dest_file_path)?;
        assert!(dest_file_path.exists());
        assert_eq!(fs::read_to_string(&dest_file_path)?, "hello world");

        // 场景 2: 拷贝目录 (目标不存在)
        let source_dir_path = from_path.join("my_dir");
        let dest_dir_path = to_path.join("my_dir");
        fs::create_dir(&source_dir_path)?;
        File::create(source_dir_path.join("inner_file.txt"))?.write_all(b"inner content")?;
        fs::create_dir(source_dir_path.join("sub_dir"))?;
        File::create(source_dir_path.join("sub_dir").join("sub_file.txt"))?
            .write_all(b"sub content")?;

        copy_item(&source_dir_path, &dest_dir_path)?;
        assert!(dest_dir_path.exists());
        assert!(dest_dir_path.is_dir());
        assert!(dest_dir_path.join("inner_file.txt").exists());
        assert_eq!(
            fs::read_to_string(dest_dir_path.join("inner_file.txt"))?,
            "inner content"
        );
        assert!(dest_dir_path.join("sub_dir").exists());
        assert!(dest_dir_path.join("sub_dir").is_dir());
        assert!(dest_dir_path.join("sub_dir").join("sub_file.txt").exists());
        assert_eq!(
            fs::read_to_string(dest_dir_path.join("sub_dir").join("sub_file.txt"))?,
            "sub content"
        );

        // 场景 3: 目标路径的父目录不存在，应自动创建
        let new_dest_parent = temp_dir.path().join("new_parent");
        let new_dest_file = new_dest_parent.join("new_file.txt");
        File::create(&source_file_path)?.write_all(b"content for new parent")?;
        copy_item(&source_file_path, &new_dest_file)?;
        assert!(new_dest_parent.exists());
        assert!(new_dest_file.exists());
        assert_eq!(
            fs::read_to_string(&new_dest_file)?,
            "content for new parent"
        );

        // 场景 4: 目标目录中有源已不存在的文件，应被删除
        let source_cleanup_dir = from_path.join("cleanup_src");
        let dest_cleanup_dir = to_path.join("cleanup_dst");
        fs::create_dir(&source_cleanup_dir)?;
        File::create(source_cleanup_dir.join("keep.txt"))?.write_all(b"keep")?;

        // 先复制一次（建立 dest）
        copy_item(&source_cleanup_dir, &dest_cleanup_dir)?;
        assert!(dest_cleanup_dir.join("keep.txt").exists());

        // 在 dest 中额外创建一个文件（模拟源中已删除的文件）
        File::create(dest_cleanup_dir.join("orphan.txt"))?.write_all(b"orphan")?;
        assert!(dest_cleanup_dir.join("orphan.txt").exists());

        // 再次同步（此时 orphan.txt 不存在于源中）
        copy_item(&source_cleanup_dir, &dest_cleanup_dir)?;

        // keep.txt 应保留，orphan.txt 应被删除
        assert!(dest_cleanup_dir.join("keep.txt").exists());
        assert!(
            !dest_cleanup_dir.join("orphan.txt").exists(),
            "orphan.txt should have been deleted during sync"
        );

        // 场景 5: 目标目录中有源已不存在的子目录，应被递归删除
        fs::create_dir(dest_cleanup_dir.join("orphan_dir"))?;
        File::create(dest_cleanup_dir.join("orphan_dir").join("nested.txt"))?
            .write_all(b"nested")?;
        assert!(dest_cleanup_dir.join("orphan_dir").exists());

        copy_item(&source_cleanup_dir, &dest_cleanup_dir)?;
        assert!(
            !dest_cleanup_dir.join("orphan_dir").exists(),
            "orphan_dir should have been deleted during sync"
        );

        Ok(())
    }

    #[test]
    fn test_copy_item_all() -> Result<()> {
        let temp_dir = tempdir()?;
        let from_path = temp_dir.path().join("source");
        let to_path = temp_dir.path().join("destination");

        fs::create_dir_all(&from_path)?;
        fs::create_dir_all(&to_path)?;

        // 场景 1: 硬链接文件 - 源文件存在，目标文件不存在
        let source_file_hardlink = from_path.join("hardlink_source.txt");
        let dest_file_hardlink = to_path.join("hardlink_dest.txt");
        File::create(&source_file_hardlink)?.write_all(b"hardlink content")?;

        copy_item_all(&source_file_hardlink, &dest_file_hardlink, true)?;
        assert!(dest_file_hardlink.exists());
        assert!(is_same_file(&source_file_hardlink, &dest_file_hardlink)?);
        assert_eq!(fs::read_to_string(&dest_file_hardlink)?, "hardlink content");

        // 场景 2: 硬链接文件 - 源文件存在，目标文件存在且内容不同
        let source_file_hardlink_2 = from_path.join("hardlink_source_2.txt");
        let dest_file_hardlink_2 = to_path.join("hardlink_dest_2.txt");
        File::create(&source_file_hardlink_2)?.write_all(b"hardlink content 2")?;
        File::create(&dest_file_hardlink_2)?.write_all(b"old content")?; // 目标文件已存在

        copy_item_all(&source_file_hardlink_2, &dest_file_hardlink_2, true)?;
        assert!(dest_file_hardlink_2.exists());
        assert!(is_same_file(
            &source_file_hardlink_2,
            &dest_file_hardlink_2
        )?);
        assert_eq!(
            fs::read_to_string(&dest_file_hardlink_2)?,
            "hardlink content 2"
        );

        // 场景 3: 硬链接文件 - 源文件存在，目标文件已是硬链接
        let source_file_hardlink_3 = from_path.join("hardlink_source_3.txt");
        let dest_file_hardlink_3 = to_path.join("hardlink_dest_3.txt");
        File::create(&source_file_hardlink_3)?.write_all(b"hardlink content 3")?;
        fs::hard_link(&source_file_hardlink_3, &dest_file_hardlink_3)?; // 预先创建硬链接

        copy_item_all(&source_file_hardlink_3, &dest_file_hardlink_3, true)?;
        assert!(dest_file_hardlink_3.exists());
        assert!(is_same_file(
            &source_file_hardlink_3,
            &dest_file_hardlink_3
        )?);
        assert_eq!(
            fs::read_to_string(&dest_file_hardlink_3)?,
            "hardlink content 3"
        );

        // 场景 4: 非硬链接文件 - 源文件存在，目标文件不存在
        let source_file_copy = from_path.join("copy_source.txt");
        let dest_file_copy = to_path.join("copy_dest.txt");
        File::create(&source_file_copy)?.write_all(b"copy content")?;

        copy_item_all(&source_file_copy, &dest_file_copy, false)?;
        assert!(dest_file_copy.exists());
        assert!(!is_same_file(&source_file_copy, &dest_file_copy)?); // 应该不是硬链接
        assert_eq!(fs::read_to_string(&dest_file_copy)?, "copy content");

        // 场景 5: 非硬链接文件 - 源文件存在，目标文件存在且内容不同
        let source_file_copy_2 = from_path.join("copy_source_2.txt");
        let dest_file_copy_2 = to_path.join("copy_dest_2.txt");
        File::create(&source_file_copy_2)?.write_all(b"copy content 2")?;
        File::create(&dest_file_copy_2)?.write_all(b"old copy content")?;

        copy_item_all(&source_file_copy_2, &dest_file_copy_2, false)?;
        assert!(dest_file_copy_2.exists());
        assert_eq!(fs::read_to_string(&dest_file_copy_2)?, "copy content 2");

        // 场景 6: 源路径不存在
        let non_existent_source = from_path.join("non_existent.txt");
        let dummy_dest = to_path.join("dummy.txt");
        let result = copy_item_all(&non_existent_source, &dummy_dest, false);
        assert!(result.is_ok()); // 应该返回 Ok(()) 但不执行操作
        assert!(!dummy_dest.exists()); // 目标文件不应该被创建

        // 场景 7: 对目录尝试硬链接
        let source_dir_hardlink = from_path.join("dir_source");
        let dest_dir_hardlink = to_path.join("dir_dest");
        fs::create_dir(&source_dir_hardlink)?;

        let result = copy_item_all(&source_dir_hardlink, &dest_dir_hardlink, true);
        assert!(result.is_ok()); // 应该返回 Ok(()) 但不执行操作
        assert!(!dest_dir_hardlink.exists()); // 目标目录不应该被创建为硬链接

        Ok(())
    }
}
