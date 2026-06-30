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
    config::{Config, DeviceOverride, GitConfig, Item, Op, Ops},
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

/// 构造一个简单的双向同步 item。
fn bidir_item(path_in_repo: &str, source: PathBuf) -> Item {
    Item {
        path_in_repo: path_in_repo.to_string(),
        source: Some(source),
        is_hardlink: false,
        ops: Ops::default(),
        devices: HashMap::new(),
    }
}

fn make_config(items: Vec<Item>) -> Config {
    Config {
        version: "0.5".to_string(),
        sync_interval: 3600,
        git: GitConfig::default(),
        aliases: HashMap::new(),
        vars: HashMap::new(),
        items,
    }
}

// =========================================================================
// 基础 collect / restore
// =========================================================================

#[test]
fn collect_file_from_default_source() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"hello from default");

    let config = make_config(vec![bidir_item(
        "file1.txt",
        work_dir.path().join("file1.txt"),
    )]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

    let collected = repo_dir.path().join("file1.txt");
    assert!(collected.exists());
    assert_eq!(read_file(&collected), "hello from default");
}

#[test]
fn collect_directory_recursively() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    let dir = work_dir.path().join("mydir");
    write_file(&dir.join("a.txt"), b"file a");
    write_file(&dir.join("sub/b.txt"), b"file b");

    let config = make_config(vec![bidir_item("mydir", dir)]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

    let collected_dir = repo_dir.path().join("mydir");
    assert_eq!(read_file(&collected_dir.join("a.txt")), "file a");
    assert_eq!(read_file(&collected_dir.join("sub/b.txt")), "file b");
}

#[test]
fn collect_with_autocommit() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"autocommit content");

    let config = make_config(vec![bidir_item(
        "file1.txt",
        work_dir.path().join("file1.txt"),
    )]);

    ops::handle_collect(&config, repo_dir.path(), true, false).unwrap();

    let device_name = utils::get_current_device_name().unwrap();
    let repo = git2::Repository::open(repo_dir.path()).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let msg = head.message().unwrap();
    assert!(msg.contains("gsb collect on"));
    assert!(msg.contains(&device_name));
}

#[test]
fn collect_skip_unchanged_files() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"content");

    let config = make_config(vec![bidir_item(
        "file1.txt",
        work_dir.path().join("file1.txt"),
    )]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    let first_mtime = fs::metadata(repo_dir.path().join("file1.txt"))
        .unwrap()
        .modified()
        .unwrap();

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
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

#[test]
fn collect_source_not_exist_skips_gracefully() {
    let repo_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    let config = make_config(vec![bidir_item(
        "missing.txt",
        PathBuf::from("/nonexistent/path/file.txt"),
    )]);

    let result = ops::handle_collect(&config, repo_dir.path(), false, false);
    assert!(result.is_ok());
    assert!(!repo_dir.path().join("missing.txt").exists());
}

#[test]
fn collect_hardlink_file() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    let source = work_dir.path().join("hardlink_source.txt");
    write_file(&source, b"hardlink content");

    let config = make_config(vec![Item {
        path_in_repo: "hardlink_file.txt".to_string(),
        source: Some(source.clone()),
        is_hardlink: true,
        ops: Ops::default(),
        devices: HashMap::new(),
    }]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

    let collected = repo_dir.path().join("hardlink_file.txt");
    assert!(collected.exists());
    assert!(same_file::is_same_file(&source, &collected).unwrap());
}

#[test]
fn collect_multiple_items() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("a.txt"), b"file a");
    write_file(&work_dir.path().join("b.txt"), b"file b");
    let dir = work_dir.path().join("subdir");
    write_file(&dir.join("c.txt"), b"file c");

    let config = make_config(vec![
        bidir_item("a.txt", work_dir.path().join("a.txt")),
        bidir_item("b.txt", work_dir.path().join("b.txt")),
        bidir_item("subdir", dir),
    ]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

    assert_eq!(read_file(&repo_dir.path().join("a.txt")), "file a");
    assert_eq!(read_file(&repo_dir.path().join("b.txt")), "file b");
    assert_eq!(read_file(&repo_dir.path().join("subdir/c.txt")), "file c");
}

// =========================================================================
// 设备覆盖（devices 表）
// =========================================================================

