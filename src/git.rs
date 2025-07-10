use std::path::Path;

use git2::{IndexAddOption, Repository, Signature};

use crate::error::{GsbError, Result};

pub struct GsbRepo {
    repo: Repository,
}

impl GsbRepo {
    /// 打开一个位于指定路径的 Git 仓库
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Repository::open(path)?;
        Ok(GsbRepo { repo })
    }

    /// 添加所有变更并提交
    pub fn add_and_commit(&self, message: &str) -> Result<()> {
        let mut index = self.repo.index()?;
        // Stage all changes, including untracked files
        index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
        index.write()?;

        let oid = index.write_tree()?;
        let tree = self.repo.find_tree(oid)?;

        // 检查是否有变更
        let head = self.repo.head()?;
        let parent_commit = head.peel_to_commit()?;
        if parent_commit.tree_id() == tree.id() {
            log::info!("No changes to commit.");
            return Ok(());
        }

        let signature = Signature::now("gsb", "gsb@localhost")?; // 可以考虑从 git config 读取
        self.repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&parent_commit],
        )?;

        log::info!("Committed changes with message: {message}");
        Ok(())
    }

    /// 从远程拉取更新
    pub fn pull(&self, remote_name: &str, branch_name: &str) -> Result<()> {
        log::info!("Fetching from remote '{remote_name}'...");
        let mut remote = self.repo.find_remote(remote_name)?;
        remote.fetch(&[branch_name], None, None)?;

        let fetch_head_oid = self.repo.refname_to_id("FETCH_HEAD")?;
        let _fetch_commit = self.repo.find_commit(fetch_head_oid)?;
        let annotated_fetch_commit = self.repo.find_annotated_commit(fetch_head_oid)?;

        let (analysis, _) = self.repo.merge_analysis(&[&annotated_fetch_commit])?;

        if analysis.is_up_to_date() {
            log::info!("Already up-to-date.");
            Ok(())
        } else if analysis.is_fast_forward() {
            log::info!("Fast-forwarding...");
            let ref_name = format!("refs/heads/{branch_name}");
            let mut reference = self.repo.find_reference(&ref_name)?;
            reference.set_target(fetch_head_oid, "Fast-Forward")?;
            self.repo.set_head(&ref_name)?;
            self.repo
                .checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
            log::info!("Pull successful.");
            Ok(())
        } else {
            // 为了简化，我们目前不支持自动合并冲突。
            // 在实际应用中，这里需要更复杂的处理。
            log::warn!("Merge required, but auto-merge is not implemented. Please merge manually.");
            // 或者可以尝试合并
            // let remote_branch_ref =
            // self.repo.find_reference(&format!("refs/remotes/{}/{}", remote_name,
            // branch_name))?; let remote_commit =
            // remote_branch_ref.peel_to_commit()?; let mut index =
            // self.repo.merge_trees(self.repo.head()?.peel_to_tree()?,
            // remote_commit.tree()?, None)?; if index.has_conflicts() { ... }
            Err(GsbError::Git(git2::Error::new(
                git2::ErrorCode::MergeConflict,
                git2::ErrorClass::Merge,
                "Non-fast-forward merge required",
            )))
        }
    }
}
