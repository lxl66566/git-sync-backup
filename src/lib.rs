#![warn(clippy::all, clippy::pedantic)]
// 以下 lint 噪声较大但对本项目价值有限，按需 allow。
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::implicit_hasher)]
// 路径统一使用 {:?} 输出，避免 Windows 路径中的特殊字符问题。
#![allow(clippy::unnecessary_debug_formatting)]

pub mod cli;
pub mod config;
pub mod error;
pub mod git;
pub mod ops;
pub mod utils;
pub mod vars;
