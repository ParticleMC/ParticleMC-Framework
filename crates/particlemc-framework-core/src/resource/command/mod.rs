// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令框架子模块（语法驱动 + 有类型参数，对齐 Minestom `command/**`）。
//!
//! 见 `.specs/implement-command-framework/`。本模块承载命令构建、参数体系、
//! 解析上下文、执行结果码、条件闸门、参数异常、补全回调与发送者抽象；
//! 管理器（`manager`）提供完整 API 与解析器，并作为 旧 ECS 方案 `Resource` 注入。

pub mod argument;
// 子模块 `command` 与父目录同名（`resource/command/command.rs`）属于命名巧合，
// 非递归包含，放宽 `module_inception`（见 `.specs/implement-command-framework/`）。
#[allow(clippy::module_inception)]
pub mod command;
pub mod condition;
pub mod context;
pub mod error;
pub mod manager;
pub mod sender;
pub mod suggestion;

pub use argument::{
    AnyArgument, Argument, ArgumentCallback, ArgumentParserType, ArgumentType, EntitySelector,
    EntitySelectorType, RelativeVec3, collect_suggestion,
};
pub use command::{Command, CommandExecutor, CommandSyntax};
pub use condition::CommandCondition;
pub use context::CommandContext;
pub use error::{ArgumentSyntaxException, IllegalCommandStructureException};
pub use manager::{
    CommandError, CommandManager, CommandResult, CommandResultType, ParseResult,
    UnknownCommandCallback,
};
pub use sender::{CommandSender, ConsoleSender, PlayerSender, ServerSender};
pub use suggestion::{Suggestion, SuggestionCallback, SuggestionEntry};
