#![allow(clippy::unnecessary_debug_formatting)]

use clap::Parser;
use config_file2::LoadConfigFile;
use fuck_backslash::FuckBackslash;
use git_sync_backup::{
    cli::{Cli, Commands},
    config::Config,
    error::Result,
};
use log::warn;

const GSB_CONFIG_FILE_NAME: &str = ".gsb.config.toml";

fn main() {
    git_sync_backup::utils::log_init();

    if let Err(e) = run() {
        // anyhow 错误链用 `{e:#}` 输出，会自动带上 context 链。
        log::error!("Application error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::Device = cli.command {
        println!("{}", git_sync_backup::utils::get_current_device_name()?);
        return Ok(());
    }

    // 找到仓库根目录并加载配置
    let repo_root = git_sync_backup::utils::find_repo_root()?.fuck_backslash();
    log::info!("Found repository root at: {repo_root:?}");
    let config = Config::load(repo_root.join(GSB_CONFIG_FILE_NAME))?.ok_or_else(|| {
        anyhow::anyhow!("Could not find {GSB_CONFIG_FILE_NAME} in repository root: please make sure the current directory is a gsb repository")
    })?;

    if !config.version.is_empty() && config.version != env!("CARGO_PKG_VERSION") {
        warn!(
            "The config file version ({}) != gsb version ({}), there may be compatibility issues, please be careful.",
            config.version,
            env!("CARGO_PKG_VERSION")
        );
    }

    match cli.command {
        Commands::Collect {
            autocommit,
            interactive,
        } => {
            git_sync_backup::ops::handle_collect(&config, &repo_root, autocommit, interactive)?;
        }
        Commands::Restore { interactive } => {
            git_sync_backup::ops::handle_restore(&config, &repo_root, interactive)?;
        }
        Commands::Sync => {
            git_sync_backup::ops::handle_sync(&config, &repo_root)?;
        }
        Commands::Device => {
            unreachable!("handled above");
        }
    }

    Ok(())
}
