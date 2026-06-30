//! 错误处理
//!
//! 本项目作为 CLI 工具，统一使用 [`anyhow::Error`] 作为错误类型。其优势：
//! - 任意位置可用 `.context(...)` / `.with_context(...)` 附加上下文，错误信息
//!   对最终用户更友好。
//! - 调用方无需为不同的错误变体定义 enum，但仍可在底层用 `GsbError` 这类
//!   类型化错误表示领域内关键错误（通过 `#[source]` 自动转换）。
//!
//! 这里 [`Result`]`<T>` 等价于 `Result<T, anyhow::Error>`，所有公共 API
//! 都使用它，避免在每个模块重复写 `anyhow::Result`。

pub use anyhow::{Context, Result as AnyhowResult};

/// 项目统一的 `Result` 别名，错误类型固定为 [`anyhow::Error`]。
pub type Result<T> = AnyhowResult<T>;
