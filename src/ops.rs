//! collect / restore / sync 三大操作的核心实现。
//!
//! 与上一版相比：
//! - 错误统一使用 [`anyhow::Result`]，并在关键位置用 `.context(...)`
//!   附加上下文；
//! - 配置语义基于新的 `ops` + 设备表，旧的 `ignore_*` / `restore_*`
//!   字段已移除；
//! - 新增 `--interactive` / `-i` 选项：collect / restore 时对每个 item 询问
//!   `y/n/a/q`，方便细粒度控制。

use std::{
    collections::HashSet,
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, UNIX_EPOCH},
};

use fuck_backslash::FuckBackslash;
use log::{debug, error, info, trace, warn};
use rayon::prelude::*;
use same_file::is_same_file;

use crate::{
    config::{Config, Item, Op},
    error::{Context, Result},
    git::GsbRepo,
    vars::Vars,
};

// =========================================================================
// 交互式提示
// =========================================================================

/// 交互模式控制。
///
/// 当 `yes_to_all == true` 时（默认或用户在交互中选择 `a`），所有询问直接
/// 返回「是」，等价于非交互模式。
#[derive(Debug)]
pub struct Prompt {
    yes_to_all: bool,
}

impl Prompt {
    /// `interactive == true` 时初始为「逐项询问」，否则初始即「全部 yes」。
    pub fn new(interactive: bool) -> Self {
        Self {
            yes_to_all: !interactive,
        }
    }

    /// 询问用户是否处理某项。
    ///
    /// 返回 `Ok(true)` 表示处理，`Ok(false)` 表示跳过；当用户选择 `quit`
    /// 时返回 `Err`，调用方据此中止整个流程。
    pub fn ask(&mut self, prompt: &str) -> Result<bool> {
        if self.yes_to_all {
            return Ok(true);
        }

        let stdin = io::stdin();
        let mut stdout = io::stdout();
        loop {
            write!(stdout, "{prompt} [y/n/a/q/?] ")?;
            stdout.flush()?;
            let mut line = String::new();
            stdin.lock().read_line(&mut line)?;
            match line.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => return Ok(true),
                "n" | "no" | "" => return Ok(false),
                "a" | "all" => {
                    self.yes_to_all = true;
                    return Ok(true);
                }
                "q" | "quit" => {
                    return Err(anyhow::anyhow!("Operation cancelled by user"));
                }
                _ => {
                    writeln!(stdout, "  y=yes / n=no / a=all / q=quit")?;
                }
            }
        }
    }
}

/// 在 collect/restore 的 item 列表展示后，询问用户是否继续。
///
/// Y 或 Enter = 继续；n = 中止。
fn confirm_or_abort() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        write!(stdout, "Proceed? [Y/n] ")?;
        stdout.flush()?;
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" | "" => return Ok(()),
            "n" | "no" => {
                return Err(anyhow::anyhow!("Operation cancelled by user"));
            }
            _ => {
                writeln!(stdout, "  y=yes / n=no")?;
            }
        }
    }
}

// =========================================================================
// 文件拷贝核心
// =========================================================================

/// 逐字节比较两个文件的内容是否相等。
///
/// 仅在文件大小相同但修改时间不可靠时作为备用检查方法。使用 8KB 缓冲区。
fn are_contents_equal(path1: &Path, path2: &Path) -> Result<bool> {
    let mut f1 = BufReader::new(
        fs::File::open(path1).with_context(|| format!("Failed to open file: {path1:?}"))?,
    );
    let mut f2 = BufReader::new(
        fs::File::open(path2).with_context(|| format!("Failed to open file: {path2:?}"))?,
    );

    let mut buf1 = [0u8; 8192];
    let mut buf2 = [0u8; 8192];
    loop {
        let n1 = f1
            .read(&mut buf1)
            .with_context(|| format!("Failed to read file: {path1:?}"))?;
        let n2 = f2
            .read(&mut buf2)
            .with_context(|| format!("Failed to read file: {path2:?}"))?;
        if n1 != n2 {
            return Ok(false);
        }
        if n1 == 0 {
            return Ok(true);
        }
        if buf1[..n1] != buf2[..n2] {
            return Ok(false);
        }
    }
}

