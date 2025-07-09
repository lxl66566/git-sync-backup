use std::path::Path;

use anyhow::{Ok, Result};
use die_exit::Die;

use crate::{
    config::{Getable, CONFIG},
    git_command::{git, REMOTE_NAME, REPO_PATH, SYNC_BRANCH},
};

/// Git pull the changes and dump the changed files.
pub async fn sync_pull() -> Result<()> {
    git(["branch", SYNC_BRANCH])?;
    let prev_commit = git(["rev-parse", "HEAD"])?;
    git(["fetch", REMOTE_NAME, SYNC_BRANCH])?;
    let files_changed = git(["diff", "--name-only", prev_commit.trim(), "FETCH_HEAD"])?;
    if files_changed.trim().is_empty() {
        return Ok(());
    }
    git(["reset", "--hard", "FETCH_HEAD"])?;
    let result = async_scoped::TokioScope::scope_and_block(|scope| {
        for path in files_changed.trim().lines() {
            scope.spawn(dump_changed_file(path.trim()));
        }
    });
    result.1.into_iter().flatten().collect::<Result<()>>()
}

/// Deal a changed file after pull. If it's a hardlink, do nothing; otherwise
/// copy it to the device.
async fn dump_changed_file(path: &str) -> Result<()> {
    let path = Path::new(path);
    let info = CONFIG
        .read()
        .unwrap()
        .sync_group
        .0
        .get(path)
        .die(format!("`{:?}` not found in config", path).as_str())
        .clone();
    assert!(path.exists(), "`{:?}` does not exist", path);
    if info.is_hardlink {
        return Ok(());
    }
    let to = info.get_on_device();
    if let Some(to) = to {
        tokio::fs::copy(REPO_PATH.join(path), to).await?;
    }
    Ok(())
}

pub async fn sync_push() -> Result<()> {
    let filemap = &CONFIG.read().unwrap().sync_group.0;
    let result = async_scoped::TokioScope::scope_and_block(move |scope| {
        for path in filemap.keys() {
            scope.spawn(sync_load(path));
        }
    });
    result.1.into_iter().flatten().collect::<Result<()>>()?;

    git(["add", "."])?;
    git(["push", REMOTE_NAME, SYNC_BRANCH])?;
    Ok(())
}

async fn sync_load(path: &Path) -> Result<()> {
    let info = CONFIG
        .read()
        .unwrap()
        .sync_group
        .0
        .get(path)
        .die(format!("`{:?}` not found in config", path).as_str())
        .clone();

    assert!(path.exists(), "`{:?}` does not exist", path);
    if info.is_hardlink {
        return Ok(());
    }

    let from = info.get_on_device();
    if let Some(from) = from {
        tokio::fs::copy(from, REPO_PATH.join(path)).await?;
    }

    Ok(())
}
