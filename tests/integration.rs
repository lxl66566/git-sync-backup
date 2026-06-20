//! 集成测试：模拟用户操作，验证 collect / restore 的端到端行为。
//!
//! 每个测试在独立的临时目录中运行，支持并行执行。
#![allow(clippy::doc_markdown)]

use std::{
    collections::HashMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use git_sync_backup::{
    config::{Config, GitConfig, Item, RestorePolicy},
    ops, utils,
};

fn init_git_repo(path: &Path) {
    let repo = git2::Repository::init(path).expect("无法初始化 Git 仓库");
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

fn read_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

/// 模拟用户操作：collect（不带 autocommit），验证普通文件从 default_source
/// 收集到仓库
#[test]
fn collect_file_from_default_source() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"hello from default");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();

    let collected = repo_dir.path().join("file1.txt");
    assert!(collected.exists());
    assert_eq!(read_file(&collected), "hello from default");
}

/// 模拟用户操作：collect，验证目录递归收集
#[test]
fn collect_directory_recursively() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    let dir = work_dir.path().join("mydir");
    write_file(&dir.join("a.txt"), b"file a");
    write_file(&dir.join("sub/b.txt"), b"file b");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "mydir".to_string(),
            default_source: Some(dir),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();

    let collected_dir = repo_dir.path().join("mydir");
    assert_eq!(read_file(&collected_dir.join("a.txt")), "file a");
    assert_eq!(read_file(&collected_dir.join("sub/b.txt")), "file b");
}

/// 模拟用户操作：collect --autocommit，验证 git 提交生成
#[test]
fn collect_with_autocommit() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"autocommit content");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), true).unwrap();

    let device_name = utils::get_current_device_name().unwrap();
    let repo = git2::Repository::open(repo_dir.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let msg = head.message().unwrap();
    assert!(msg.contains("gsb collect on"));
    assert!(msg.contains(&device_name));
}

/// 验证 bug 修复：sources 中使用别名作为 key 时，collect
/// 能正确找到设备对应的路径
#[test]
fn collect_uses_alias_source_instead_of_default() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let default_dir = tempfile::tempdir().unwrap();
    let alias_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    // 两个路径都有内容，但 alias 路径的内容不同
    write_file(&default_dir.path().join("data.txt"), b"from default");
    write_file(&alias_dir.path().join("data.txt"), b"from alias");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::from([("mywork".to_string(), device_name.clone())]),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "data.txt".to_string(),
            default_source: Some(default_dir.path().join("data.txt")),
            is_hardlink: false,
            sources: Some(HashMap::from([(
                "mywork".to_string(),
                alias_dir.path().join("data.txt"),
            )])),
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();

    let collected = repo_dir.path().join("data.txt");
    assert!(collected.exists());
    assert_eq!(read_file(&collected), "from alias");
}

/// 验证 sources 中使用原始 hash 作为 key 时也能正确匹配
#[test]
fn collect_uses_hash_source_key() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let hash_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&hash_dir.path().join("data.txt"), b"from hash key");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "data.txt".to_string(),
            default_source: Some(PathBuf::from("/nonexistent/default.txt")),
            is_hardlink: false,
            sources: Some(HashMap::from([(
                device_name,
                hash_dir.path().join("data.txt"),
            )])),
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();

    let collected = repo_dir.path().join("data.txt");
    assert!(collected.exists());
    assert_eq!(read_file(&collected), "from hash key");
}

/// 模拟用户操作：restore，验证文件从仓库恢复到本地路径
#[test]
fn restore_file_to_default_source() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("file1.txt"), b"from repo");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_restore(&config, repo_dir.path(), true).unwrap();

    let restored = work_dir.path().join("file1.txt");
    assert!(restored.exists());
    assert_eq!(read_file(&restored), "from repo");
}

/// 模拟用户操作：restore，验证 sources 别名 key 生效
#[test]
fn restore_uses_alias_source_instead_of_default() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let default_dir = tempfile::tempdir().unwrap();
    let alias_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("data.txt"), b"repo content");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::from([("mywork".to_string(), device_name)]),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "data.txt".to_string(),
            default_source: Some(default_dir.path().join("data.txt")),
            is_hardlink: false,
            sources: Some(HashMap::from([(
                "mywork".to_string(),
                alias_dir.path().join("data.txt"),
            )])),
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_restore(&config, repo_dir.path(), true).unwrap();

    assert!(alias_dir.path().join("data.txt").exists());
    assert_eq!(
        read_file(&alias_dir.path().join("data.txt")),
        "repo content"
    );
    assert!(!default_dir.path().join("data.txt").exists());
}