#[test]
fn collect_uses_alias_source_override() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let default_dir = tempfile::tempdir().unwrap();
    let alias_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    // 两个路径都有内容，alias 路径不同
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
    let item = Item {
        path_in_repo: "data.txt".to_string(),
        source: Some(default_dir.path().join("data.txt")),
        is_hardlink: false,
        ops: Ops::default(),
        devices,
    };
    let config = Config {
        version: "0.5".to_string(),
        sync_interval: 3600,
        git: GitConfig::default(),
        aliases: HashMap::from([("mywork".to_string(), device_name)]),
        vars: HashMap::new(),
        items: vec![item],
    };

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

    assert_eq!(read_file(&repo_dir.path().join("data.txt")), "from alias");
}

#[test]
fn collect_uses_hash_source_key() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let hash_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&hash_dir.path().join("data.txt"), b"from hash key");

    let mut devices = HashMap::new();
    devices.insert(
        device_name.clone(),
        DeviceOverride {
            source: Some(hash_dir.path().join("data.txt")),
            ops: None,
        },
    );
    let item = Item {
        path_in_repo: "data.txt".to_string(),
        source: Some(PathBuf::from("/nonexistent/default.txt")),
        is_hardlink: false,
        ops: Ops::default(),
        devices,
    };
    let config = make_config(vec![item]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

    let collected = repo_dir.path().join("data.txt");
    assert!(collected.exists());
    assert_eq!(read_file(&collected), "from hash key");
}

// =========================================================================
// ops 配置：仅 collect / 仅 restore / 跳过
// =========================================================================

#[test]
fn ops_collect_only_skips_restore() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("b.txt"), b"from repo");

    let item = Item {
        path_in_repo: "b.txt".to_string(),
        source: Some(work_dir.path().join("b.txt")),
        is_hardlink: false,
        ops: Ops::new([Op::Collect]),
        devices: HashMap::new(),
    };
    let config = make_config(vec![item]);

    ops::handle_restore(&config, repo_dir.path(), false).unwrap();
    assert!(!work_dir.path().join("b.txt").exists());
}

#[test]
fn ops_restore_only_skips_collect() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("c.txt"), b"local content");

    let item = Item {
        path_in_repo: "c.txt".to_string(),
        source: Some(work_dir.path().join("c.txt")),
        is_hardlink: false,
        ops: Ops::new([Op::Restore]),
        devices: HashMap::new(),
    };
    let config = make_config(vec![item]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    // collect 被跳过：仓库中不应有文件
    assert!(!repo_dir.path().join("c.txt").exists());

    // 反过来在仓库中放文件再 restore 应该工作
    write_file(&repo_dir.path().join("c.txt"), b"repo content");
    ops::handle_restore(&config, repo_dir.path(), false).unwrap();
    assert_eq!(read_file(&work_dir.path().join("c.txt")), "repo content");
}

#[test]
fn ops_empty_skips_both() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("d.txt"), b"local");
    write_file(&repo_dir.path().join("d.txt"), b"repo");

    let item = Item {
        path_in_repo: "d.txt".to_string(),
        source: Some(work_dir.path().join("d.txt")),
        is_hardlink: false,
        ops: Ops::new([]),
        devices: HashMap::new(),
    };
    let config = make_config(vec![item]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    // 仓库文件不变
    assert_eq!(read_file(&repo_dir.path().join("d.txt")), "repo");

    ops::handle_restore(&config, repo_dir.path(), false).unwrap();
    // 本地文件不变
    assert_eq!(read_file(&work_dir.path().join("d.txt")), "local");
}

/// 设备级 ops 覆盖（仅当前设备跳过 restore）。
#[test]
fn device_level_ops_override() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("e.txt"), b"local");
    write_file(&repo_dir.path().join("e.txt"), b"repo");

    let mut devices = HashMap::new();
    devices.insert(
        device_name,
        DeviceOverride {
            source: None,
            ops: Some(Ops::new([Op::Collect])),
        },
    );
    let item = Item {
        path_in_repo: "e.txt".to_string(),
        source: Some(work_dir.path().join("e.txt")),
        is_hardlink: false,
        ops: Ops::default(),
        devices,
    };
    let config = make_config(vec![item]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    assert_eq!(read_file(&repo_dir.path().join("e.txt")), "local");

    // 修改仓库文件，restore 应被跳过
    write_file(&repo_dir.path().join("e.txt"), b"repo v2");
    ops::handle_restore(&config, repo_dir.path(), false).unwrap();
    assert_eq!(read_file(&work_dir.path().join("e.txt")), "local");
}

// =========================================================================
// restore
// =========================================================================

