use std::{path::PathBuf, sync::OnceLock};

use clap::{Parser, Subcommand, ValueEnum};

pub static CLI: OnceLock<Cli> = OnceLock::new();

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None, after_help = r#"Examples:
"#)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct Cli {
    /// Encrypt, Decrypt and Add
    #[command(subcommand)]
    pub command: SubCommand,
    /// Repository path
    #[arg(short, long, global = true)]
    pub repo: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone, Default)]
pub enum SubCommand {
    /// Sync all files in sync group.
    #[default]
    #[clap(alias("s"))]
    Sync,
    /// Add files to a group.
    Add {
        #[clap(required = true)]
        paths: Vec<String>,
        #[clap(short, long)]
        group: Option<Group>,
    },
    /// Init the backup repository in specified path.
    Init { path: Option<PathBuf> },
}

/// What group the file should be add to, Backup or Sync.
#[derive(ValueEnum, Debug, Clone, Default)]
pub enum Group {
    #[default]
    #[clap(alias("b"))]
    Backup,
    #[clap(alias("s"))]
    Sync,
}