/// 模拟用户操作：先 collect 再 restore 的完整来回
#[test]
fn collect_then_restore_roundtrip() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"original content");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    // collect: work -> repo
    ops::handle_collect(&config, repo_dir.path(), false).unwrap();
    assert_eq!(
        read_file(&repo_dir.path().join("file1.txt")),
        "original content"
    );

    // 修改本地文件（使用不同长度以确保大小不同，避免 mtime 相同秒的误判）
    write_file(&work_dir.path().join("file1.txt"), b"modified!");

    // restore: repo -> work（恢复到仓库中的版本）
    ops::handle_restore(&config, repo_dir.path(), true).unwrap();
    assert_eq!(
        read_file(&work_dir.path().join("file1.txt")),
        "original content"
    );
}

/// 验证：未修改的文件在第二次 collect 时不会被覆盖（修改时间不变）
#[test]
fn collect_skip_unchanged_files() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"content");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();
    let first_mtime = fs::metadata(repo_dir.path().join("file1.txt"))
        .unwrap()
        .modified()
        .unwrap();

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();
    let second_mtime = fs::metadata(repo_dir.path().join("file1.txt"))
        .unwrap()
        .modified()
        .unwrap();

    assert_eq!(
        first_mtime
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        second_mtime
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
}

/// 验证：源路径不存在时 collect 不报错，跳过该项
#[test]
fn collect_source_not_exist_skips_gracefully() {
    let repo_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "missing.txt".to_string(),
            default_source: Some(PathBuf::from("/nonexistent/path/file.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    let result = ops::handle_collect(&config, repo_dir.path(), false);
    assert!(result.is_ok());
    assert!(!repo_dir.path().join("missing.txt").exists());
}

/// 验证：ignore_collect 使当前设备跳过收集
#[test]
fn collect_ignored_device_skips_item() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(
        &work_dir.path().join("ignored.txt"),
        b"should not be collected",
    );

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "ignored.txt".to_string(),
            default_source: Some(work_dir.path().join("ignored.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![device_name],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();
    assert!(!repo_dir.path().join("ignored.txt").exists());
}

/// 验证：ignore_restore 使当前设备跳过恢复
#[test]
fn restore_ignored_device_skips_item() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("file1.txt"), b"from repo");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![device_name],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_restore(&config, repo_dir.path(), true).unwrap();
    assert!(!work_dir.path().join("file1.txt").exists());
}

/// 验证：ignore_collect 使用别名也能匹配
#[test]
fn collect_ignored_by_alias() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"content");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::from([("mywork".to_string(), device_name)]),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "file1.txt".to_string(),
            default_source: Some(work_dir.path().join("file1.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec!["mywork".to_string()],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();
    assert!(!repo_dir.path().join("file1.txt").exists());
}

/// 验证：硬链接文件收集后与源文件是同一 inode
#[test]
fn collect_hardlink_file() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    let source = work_dir.path().join("hardlink_source.txt");
    write_file(&source, b"hardlink content");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "hardlink_file.txt".to_string(),
            default_source: Some(source.clone()),
            is_hardlink: true,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        }],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();

    let collected = repo_dir.path().join("hardlink_file.txt");
    assert!(collected.exists());
    assert!(same_file::is_same_file(&source, &collected).unwrap());
}

/// 验证：多个 item 同时 collect
#[test]
fn collect_multiple_items() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("a.txt"), b"file a");
    write_file(&work_dir.path().join("b.txt"), b"file b");
    let dir = work_dir.path().join("subdir");
    write_file(&dir.join("c.txt"), b"file c");

    let config = Config {
        version: "0.2.1".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![
            Item {
                path_in_repo: "a.txt".to_string(),
                default_source: Some(work_dir.path().join("a.txt")),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        },
            Item {
                path_in_repo: "b.txt".to_string(),
                default_source: Some(work_dir.path().join("b.txt")),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        },
            Item {
                path_in_repo: "subdir".to_string(),
                default_source: Some(dir),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::All,
            restore_devices: vec![],
        },
        ],
    };

    ops::handle_collect(&config, repo_dir.path(), false).unwrap();

    assert_eq!(read_file(&repo_dir.path().join("a.txt")), "file a");
    assert_eq!(read_file(&repo_dir.path().join("b.txt")), "file b");
    assert_eq!(read_file(&repo_dir.path().join("subdir/c.txt")), "file c");
}

