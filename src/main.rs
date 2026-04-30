use clap::Parser;
use config_file2::LoadConfigFile;
use fuck_backslash::FuckBackslash;
use git_sync_backup::{
    cli::{Cli, Commands},
    config::Config,
    error::{GsbError, Result},
};
use log::warn;

const GSB_CONFIG_FILE_NAME: &str = ".gsb.config.toml";

fn main() {
    git_sync_backup::utils::log_init();

    if let Err(e) = run() {
        log::error!("Application error: {e}");
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
    let config =
        Config::load(repo_root.join(GSB_CONFIG_FILE_NAME))?.ok_or(GsbError::ConfigNotFound)?;

    if config.version != env!("CARGO_PKG_VERSION") {
        warn!(
            "The config file version ({}) != gsb version ({}), there may be compatibility issues, please be careful.",
            config.version,
            env!("CARGO_PKG_VERSION")
        );
    }

    // 根据子命令执行相应操作
    match cli.command {
        Commands::Collect { autocommit } => {
            git_sync_backup::ops::handle_collect(&config, &repo_root, autocommit)?;
        }
        Commands::Restore => {
            git_sync_backup::ops::handle_restore(&config, &repo_root)?;
        }
        Commands::Sync => {
            git_sync_backup::ops::handle_sync(&config, &repo_root)?;
        }
        _ => unreachable!("handled above"),
    }

    Ok(())
}