#[test]
fn restore_file_to_default_source() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("file1.txt"), b"from repo");

    let config = make_config(vec![bidir_item(
        "file1.txt",
        work_dir.path().join("file1.txt"),
    )]);

    ops::handle_restore(&config, repo_dir.path(), false).unwrap();

    let restored = work_dir.path().join("file1.txt");
    assert!(restored.exists());
    assert_eq!(read_file(&restored), "from repo");
}

#[test]
fn restore_uses_alias_source_instead_of_default() {
    let device_name = utils::get_current_device_name().unwrap();
    let repo_dir = tempfile::tempdir().unwrap();
    let default_dir = tempfile::tempdir().unwrap();
    let alias_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&repo_dir.path().join("data.txt"), b"repo content");

    let mut devices = HashMap::new();
    devices.insert(
        "mywork".to_string(),
        DeviceOverride {
            source: Some(alias_dir.path().join("data.txt")),
            ops: None,
        },
    );
    let item = Item {
        path_in_repo: "data.txt".to_string(),
        source: Some(default_dir.path().join("data.txt")),
        is_hardlink: false,
        ops: Ops::default(),
        devices,
    };
    let config = Config {
        version: "0.5".to_string(),
        sync_interval: 3600,
        git: GitConfig::default(),
        aliases: HashMap::from([("mywork".to_string(), device_name)]),
        vars: HashMap::new(),
        items: vec![item],
    };

    ops::handle_restore(&config, repo_dir.path(), false).unwrap();

    assert!(alias_dir.path().join("data.txt").exists());
    assert_eq!(
        read_file(&alias_dir.path().join("data.txt")),
        "repo content"
    );
    assert!(!default_dir.path().join("data.txt").exists());
}

#[test]
fn collect_then_restore_roundtrip() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("file1.txt"), b"original content");

    let config = make_config(vec![bidir_item(
        "file1.txt",
        work_dir.path().join("file1.txt"),
    )]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    assert_eq!(
        read_file(&repo_dir.path().join("file1.txt")),
        "original content"
    );

    write_file(&work_dir.path().join("file1.txt"), b"modified!");

    ops::handle_restore(&config, repo_dir.path(), false).unwrap();
    assert_eq!(
        read_file(&work_dir.path().join("file1.txt")),
        "original content"
    );
}

#[test]
fn backup_only_item_collect_works_restore_skipped() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());

    write_file(&work_dir.path().join("important.txt"), b"my important data");

    let item = Item {
        path_in_repo: "important.txt".to_string(),
        source: Some(work_dir.path().join("important.txt")),
        is_hardlink: false,
        ops: Ops::new([Op::Collect]),
        devices: HashMap::new(),
    };
    let config = make_config(vec![item]);

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    assert_eq!(
        read_file(&repo_dir.path().join("important.txt")),
        "my important data"
    );

    write_file(
        &repo_dir.path().join("important.txt"),
        b"overwritten by another device",
    );

    ops::handle_restore(&config, repo_dir.path(), false).unwrap();
    assert_eq!(
        read_file(&work_dir.path().join("important.txt")),
        "my important data"
    );
}

// =========================================================================
// 变量展开
// =========================================================================