/// 验证：restore = "off" 的 item 永远不会被 restore，即使仓库中有文件
#[test]
fn restore_policy_off_never_restores() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    // 仓库中有文件
    write_file(&repo_dir.path().join("backup_only.txt"), b"from repo");

    let config = Config {
        version: "0.3.0".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "backup_only.txt".to_string(),
            default_source: Some(work_dir.path().join("backup_only.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Off,
            restore_devices: vec![],
        }],
    };

    ops::handle_restore(&config, repo_dir.path(), true).unwrap();

    // 目标文件不应被创建
    assert!(!work_dir.path().join("backup_only.txt").exists());
}

/// 验证：restore = "explicit" 仅白名单设备才 restore
#[test]
fn restore_policy_explicit_whitelist() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("work_docs.txt"), b"from repo");

    let config = Config {
        version: "0.3.0".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "work_docs.txt".to_string(),
            default_source: Some(work_dir.path().join("work_docs.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Explicit,
            restore_devices: vec![device_name],
        }],
    };

    // 白名单包含当前设备 → 应该 restore
    ops::handle_restore(&config, repo_dir.path(), true).unwrap();
    assert!(work_dir.path().join("work_docs.txt").exists());
    assert_eq!(
        read_file(&work_dir.path().join("work_docs.txt")),
        "from repo"
    );
}

/// 验证：restore = "explicit" 时当前设备不在白名单 → 不 restore
#[test]
fn restore_policy_explicit_not_in_whitelist_skips() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("work_docs.txt"), b"from repo");

    let config = Config {
        version: "0.3.0".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "work_docs.txt".to_string(),
            default_source: Some(work_dir.path().join("work_docs.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Explicit,
            // 白名单为空 → 当前设备不在其中
            restore_devices: vec!["some-other-device".to_string()],
        }],
    };

    ops::handle_restore(&config, repo_dir.path(), true).unwrap();

    // 当前设备不在白名单 → 不应被 restore
    assert!(!work_dir.path().join("work_docs.txt").exists());
}

/// 验证：restore = "explicit" + 别名匹配白名单
#[test]
fn restore_policy_explicit_alias_in_whitelist() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("data.txt"), b"from repo");

    let config = Config {
        version: "0.3.0".to_string(),
        sync_interval: 3600,
        aliases: HashMap::from([("mywork".to_string(), device_name)]),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "data.txt".to_string(),
            default_source: Some(work_dir.path().join("data.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Explicit,
            // 用别名而非原始 hash
            restore_devices: vec!["mywork".to_string()],
        }],
    };

    ops::handle_restore(&config, repo_dir.path(), true).unwrap();
    assert!(work_dir.path().join("data.txt").exists());
}

/// 验证：collect + restore roundtrip 对 restore = "off" 的 item 仍然安全
/// （collect 正常，restore 跳过，数据不会被覆盖）
#[test]
fn backup_only_item_collect_works_restore_skipped() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    // 本地有重要数据
    write_file(&work_dir.path().join("important.txt"), b"my important data");

    let config = Config {
        version: "0.3.0".to_string(),
        sync_interval: 3600,
        aliases: HashMap::new(),
        git: GitConfig {
            remote: None,
            branch: None,
        },
        items: vec![Item {
            path_in_repo: "important.txt".to_string(),
            default_source: Some(work_dir.path().join("important.txt")),
            is_hardlink: false,
            sources: None,
            ignore_collect: vec![],
            ignore_restore: vec![],
            ignore: vec![],
            restore: RestorePolicy::Off,
            restore_devices: vec![],
        }],
    };

    // collect：本地 -> 仓库（正常工作）
    ops::handle_collect(&config, repo_dir.path(), false).unwrap();
    assert_eq!(
        read_file(&repo_dir.path().join("important.txt")),
        "my important data"
    );

    // 模拟：仓库中被其他设备修改
    write_file(
        &repo_dir.path().join("important.txt"),
        b"overwritten by another device",
    );

    // restore：由于 restore = off，本地数据不会被覆盖
    ops::handle_restore(&config, repo_dir.path(), true).unwrap();
    assert_eq!(
        read_file(&work_dir.path().join("important.txt")),
        "my important data"
    );
}

// =========================================================================
// 加密集成测试（feature = "encrypt"）
// =========================================================================

#[cfg(feature = "encrypt")]
mod encrypt_tests {
    use super::*;

