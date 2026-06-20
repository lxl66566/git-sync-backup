use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about = "A git-based sync/backup tool.", long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Collect all specified items into the git repository
    #[command(alias = "c")]
    Collect {
        /// Whether to automatically commit the changes
        #[arg(short, long)]
        autocommit: bool,
    },

    /// Restore all specified items from the git repository to local paths
    #[command(alias = "r")]
    Restore {
        /// Skip the confirmation prompt (dry-run summary).
        ///
        /// 默认情况下 `gsb r` 会先打印将要 restore 的文件列表并等待用户确认。
        /// 传入此选项可直接执行，适用于脚本或 `gsb sync` 后台模式。
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Run in background, continuously fetch and restore updates
    #[command(alias = "s")]
    Sync,

    /// Get the device name of current device
    #[command(alias = "d")]
    Device,
}