#[test]
fn collect_with_vars_expansion() {
    let repo_dir = tempfile::tempdir().unwrap();
    let work_dir = tempfile::tempdir().unwrap();
    init_git_repo(repo_dir.path());
    write_file(&work_dir.path().join("v.txt"), b"var content");

    let item = Item {
        path_in_repo: "v.txt".to_string(),
        source: Some(PathBuf::from("{MY_VAR}/v.txt")),
        is_hardlink: false,
        ops: Ops::default(),
        devices: HashMap::new(),
    };
    let config = Config {
        version: "0.5".to_string(),
        sync_interval: 3600,
        git: GitConfig::default(),
        aliases: HashMap::new(),
        vars: HashMap::from([(
            "MY_VAR".to_string(),
            work_dir.path().to_string_lossy().to_string(),
        )]),
        items: vec![item],
    };

    ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
    assert_eq!(read_file(&repo_dir.path().join("v.txt")), "var content");
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

    fn write_gse_config(repo_root: &Path, crypt_list: &[&str]) {
        let list_entries: Vec<String> = crypt_list.iter().map(|s| format!("\"{s}\"")).collect();
        let toml_content = format!(
            "use_zstd = false\nzstd_level = 0\ncrypt_list = [{}]\n",
            list_entries.join(", ")
        );
        write_file(
            &repo_root.join("git_simple_encrypt.toml"),
            toml_content.as_bytes(),
        );
    }

    fn set_encrypt_key(repo_root: &Path, key: &str) {
        let repo = git2::Repository::open(repo_root).unwrap();
        repo.config()
            .unwrap()
            .set_str("git-simple-encrypt.key", key)
            .unwrap();
    }

    #[test]
    fn collect_encrypts_files_in_crypt_list() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_gse_config(repo_dir.path(), &["secret.txt"]);
        set_encrypt_key(repo_dir.path(), "test_password");

        write_file(&work_dir.path().join("secret.txt"), b"top secret data");

        let config = make_config(vec![bidir_item(
            "secret.txt",
            work_dir.path().join("secret.txt"),
        )]);

        ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

        let repo_file = repo_dir.path().join("secret.txt");
        assert!(repo_file.exists());
        assert!(is_encrypted(&repo_file), "File should be encrypted in repo");
        assert_ne!(fs::read(&repo_file).unwrap(), b"top secret data");
    }

    #[test]
    fn collect_does_not_encrypt_files_not_in_crypt_list() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_gse_config(repo_dir.path(), &["secret.txt"]);
        set_encrypt_key(repo_dir.path(), "test_password");

        write_file(&work_dir.path().join("plain.txt"), b"plain content");
        write_file(&work_dir.path().join("secret.txt"), b"secret content");

        let config = make_config(vec![
            bidir_item("plain.txt", work_dir.path().join("plain.txt")),
            bidir_item("secret.txt", work_dir.path().join("secret.txt")),
        ]);

        ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

        let plain_file = repo_dir.path().join("plain.txt");
        assert!(!is_encrypted(&plain_file));
        assert_eq!(read_file(&plain_file), "plain content");

        let secret_file = repo_dir.path().join("secret.txt");
        assert!(is_encrypted(&secret_file));
    }

    #[test]
    fn collect_then_restore_decrypts_to_plaintext() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_gse_config(repo_dir.path(), &["secret.txt"]);
        set_encrypt_key(repo_dir.path(), "roundtrip_password");

        write_file(&work_dir.path().join("secret.txt"), b"original secret");

        let config = make_config(vec![bidir_item(
            "secret.txt",
            work_dir.path().join("secret.txt"),
        )]);

        ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();
        assert!(is_encrypted(&repo_dir.path().join("secret.txt")));

        fs::remove_file(work_dir.path().join("secret.txt")).unwrap();

        ops::handle_restore(&config, repo_dir.path(), false).unwrap();

        let restored = work_dir.path().join("secret.txt");
        assert!(restored.exists());
        assert!(!is_encrypted(&restored));
        assert_eq!(read_file(&restored), "original secret");

        assert!(
            is_encrypted(&repo_dir.path().join("secret.txt")),
            "Repo file should be re-encrypted after restore"
        );
    }

    #[test]
    fn no_gse_config_skips_encryption() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_file(&work_dir.path().join("plain.txt"), b"no encryption here");

        let config = make_config(vec![bidir_item(
            "plain.txt",
            work_dir.path().join("plain.txt"),
        )]);

        ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

        assert!(!is_encrypted(&repo_dir.path().join("plain.txt")));
        assert_eq!(
            read_file(&repo_dir.path().join("plain.txt")),
            "no encryption here"
        );
    }

    #[test]
    fn encrypt_decrypt_directory_roundtrip() {
        let repo_dir = tempfile::tempdir().unwrap();
        let work_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());

        write_gse_config(repo_dir.path(), &["secrets"]);
        set_encrypt_key(repo_dir.path(), "dir_password");

        write_file(&work_dir.path().join("secrets/a.txt"), b"secret A");
        write_file(&work_dir.path().join("secrets/sub/b.txt"), b"secret B");

        let config = make_config(vec![bidir_item("secrets", work_dir.path().join("secrets"))]);

        ops::handle_collect(&config, repo_dir.path(), false, false).unwrap();

        assert!(is_encrypted(&repo_dir.path().join("secrets/a.txt")));
        assert!(is_encrypted(&repo_dir.path().join("secrets/sub/b.txt")));

        ops::handle_restore(&config, repo_dir.path(), false).unwrap();

        assert_eq!(
            read_file(&work_dir.path().join("secrets/a.txt")),
            "secret A"
        );
        assert_eq!(
            read_file(&work_dir.path().join("secrets/sub/b.txt")),
            "secret B"
        );

        assert!(is_encrypted(&repo_dir.path().join("secrets/a.txt")));
    }
}