    /// GITSE 加密文件的 magic bytes
    const GITSE_MAGIC: &[u8] = b"GITSE";

    /// 判断文件是否已被 git-simple-encrypt 加密（检查 GITSE magic header）
    fn is_encrypted(path: &Path) -> bool {
        if let Ok(content) = fs::read(path) {
            content.len() >= 64 && &content[..5] == GITSE_MAGIC
        } else {
            false
        }
    }

    /// 在仓库中创建 git_simple_encrypt.toml 配置文件
    fn write_gse_config(repo_root: &Path, crypt_list: &[&str]) {
        let list_entries: Vec<String> = crypt_list.iter().map(|s| format!("\"{s}\"")).collect();
        let toml_content = format!(
            "use_zstd = false\nzstd_level = 0\ncrypt_list = [{}]\n",
            list_entries.join(", ")
        );
        write_file(&repo_root.join("git_simple_encrypt.toml"), toml_content.as_bytes());
    }

    /// 设置 git-simple-encrypt 的主密钥（通过 git config）
    fn set_encrypt_key(repo_root: &Path, key: &str) {
        let repo = git2::Repository::open(repo_root).unwrap();
        repo.config()
            .unwrap()
            .set_str("git-simple-encrypt.key", key)
            .unwrap();
    }

    /// 验证：collect 后仓库中的加密列表文件确实被加密
    #[test]
    fn collect_encrypts_files_in_crypt_list() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_gse_config(repo_dir.path(), &["secret.txt"]);
        set_encrypt_key(repo_dir.path(), "test_password");

        write_file(&work_dir.path().join("secret.txt"), b"top secret data");

        let config = Config {
            version: "0.3.0".to_string(),
            sync_interval: 3600,
            aliases: HashMap::new(),
            git: GitConfig {
                remote: None,
                branch: None,
            },
            items: vec![Item {
                path_in_repo: "secret.txt".to_string(),
                default_source: Some(work_dir.path().join("secret.txt")),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
                ignore: vec![],
                restore: RestorePolicy::All,
                restore_devices: vec![],
            }],
        };

        ops::handle_collect(&config, repo_dir.path(), false).unwrap();

