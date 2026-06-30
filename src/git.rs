//! git 操作封装。

use std::path::Path;

use git2::{IndexAddOption, Repository, Signature};

use crate::error::{Context, Result};

pub struct GsbRepo {
    repo: Repository,
}

impl GsbRepo {
    /// 打开一个位于指定路径的 Git 仓库。
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Repository::open(path)
            .with_context(|| format!("Failed to open git repository: {path:?}"))?;
        Ok(GsbRepo { repo })
    }

    /// 添加所有变更并提交。若无变更则跳过。
    pub fn add_and_commit(&self, message: &str) -> Result<()> {
        let mut index = self.repo.index().context("Failed to get git index")?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .context("git add failed")?;
        index.write().context("git index write failed")?;

        let oid = index.write_tree().context("write_tree failed")?;
        let tree = self.repo.find_tree(oid).context("find_tree failed")?;

        let head = self.repo.head().context("Failed to read HEAD")?;
        let parent_commit = head.peel_to_commit().context("Failed to parse commit")?;
        if parent_commit.tree_id() == tree.id() {
            log::info!("No changes to commit.");
            return Ok(());
        }

        // 优先复用仓库默认 signature，失败再 fallback 到 gsb 默认
        let signature = self
            .repo
            .signature()
            .or_else(|_| Signature::now("gsb", "gsb@localhost"))
            .context("Failed to get git signature")?;
        self.repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent_commit],
            )
            .context("git commit failed")?;

        log::info!("Committed changes with message: {message}");
        Ok(())
    }

    /// 从远程拉取更新（仅支持 fast-forward，不支持自动解决冲突）。
    pub fn pull(&self, remote_name: &str, branch_name: &str) -> Result<()> {
        log::info!("Fetching from remote '{remote_name}'...");
        let mut remote = self
            .repo
            .find_remote(remote_name)
            .with_context(|| format!("Remote not found: {remote_name:?}"))?;
        remote
            .fetch(&[branch_name], None, None)
            .with_context(|| format!("Failed to fetch {remote_name}/{branch_name}"))?;

        let fetch_head_oid = self
            .repo
            .refname_to_id("FETCH_HEAD")
            .context("Failed to read FETCH_HEAD")?;
        let annotated_fetch_commit = self
            .repo
            .find_annotated_commit(fetch_head_oid)
            .context("Failed to parse fetch commit")?;

        let (analysis, _) = self
            .repo
            .merge_analysis(&[&annotated_fetch_commit])
            .context("merge_analysis failed")?;

        if analysis.is_up_to_date() {
            log::info!("Already up-to-date.");
            Ok(())
        } else if analysis.is_fast_forward() {
            log::info!("Fast-forwarding...");
            let ref_name = format!("refs/heads/{branch_name}");
            let mut reference = self
                .repo
                .find_reference(&ref_name)
                .with_context(|| format!("Branch not found: {branch_name:?}"))?;
            reference
                .set_target(fetch_head_oid, "Fast-Forward")
                .context("set_target failed")?;
            self.repo.set_head(&ref_name).context("set_head failed")?;
            self.repo
                .checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                .context("checkout_head failed")?;
            log::info!("Pull successful.");
            Ok(())
        } else {
            log::warn!("Merge required, but auto-merge is not implemented. Please merge manually.");
            Err(anyhow::anyhow!(
                "Non-fast-forward merge required; gsb does not support automatic merge. Please merge manually and retry."
            ))
        }
    }
}
