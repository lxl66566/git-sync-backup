mod cli;
mod config;
mod error;
mod git;
mod ops;
mod utils;

use crate::cli::{Cli, Commands};
use crate::config::Config;
use crate::error::{GsbError, Result};
use clap::Parser;
use config_file2::LoadConfigFile;

fn main() {
    utils::log_init();

    if let Err(e) = run() {
        log::error!("Application error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Commands::Device = cli.command {
        println!("{}", utils::get_current_device_name()?);
        return Ok(());
    }

    // 找到仓库根目录并加载配置
    let repo_root = utils::find_repo_root()?;
    log::info!("Found repository root at: {:?}", repo_root);
    let config = Config::load(&repo_root)?.ok_or(GsbError::ConfigNotFound)?;

    // 根据子命令执行相应操作
    match cli.command {
        Commands::Collect => {
            ops::handle_collect(&config, &repo_root)?;
        }
        Commands::Restore => {
            ops::handle_restore(&config, &repo_root)?;
        }
        Commands::Sync => {
            ops::handle_sync(&config, &repo_root)?;
        }
        _ => unreachable!("handled above"),
    }

    Ok(())
}
