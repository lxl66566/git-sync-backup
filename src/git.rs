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
        let repo =
            Repository::open(path).with_context(|| format!("打开 git 仓库失败: {path:?}"))?;
        Ok(GsbRepo { repo })
    }

    /// 添加所有变更并提交。若无变更则跳过。
    pub fn add_and_commit(&self, message: &str) -> Result<()> {
        let mut index = self.repo.index().context("获取 git index 失败")?;
        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .context("git add 失败")?;
        index.write().context("git index write 失败")?;

        let oid = index.write_tree().context("write_tree 失败")?;
        let tree = self.repo.find_tree(oid).context("find_tree 失败")?;

        let head = self.repo.head().context("读取 HEAD 失败")?;
        let parent_commit = head.peel_to_commit().context("解析 commit 失败")?;
        if parent_commit.tree_id() == tree.id() {
            log::info!("No changes to commit.");
            return Ok(());
        }

        // 优先复用仓库默认 signature，失败再 fallback 到 gsb 默认
        let signature = self
            .repo
            .signature()
            .or_else(|_| Signature::now("gsb", "gsb@localhost"))
            .context("获取 git signature 失败")?;
        self.repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent_commit],
            )
            .context("git commit 失败")?;

        log::info!("Committed changes with message: {message}");
        Ok(())
    }

    /// 从远程拉取更新（仅支持 fast-forward，不支持自动解决冲突）。
    pub fn pull(&self, remote_name: &str, branch_name: &str) -> Result<()> {
        log::info!("Fetching from remote '{remote_name}'...");
        let mut remote = self
            .repo
            .find_remote(remote_name)
            .with_context(|| format!("未找到 remote: {remote_name:?}"))?;
        remote
            .fetch(&[branch_name], None, None)
            .with_context(|| format!("fetch {remote_name}/{branch_name} 失败"))?;

        let fetch_head_oid = self
            .repo
            .refname_to_id("FETCH_HEAD")
            .context("读取 FETCH_HEAD 失败")?;
        let annotated_fetch_commit = self
            .repo
            .find_annotated_commit(fetch_head_oid)
            .context("解析 fetch commit 失败")?;

        let (analysis, _) = self
            .repo
            .merge_analysis(&[&annotated_fetch_commit])
            .context("merge_analysis 失败")?;

        if analysis.is_up_to_date() {
            log::info!("Already up-to-date.");
            Ok(())
        } else if analysis.is_fast_forward() {
            log::info!("Fast-forwarding...");
            let ref_name = format!("refs/heads/{branch_name}");
            let mut reference = self
                .repo
                .find_reference(&ref_name)
                .with_context(|| format!("未找到分支: {branch_name:?}"))?;
            reference
                .set_target(fetch_head_oid, "Fast-Forward")
                .context("set_target 失败")?;
            self.repo.set_head(&ref_name).context("set_head 失败")?;
            self.repo
                .checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                .context("checkout_head 失败")?;
            log::info!("Pull successful.");
            Ok(())
        } else {
            log::warn!("Merge required, but auto-merge is not implemented. Please merge manually.");
            Err(anyhow::anyhow!(
                "需要非 fast-forward 合并，gsb 暂不支持自动处理；请手动 merge 后重试"
            ))
        }
    }
}
