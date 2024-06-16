use std::path::PathBuf;

use anyhow::Result;
use die_exit::Die;

use crate::{
    config::CONFIG,
    git_command::{git, BACKUP_BRANCH, REMOTE_NAME, SYNC_BRANCH},
};

pub async fn backup() -> Result<()> {
    git(["switch", &BACKUP_BRANCH])?;
    let backup_list = &CONFIG.read().unwrap().backup_group.0;
    let result = async_scoped::TokioScope::scope_and_block(move |scope| {
        for path in backup_list.keys() {
            scope.spawn(backup_file(path));
        }
    });

    result.1.into_iter().flatten().collect::<Result<()>>()?;
    git(["add", "."])?;
    git(["push", REMOTE_NAME, SYNC_BRANCH])?;
    Ok(())
}

async fn backup_file(path: &PathBuf) -> Result<()> {
    todo!()
}