        // 仓库中的文件应该是加密的
        let repo_file = repo_dir.path().join("secret.txt");
        assert!(repo_file.exists());
        assert!(is_encrypted(&repo_file), "File should be encrypted in repo");
        // 确保内容不是明文（用字节比较，密文可能不是有效 UTF-8）
        assert_ne!(fs::read(&repo_file).unwrap(), b"top secret data");
    }

    /// 验证：不在 crypt_list 中的文件不会被加密
    #[test]
    fn collect_does_not_encrypt_files_not_in_crypt_list() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        // crypt_list 只包含 secret.txt，不包含 plain.txt
        write_gse_config(repo_dir.path(), &["secret.txt"]);
        set_encrypt_key(repo_dir.path(), "test_password");

        write_file(&work_dir.path().join("plain.txt"), b"plain content");
        write_file(&work_dir.path().join("secret.txt"), b"secret content");

        let config = Config {
            version: "0.3.0".to_string(),
            sync_interval: 3600,
            aliases: HashMap::new(),
            git: GitConfig {
                remote: None,
                branch: None,
            },
            items: vec![
                Item {
                    path_in_repo: "plain.txt".to_string(),
                    default_source: Some(work_dir.path().join("plain.txt")),
                    is_hardlink: false,
                    sources: None,
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                    ignore: vec![],
                    restore: RestorePolicy::All,
                    restore_devices: vec![],
                },
                Item {
                    path_in_repo: "secret.txt".to_string(),
                    default_source: Some(work_dir.path().join("secret.txt")),
                    is_hardlink: false,
                    sources: None,
                    ignore_collect: vec![],
                    ignore_restore: vec![],
                    ignore: vec![],
                    restore: RestorePolicy::All,
                    restore_devices: vec![],
                },
            ],
        };

        ops::handle_collect(&config, repo_dir.path(), false).unwrap();

        // plain.txt 应该是明文
        let plain_file = repo_dir.path().join("plain.txt");
        assert!(!is_encrypted(&plain_file));
        assert_eq!(read_file(&plain_file), "plain content");

        // secret.txt 应该是密文
        let secret_file = repo_dir.path().join("secret.txt");
        assert!(is_encrypted(&secret_file));
    }

    /// 验证：collect → restore roundtrip，目标文件是明文
    #[test]
    fn collect_then_restore_decrypts_to_plaintext() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_gse_config(repo_dir.path(), &["secret.txt"]);
        set_encrypt_key(repo_dir.path(), "roundtrip_password");

        write_file(&work_dir.path().join("secret.txt"), b"original secret");

        let config = Config {
            version: "0.3.0".to_string(),
            sync_interval: 3600,
            aliases: HashMap::new(),
            git: GitConfig {
                remote: None,
                branch: None,
            },
            items: vec![Item {
                path_in_repo: "secret.txt".to_string(),
                default_source: Some(work_dir.path().join("secret.txt")),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
                ignore: vec![],
                restore: RestorePolicy::All,
                restore_devices: vec![],
            }],
        };

        // collect: work -> repo (加密)
        ops::handle_collect(&config, repo_dir.path(), false).unwrap();
        assert!(is_encrypted(&repo_dir.path().join("secret.txt")));

        // 删除本地文件，模拟需要 restore 的场景
        fs::remove_file(work_dir.path().join("secret.txt")).unwrap();

        // restore: repo (解密) -> work
        ops::handle_restore(&config, repo_dir.path(), true).unwrap();

        // 目标文件应该是明文，内容正确
        let restored = work_dir.path().join("secret.txt");
        assert!(restored.exists());
        assert!(!is_encrypted(&restored));
        assert_eq!(read_file(&restored), "original secret");

        // restore 后仓库文件应该恢复为密文状态
        assert!(
            is_encrypted(&repo_dir.path().join("secret.txt")),
            "Repo file should be re-encrypted after restore"
        );
    }

    /// 验证：无 git_simple_encrypt.toml 时不执行加解密
    #[test]
    fn no_gse_config_skips_encryption() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        // 不创建 git_simple_encrypt.toml
        write_file(&work_dir.path().join("plain.txt"), b"no encryption here");

        let config = Config {
            version: "0.3.0".to_string(),
            sync_interval: 3600,
            aliases: HashMap::new(),
            git: GitConfig {
                remote: None,
                branch: None,
            },
            items: vec![Item {
                path_in_repo: "plain.txt".to_string(),
                default_source: Some(work_dir.path().join("plain.txt")),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
                ignore: vec![],
                restore: RestorePolicy::All,
                restore_devices: vec![],
            }],
        };

        ops::handle_collect(&config, repo_dir.path(), false).unwrap();

        // 文件应该是明文
        assert!(!is_encrypted(&repo_dir.path().join("plain.txt")));
        assert_eq!(
            read_file(&repo_dir.path().join("plain.txt")),
            "no encryption here"
        );
    }

    /// 验证：目录加解密 roundtrip
    #[test]
    fn encrypt_decrypt_directory_roundtrip() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        // crypt_list 包含整个目录
        write_gse_config(repo_dir.path(), &["secrets"]);
        set_encrypt_key(repo_dir.path(), "dir_password");

        // 创建多文件目录
        write_file(&work_dir.path().join("secrets/a.txt"), b"secret A");
        write_file(&work_dir.path().join("secrets/sub/b.txt"), b"secret B");

        let config = Config {
            version: "0.3.0".to_string(),
            sync_interval: 3600,
            aliases: HashMap::new(),
            git: GitConfig {
                remote: None,
                branch: None,
            },
            items: vec![Item {
                path_in_repo: "secrets".to_string(),
                default_source: Some(work_dir.path().join("secrets")),
                is_hardlink: false,
                sources: None,
                ignore_collect: vec![],
                ignore_restore: vec![],
                ignore: vec![],
                restore: RestorePolicy::All,
                restore_devices: vec![],
            }],
        };

        // collect
        ops::handle_collect(&config, repo_dir.path(), false).unwrap();

        // 仓库中的文件应该是加密的
        assert!(is_encrypted(&repo_dir.path().join("secrets/a.txt")));
        assert!(is_encrypted(&repo_dir.path().join("secrets/sub/b.txt")));

        // restore
        ops::handle_restore(&config, repo_dir.path(), true).unwrap();

        // 目标文件应该是明文
        assert_eq!(
            read_file(&work_dir.path().join("secrets/a.txt")),
            "secret A"
        );
        assert_eq!(
            read_file(&work_dir.path().join("secrets/sub/b.txt")),
            "secret B"
        );

        // 仓库文件应该恢复为密文
        assert!(is_encrypted(&repo_dir.path().join("secrets/a.txt")));
    }
}
