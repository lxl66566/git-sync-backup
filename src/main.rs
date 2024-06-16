#![feature(anonymous_lifetime_in_impl_trait)]
mod backup;
mod cli;
mod config;
mod git_command;
mod sync;

use clap::Parser;
use cli::{Cli, CLI};

fn main() {
    CLI.get_or_init(Cli::parse);
}
