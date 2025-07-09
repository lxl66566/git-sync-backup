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
    Collect,

    /// Restore all specified items from the git repository to local paths
    #[command(alias = "r")]
    Restore,

    /// Run in background, continuously fetch and restore updates
    #[command(alias = "s")]
    Sync,

    /// Get the device name of current device
    #[command(alias = "d")]
    Device,
}