/// 智能拷贝文件或目录，仅在必要时执行 I/O。
///
/// - 目录：递归同步（含删除目标侧孤儿文件）
/// - 文件：先比较大小/修改时间，必要时再拷贝；若修改时间不可用则回退到
///   逐字节比较。
fn copy_item(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {parent:?}"))?;
    }

    if from.is_dir() {
        if !to.exists() {
            fs::create_dir(to).with_context(|| format!("Failed to create directory: {to:?}"))?;
        }

        let mut src_names = HashSet::new();
        for entry in
            fs::read_dir(from).with_context(|| format!("Failed to read directory: {from:?}"))?
        {
            let entry =
                entry.with_context(|| format!("Failed to read directory entry: {from:?}"))?;
            let file_name = entry.file_name();
            src_names.insert(file_name.clone());
            let dest_path = to.join(&file_name).fuck_backslash();
            copy_item(&entry.path(), &dest_path)?;
        }

        for entry in
            fs::read_dir(to).with_context(|| format!("Failed to read directory: {to:?}"))?
        {
            let entry = entry.with_context(|| format!("Failed to read directory entry: {to:?}"))?;
            let file_name = entry.file_name();
            if !src_names.contains(&file_name) {
                let dest_path = entry.path();
                if dest_path.is_dir() {
                    fs::remove_dir_all(&dest_path)
                        .with_context(|| format!("Failed to remove directory: {dest_path:?}"))?;
                } else {
                    fs::remove_file(&dest_path)
                        .with_context(|| format!("Failed to remove file: {dest_path:?}"))?;
                }
                debug!("Removed {dest_path:?} (no longer exists in source)");
            }
        }
        return Ok(());
    }

    // --- 文件拷贝 ---
    let mut should_copy = true;
    if to.exists() {
        let from_meta =
            fs::metadata(from).with_context(|| format!("Failed to read metadata: {from:?}"))?;
        let to_meta =
            fs::metadata(to).with_context(|| format!("Failed to read metadata: {to:?}"))?;

        if from_meta.len() == to_meta.len() {
            if let (Ok(from_time), Ok(to_time)) = (from_meta.modified(), to_meta.modified()) {
                if from_time.duration_since(UNIX_EPOCH).unwrap().as_secs()
                    == to_time.duration_since(UNIX_EPOCH).unwrap().as_secs()
                {
                    should_copy = false;
                }
            } else {
                warn!(
                    "Could not read modification time for {from:?} or {to:?}. Falling back to byte-by-byte comparison."
                );
                if are_contents_equal(from, to)? {
                    should_copy = false;
                }
            }
        }
    }

    if should_copy {
        debug!("Copying  file: {from:?} -> {to:?}");
        fs::copy(from, to).with_context(|| format!("Failed to copy file: {from:?} -> {to:?}"))?;
    } else {
        trace!("Skipping unchanged file: {from:?}");
    }

    Ok(())
}

/// 智能复制文件或目录，支持硬链接。
///
/// 错误处理策略：
/// - 源路径不存在：记录日志但返回 `Ok`（与历史行为一致，避免一个 item 失败
///   中断整批同步）
/// - `is_hardlink == true` 但 `from` 不是文件：记录 warning，忽略 `is_hardlink`
///   并回退到普通复制
/// - 其它 I/O 错误：返回 `Err` 并附带上下文
fn copy_item_all(from: &Path, to: &Path, is_hardlink: bool) -> Result<()> {
    if !from.exists() {
        error!("Source path does not exist, skipping copy: {from:?}");
        return Ok(());
    }
    if is_hardlink {
        if !from.is_file() {
            warn!(
                "Source path is not a file, ignoring is_hardlink and falling back to copy: {from:?}"
            );
            copy_item(from, to)?;
        } else if to.exists() && is_same_file(from, to)? {
            warn!("Skipping hardlink copy: {from:?} -> {to:?}");
        } else {
            info!("Hardlink {from:?} -> {to:?}");
            _ = fs::remove_file(to); // 尝试删除目标，忽略错误（可能本不存在）
            fs::hard_link(from, to)
                .with_context(|| format!("Failed to create hard link: {from:?} -> {to:?}"))?;
        }
    } else {
        copy_item(from, to)?;
    }
    Ok(())
}

