use std::{path::PathBuf, process::Command, sync::LazyLock};

use anyhow::Result;
use die_exit::{die, Die, DieWith};
use whoami::devicename;

use crate::cli::CLI;

pub const REMOTE_NAME: &str = "origin";
pub const SYNC_BRANCH: &str = "sync";
pub static BACKUP_BRANCH: LazyLock<String> =
    LazyLock::new(|| "backup-".to_string() + devicename().as_str());

/// Read from env first, parameter second, cwd third.
pub static REPO_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var(env!("CARGO_PKG_NAME").to_uppercase())
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            CLI.get()
                .and_then(|cli| cli.repo.clone())
                .unwrap_or(std::env::current_dir().die("no repo path found."))
        })
});

pub fn ensure_utf8() -> Result<()> {
    #[cfg(target_os = "windows")]
    Command::new("cmd").args(["/C", "chcp", "65001"]).output()?;
    Ok(())
}

pub fn git(args: impl AsRef<[&str]>) -> Result<String> {
    let _ = ensure_utf8();
    let mut command = Command::new("cmd");
    let output = command
        .args(["/C", "git"])
        .args(args.as_ref())
        .current_dir(REPO_PATH.as_path())
        .output()?;
    Ok(String::from_utf8(output.stdout)?)
}

mod tests {
    use super::*;

    /// Needs REPO_PATH to be set to a real repo, like `.` as default.
    #[test]
    fn test_git() {
        let result = git(["status"]);
        assert!(result.is_ok());
        dbg!(result.unwrap());
    }
}