// =========================================================================
// item 解析
// =========================================================================

/// 在当前配置下，对每个 item 解析出「目标操作 + source 路径」。
///
/// `op` 决定我们要 collect 还是 restore。`local_path` 是本地侧绝对路径
/// （已展开变量、`~`、`fuck_backslash`）；`repo_path` 是仓库侧绝对路径
/// （已与 `repo_root` 拼接）。
#[derive(Debug)]
struct Plan {
    /// `path_in_repo` 字段，用于日志和加密路径匹配。
    path_in_repo: String,
    /// 仓库侧绝对路径。
    repo_path: PathBuf,
    /// 本地侧绝对路径。
    local_path: PathBuf,
    /// 是否硬链接（仅文件有效）。
    is_hardlink: bool,
}

/// 收集所有需要执行 op 的 items，并对每个 item 解析路径。
fn plan_items(config: &Config, repo_root: &Path, op: Op, vars: &Vars) -> Result<Vec<Plan>> {
    let device_id = vars.device();
    let mut plans = Vec::with_capacity(config.items.len());

    for item in &config.items {
        let ops = item.effective_ops(device_id, &config.aliases);
        if !ops.contains(op) {
            trace!(
                "Skip {} for {:?}: ops={:?} on device {}",
                op,
                item.path_in_repo,
                ops.as_slice(),
                device_id
            );
            continue;
        }

        let raw_source = item
            .effective_source(device_id, &config.aliases)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "item {:?} has no source path configured on device ({})",
                    item.path_in_repo,
                    device_id
                )
            })?;
        let local_path = vars
            .expand_path(&raw_source)
            .with_context(|| {
                format!(
                    "Failed to expand source path for item {:?}",
                    item.path_in_repo
                )
            })?
            .fuck_backslash();
        let repo_path = item
            .resolve_repo_relative(repo_root)
            .with_context(|| {
                format!(
                    "Failed to resolve repo path for item {:?}",
                    item.path_in_repo
                )
            })?
            .fuck_backslash();

        plans.push(Plan {
            path_in_repo: item.path_in_repo.clone(),
            repo_path,
            local_path,
            is_hardlink: item.is_hardlink,
        });
    }

    Ok(plans)
}

// =========================================================================
// collect / restore / sync
// =========================================================================

/// 处理 `collect` 命令。
///
/// `interactive == true` 时，对每个 item 询问 y/n/a/q。
pub fn handle_collect(
    config: &Config,
    repo_root: &Path,
    autocommit: bool,
    interactive: bool,
) -> Result<()> {
    info!("Starting collection process...");
    let vars = Vars::build(config, crate::vars::current_device()?, repo_root)?;
    let repo = GsbRepo::open(repo_root).context("Failed to open git repository")?;

    let plans = plan_items(config, repo_root, Op::Collect, &vars)?;
    if plans.is_empty() {
        info!("No items to collect on this device.");
        return Ok(());
    }

    println!("The following {} item(s) will be collected:", plans.len());
    for p in &plans {
        println!("  {}  <-  {}", p.path_in_repo, p.local_path.display());
    }

    confirm_or_abort()?;
    let accepted = prompt_filter(plans.iter(), "collect", interactive)?;

    if accepted.is_empty() {
        info!("No items selected to collect.");
        return Ok(());
    }

    #[cfg(feature = "encrypt")]
    let active_paths: Vec<&str> = accepted.iter().map(|p| p.path_in_repo.as_str()).collect();
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

    accepted.par_iter().try_for_each(|p| -> Result<()> {
        let path_in_repo = &p.path_in_repo;
        // 加密项不允许硬链接（密文与明文内容不同）
        let use_hardlink = if encrypted_paths.contains(path_in_repo) {
            false
        } else {
            p.is_hardlink
        };
        copy_item_all(&p.local_path, &p.repo_path, use_hardlink)
            .with_context(|| format!("Failed to collect item {path_in_repo:?}"))
    })?;

    #[cfg(feature = "encrypt")]
    if let Some(plan) = &crypt_plan {
        info!("Encrypting collected files...");
        plan.encrypt().context("Encryption failed")?;
    }

    info!("Collection process finished.");
    if autocommit {
        let timestamp = chrono::Local::now();
        let commit_message = format!(
            "gsb collect on {} at {}",
            vars.device(),
            timestamp.format("%Y-%m-%d %H:%M:%S")
        );
        repo.add_and_commit(&commit_message)
            .context("git commit failed")?;
    }
    Ok(())
}

/// 在交互模式下逐项询问用户；非交互模式下保留全部。
///
/// 用户选择 `quit` 时立即返回错误，调用方据此中止整个流程。
fn prompt_filter<'a, I>(iter: I, verb: &str, interactive: bool) -> Result<Vec<&'a Plan>>
where
    I: IntoIterator<Item = &'a Plan>,
{
    let mut prompt = Prompt::new(interactive);
    let mut out = Vec::new();
    for p in iter {
        let yes = prompt
            .ask(&format!(
                "{verb} '{}' ({})?",
                p.path_in_repo,
                p.local_path.display()
            ))
            .context("Failed to read user input")?;
        if yes {
            out.push(p);
        }
    }
    Ok(out)
}

/// 处理 `restore` 命令。
///
/// `interactive == true` 时，对每个 item 询问 y/n/a/q。
/// `gsb sync` 调用时直接传 `false`，等价于 `gsb r -y`。
pub fn handle_restore(config: &Config, repo_root: &Path, interactive: bool) -> Result<()> {
    info!("Starting restore process...");
    let vars = Vars::build(config, crate::vars::current_device()?, repo_root)?;

    let plans = plan_items(config, repo_root, Op::Restore, &vars)?;
    if plans.is_empty() {
        info!("No items to restore on this device.");
        return Ok(());
    }

    println!("The following {} item(s) will be restored:", plans.len());
    for p in &plans {
        println!("  {}  ->  {}", p.path_in_repo, p.local_path.display());
    }

    // 收集跳过的 item 数（信息性）
    let skipped: Vec<&Item> = config
        .items
        .iter()
        .filter(|item| {
            !item
                .effective_ops(vars.device(), &config.aliases)
                .contains(Op::Restore)
        })
        .collect();
    if !skipped.is_empty() {
        println!("\nSkipped ({}):", skipped.len());
        for item in &skipped {
            println!("  {}  (ops does not include restore)", item.path_in_repo);
        }
    }

    confirm_or_abort()?;

    let accepted = prompt_filter(plans.iter(), "restore", interactive)?;
    if accepted.is_empty() {
        info!("No items selected to restore.");
        return Ok(());
    }

    #[cfg(feature = "encrypt")]
    let active_paths: Vec<&str> = accepted.iter().map(|p| p.path_in_repo.as_str()).collect();
    #[cfg(feature = "encrypt")]
    let crypt_plan = CryptPlan::build(repo_root, &active_paths)?;

    #[cfg(feature = "encrypt")]
    if let Some(plan) = crypt_plan {
        let key = plan.key.as_bytes().to_vec();
        accepted.par_iter().try_for_each(|p| -> Result<()> {
            let path_in_repo = &p.path_in_repo;
            if crypt_plan::path_intersects_crypt_list(path_in_repo, &plan.crypt_list) {
                smart_restore_item(&p.repo_path, &p.local_path, &key)
            } else {
                copy_item_all(&p.repo_path, &p.local_path, p.is_hardlink)
            }
            .with_context(|| format!("Failed to restore item {path_in_repo:?}"))
        })?;
        info!("Restore process finished.");
        return Ok(());
    }

    accepted.par_iter().try_for_each(|p| -> Result<()> {
        let path_in_repo = &p.path_in_repo;
        copy_item_all(&p.repo_path, &p.local_path, p.is_hardlink)
            .with_context(|| format!("Failed to restore item {path_in_repo:?}"))
    })?;

    info!("Restore  process finished.");
    Ok(())
}

/// 处理 `sync` 命令：循环 fetch + restore。
pub fn handle_sync(config: &Config, repo_root: &Path) -> Result<()> {
    info!(
        "Starting sync process. Interval: {} seconds.",
        config.sync_interval
    );
    let repo = GsbRepo::open(repo_root).context("Failed to open git repository")?;
    let sleep_duration = Duration::from_secs(config.sync_interval);

    loop {
        info!("Running sync cycle...");
        match repo.pull(
            config.git.remote.as_deref().unwrap_or("origin"),
            config.git.branch.as_deref().unwrap_or("main"),
        ) {
            Ok(()) => {
                info!("Pull successful, now restoring files...");
                if let Err(e) = handle_restore(config, repo_root, false) {
                    error!("Failed to restore after pull: {e:#}");
                }
            }
            Err(e) => {
                error!("Failed to pull from remote: {e:#}");
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

    use crate::error::{Context, Result};

    /// 一次 collect/restore 的加解密计划。
    ///
    /// 持有 git-simple-encrypt 的 [`Repo`] 句柄、完整 `crypt_list`、本次需要
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

            let paths: Vec<PathBuf> = active_paths
                .iter()
                .filter(|p| path_matches_crypt_list(p, &gse_repo.conf.crypt_list))
                .map(PathBuf::from)
                .collect();

            if paths.is_empty() {
                return Ok(None);
            }

            let key = gse_repo.get_key().context(
                "git-simple-encrypt key not configured, but crypt_list has matching items",
            )?;

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

        /// 加密仓库中的文件（原地，幂等）。
        pub fn encrypt(&self) -> Result<()> {
            crypt::encrypt_repo(&self.repo, &self.paths).context("Encryption failed")?;
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
        if decrypt_file_to(src, dst, key)
            .with_context(|| format!("Failed to decrypt: {src:?} -> {dst:?}"))?
            .is_none()
        {
            copy_item(src, dst)?;
        }
    } else if src.is_dir() {
        if !dst.exists() {
            fs::create_dir(dst).with_context(|| format!("Failed to create directory: {dst:?}"))?;
        }
        let mut src_names = HashSet::new();
        for entry in
            fs::read_dir(src).with_context(|| format!("Failed to read directory: {src:?}"))?
        {
            let entry =
                entry.with_context(|| format!("Failed to read directory entry: {src:?}"))?;
            let name = entry.file_name();
            src_names.insert(name.clone());
            smart_restore_item(&entry.path(), &dst.join(&name), key)?;
        }
        for entry in
            fs::read_dir(dst).with_context(|| format!("Failed to read directory: {dst:?}"))?
        {
            let entry =
                entry.with_context(|| format!("Failed to read directory entry: {dst:?}"))?;
            let name = entry.file_name();
            if !src_names.contains(&name) {
                let path = entry.path();
                if path.is_dir() {
                    fs::remove_dir_all(&path)
                        .with_context(|| format!("Failed to remove directory: {path:?}"))?;
                } else {
                    fs::remove_file(&path)
                        .with_context(|| format!("Failed to remove file: {path:?}"))?;
                }
                debug!("Removed orphan {path:?} (no longer exists in repo)");
            }
        }
    }
    Ok(())
}

#[cfg(feature = "encrypt")]
pub use crypt_plan::CryptPlan;

// =========================================================================
// 单元测试
// =========================================================================

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs::{self, File},
        io::Write,
    };

    use tempfile::tempdir;

    use super::*;
    use crate::{
        config::{Config, DeviceOverride, GitConfig, Item, Ops},
        vars::Vars,
    };

    /// 构造一个最小的可执行 `Plan` 上下文：`vars` + `repo_root`。
    fn build_ctx(device_id: &str, repo_root: &Path) -> Vars {
        let cfg = Config {
            version: "0.0".into(),
            sync_interval: 3600,
            git: GitConfig::default(),
            aliases: HashMap::new(),
            vars: HashMap::new(),
            items: vec![],
        };
        Vars::build(&cfg, device_id.to_string(), repo_root).unwrap()
    }

    fn make_config(device_id: &str, items: Vec<Item>) -> (Config, HashMap<String, String>) {
        let cfg = Config {
            version: "0.5".into(),
            sync_interval: 3600,
            git: GitConfig::default(),
            aliases: HashMap::new(),
            vars: HashMap::new(),
            items,
        };
        (
            cfg,
            HashMap::from([(device_id.to_string(), device_id.to_string())]),
        )
    }

    // === copy_item / copy_item_all 单测 ===

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
        assert_eq!(fs::read_to_string(&dest_file_path)?, "hello world");

        // 场景 2: 拷贝目录 (递归)
        let source_dir_path = from_path.join("my_dir");
        let dest_dir_path = to_path.join("my_dir");
        fs::create_dir(&source_dir_path)?;
        File::create(source_dir_path.join("inner_file.txt"))?.write_all(b"inner content")?;
        fs::create_dir(source_dir_path.join("sub_dir"))?;
        File::create(source_dir_path.join("sub_dir").join("sub_file.txt"))?
            .write_all(b"sub content")?;
        copy_item(&source_dir_path, &dest_dir_path)?;
        assert_eq!(
            fs::read_to_string(dest_dir_path.join("inner_file.txt"))?,
            "inner content"
        );
        assert_eq!(
            fs::read_to_string(dest_dir_path.join("sub_dir").join("sub_file.txt"))?,
            "sub content"
        );

        // 场景 3: 父目录不存在应自动创建
        let new_dest_parent = temp_dir.path().join("new_parent");
        let new_dest_file = new_dest_parent.join("new_file.txt");
        File::create(&source_file_path)?.write_all(b"content for new parent")?;
        copy_item(&source_file_path, &new_dest_file)?;
        assert_eq!(
            fs::read_to_string(&new_dest_file)?,
            "content for new parent"
        );

        // 场景 4: 目标中存在源中已删除的文件应被清理
        let src_cleanup = from_path.join("cleanup_src");
        let dst_cleanup = to_path.join("cleanup_dst");
        fs::create_dir(&src_cleanup)?;
        File::create(src_cleanup.join("keep.txt"))?.write_all(b"keep")?;
        copy_item(&src_cleanup, &dst_cleanup)?;
        File::create(dst_cleanup.join("orphan.txt"))?.write_all(b"orphan")?;
        copy_item(&src_cleanup, &dst_cleanup)?;
        assert!(dst_cleanup.join("keep.txt").exists());
        assert!(!dst_cleanup.join("orphan.txt").exists());

        // 场景 5: 孤儿目录递归删除
        fs::create_dir(dst_cleanup.join("orphan_dir"))?;
        File::create(dst_cleanup.join("orphan_dir").join("nested.txt"))?.write_all(b"nested")?;
        copy_item(&src_cleanup, &dst_cleanup)?;
        assert!(!dst_cleanup.join("orphan_dir").exists());

        Ok(())
    }

    #[test]
    fn test_copy_item_all_hardlink_and_missing() -> Result<()> {
        let temp_dir = tempdir()?;
        let from_path = temp_dir.path().join("source");
        let to_path = temp_dir.path().join("destination");
        fs::create_dir_all(&from_path)?;
        fs::create_dir_all(&to_path)?;

        // 硬链接文件 - 目标不存在
        let source_file = from_path.join("hardlink_source.txt");
        let dest_file = to_path.join("hardlink_dest.txt");
        File::create(&source_file)?.write_all(b"hardlink content")?;
        copy_item_all(&source_file, &dest_file, true)?;
        assert!(is_same_file(&source_file, &dest_file)?);

        // 源不存在：返回 Ok 但不做事
        let result = copy_item_all(&from_path.join("missing"), &to_path.join("x"), false);
        assert!(result.is_ok());
        assert!(!to_path.join("x").exists());

        // 目录硬链接：忽略 is_hardlink，回退到复制
        let src_dir = from_path.join("dir");
        fs::create_dir(&src_dir)?;
        File::create(src_dir.join("nested.txt"))?.write_all(b"nested content")?;
        let result = copy_item_all(&src_dir, &to_path.join("dir_dest"), true);
        assert!(result.is_ok());
        assert!(to_path.join("dir_dest").exists());
        assert!(to_path.join("dir_dest").join("nested.txt").exists());

        Ok(())
    }

    #[test]
    fn test_copy_item_all_directory_with_hardlink_flag_copies_fallback() -> Result<()> {
        let temp_dir = tempdir()?;
        let from_path = temp_dir.path().join("source");
        let to_path = temp_dir.path().join("destination");
        fs::create_dir_all(&from_path)?;

        let src_dir = from_path.join("cfg");
        fs::create_dir(&src_dir)?;
        File::create(src_dir.join("settings.toml"))?.write_all(b"config = 1")?;

        // is_hardlink=true + 目录 → 应回退到普通复制
        let result = copy_item_all(&src_dir, &to_path.join("cfg"), true);
        assert!(result.is_ok());
        assert!(to_path.join("cfg").join("settings.toml").exists());
        assert_eq!(
            fs::read_to_string(to_path.join("cfg").join("settings.toml"))?,
            "config = 1"
        );

        Ok(())
    }

    // === plan_items 单测 ===

    #[test]
    fn plan_items_respects_ops_filter() -> Result<()> {
        let device_id = "device-test";
        let items = vec![
            Item {
                path_in_repo: "collect_only".into(),
                source: Some(PathBuf::from("/a")),
                is_hardlink: false,
                ops: Ops::new([Op::Collect]),
                devices: HashMap::new(),
            },
            Item {
                path_in_repo: "restore_only".into(),
                source: Some(PathBuf::from("/b")),
                is_hardlink: false,
                ops: Ops::new([Op::Restore]),
                devices: HashMap::new(),
            },
            Item {
                path_in_repo: "skip".into(),
                source: Some(PathBuf::from("/c")),
                is_hardlink: false,
                ops: Ops::new([]),
                devices: HashMap::new(),
            },
        ];
        let (cfg, _) = make_config(device_id, items);
        let vars = build_ctx(device_id, Path::new("/repo"));

        let collect_plans = plan_items(&cfg, Path::new("/repo"), Op::Collect, &vars)?;
        assert_eq!(collect_plans.len(), 1);
        assert_eq!(collect_plans[0].path_in_repo, "collect_only");

        let restore_plans = plan_items(&cfg, Path::new("/repo"), Op::Restore, &vars)?;
        assert_eq!(restore_plans.len(), 1);
        assert_eq!(restore_plans[0].path_in_repo, "restore_only");

        Ok(())
    }

    #[test]
    fn plan_items_uses_device_override_ops() -> Result<()> {
        let device_id = "device-xyz";
        let mut devices = HashMap::new();
        devices.insert(
            device_id.to_string(),
            DeviceOverride {
                source: Some(PathBuf::from("/override")),
                ops: Some(Ops::new([Op::Collect])),
            },
        );
        let items = vec![Item {
            path_in_repo: "x".into(),
            source: Some(PathBuf::from("/default")),
            is_hardlink: false,
            ops: Ops::default(),
            devices,
        }];
        let (cfg, _) = make_config(device_id, items);
        let vars = build_ctx(device_id, Path::new("/repo"));

        // 该设备只 collect，不 restore
        let collect_plans = plan_items(&cfg, Path::new("/repo"), Op::Collect, &vars)?;
        assert_eq!(collect_plans.len(), 1);
        let restore_plans = plan_items(&cfg, Path::new("/repo"), Op::Restore, &vars)?;
        assert_eq!(restore_plans.len(), 0);
        // 路径用 override
        assert_eq!(collect_plans[0].local_path, PathBuf::from("/override"));
        Ok(())
    }

    #[test]
    fn plan_items_errors_when_no_source_for_device() {
        let device_id = "device-xyz";
        let items = vec![Item {
            path_in_repo: "x".into(),
            source: None,
            is_hardlink: false,
            ops: Ops::default(),
            devices: HashMap::new(),
        }];
        let (cfg, _) = make_config(device_id, items);
        let vars = build_ctx(device_id, Path::new("/repo"));
        let result = plan_items(&cfg, Path::new("/repo"), Op::Collect, &vars);
        assert!(result.is_err());
        let err = result.unwrap_err();
        // 错误信息应包含 path_in_repo 与设备 id
        let msg = format!("{err:#}");
        assert!(msg.contains('x'), "msg = {msg}");
    }

    // === handle_collect / handle_restore 集成 ===

    fn init_repo(path: &Path) {
        let repo = git2::Repository::init(path).expect("Failed to initialize git repository");
        let mut index = repo.index().unwrap();
        let oid = index.write_tree().unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            "Initial commit",
            &repo.find_tree(oid).unwrap(),
            &[],
        )
        .unwrap();
    }

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        File::create(path).unwrap().write_all(content).unwrap();
    }

    #[test]
    fn collect_and_restore_roundtrip() -> Result<()> {
        let device_id = crate::utils::get_current_device_name()?;
        let repo_dir = tempdir()?;
        let work_dir = tempdir()?;
        init_repo(repo_dir.path());

        write_file(&work_dir.path().join("a.txt"), b"hello");

        let items = vec![Item {
            path_in_repo: "a.txt".into(),
            source: Some(work_dir.path().join("a.txt")),
            is_hardlink: false,
            ops: Ops::default(),
            devices: HashMap::new(),
        }];
        let (cfg, _) = make_config(&device_id, items);

        handle_collect(&cfg, repo_dir.path(), false, false)?;
        assert_eq!(fs::read_to_string(repo_dir.path().join("a.txt"))?, "hello");

        // 改本地后再 restore
        write_file(&work_dir.path().join("a.txt"), b"modified!!!");
        handle_restore(&cfg, repo_dir.path(), false)?;
        assert_eq!(fs::read_to_string(work_dir.path().join("a.txt"))?, "hello");

        Ok(())
    }

    #[test]
    fn collect_uses_alias_source_override() -> Result<()> {
        let device_id = crate::utils::get_current_device_name()?;
        let repo_dir = tempdir()?;
        let default_dir = tempdir()?;
        let alias_dir = tempdir()?;
        init_repo(repo_dir.path());

        write_file(&default_dir.path().join("data.txt"), b"from default");
        write_file(&alias_dir.path().join("data.txt"), b"from alias");

        let mut devices = HashMap::new();
        devices.insert(
            "mywork".to_string(),
            DeviceOverride {
                source: Some(alias_dir.path().join("data.txt")),
                ops: None,
            },
        );
        let items = vec![Item {
            path_in_repo: "data.txt".into(),
            source: Some(default_dir.path().join("data.txt")),
            is_hardlink: false,
            ops: Ops::default(),
            devices,
        }];
        let cfg = Config {
            version: "0.5".into(),
            sync_interval: 3600,
            git: GitConfig::default(),
            aliases: HashMap::from([("mywork".to_string(), device_id.clone())]),
            vars: HashMap::new(),
            items,
        };

        handle_collect(&cfg, repo_dir.path(), false, false)?;
        assert_eq!(
            fs::read_to_string(repo_dir.path().join("data.txt"))?,
            "from alias"
        );

        Ok(())
    }

    #[test]
    fn restore_with_ops_collect_only_skips() -> Result<()> {
        let device_id = crate::utils::get_current_device_name()?;
        let repo_dir = tempdir()?;
        let work_dir = tempdir()?;
        init_repo(repo_dir.path());
        write_file(&repo_dir.path().join("b.txt"), b"from repo");

        let items = vec![Item {
            path_in_repo: "b.txt".into(),
            source: Some(work_dir.path().join("b.txt")),
            is_hardlink: false,
            ops: Ops::new([Op::Collect]),
            devices: HashMap::new(),
        }];
        let (cfg, _) = make_config(&device_id, items);

        handle_restore(&cfg, repo_dir.path(), false)?;
        assert!(!work_dir.path().join("b.txt").exists());
        Ok(())
    }

    #[test]
    fn collect_with_vars_expansion() -> Result<()> {
        let repo_dir = tempdir()?;
        let work_dir = tempdir()?;
        init_repo(repo_dir.path());
        write_file(&work_dir.path().join("v.txt"), b"var content");

        let items = vec![Item {
            path_in_repo: "v.txt".into(),
            source: Some(PathBuf::from("{MY_VAR}/v.txt")),
            is_hardlink: false,
            ops: Ops::default(),
            devices: HashMap::new(),
        }];
        let cfg = Config {
            version: "0.5".into(),
            sync_interval: 3600,
            git: GitConfig::default(),
            aliases: HashMap::new(),
            vars: HashMap::from([(
                "MY_VAR".to_string(),
                work_dir.path().to_string_lossy().to_string(),
            )]),
            items,
        };

        handle_collect(&cfg, repo_dir.path(), false, false)?;
        assert_eq!(
            fs::read_to_string(repo_dir.path().join("v.txt"))?,
            "var content"
        );
        Ok(())
    }
}
