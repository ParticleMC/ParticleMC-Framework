// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令参数解析异常与命令结构异常（见 `.specs/implement-command-framework/`）。

use std::fmt;

/// 参数语法异常：参数解析失败（类型不符、越界、限制不符等）时抛出。
///
/// 对齐 Minestom `ArgumentSyntaxException`：仅携带 `input`（出错原文）与
/// `error_code`（非零整型分类），不捕获栈帧（本实现直接构造，无栈开销）。
/// 解析失败时由参数 `parse_erased` 返回，供管理器 emit 默认或自定义错误消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentSyntaxException {
    /// 出错时的原始输入片段。
    pub input: String,
    /// 错误分类码（非零；0 表示未分类）。
    pub error_code: u32,
    /// 人类可读描述。
    pub message: String,
}

impl ArgumentSyntaxException {
    /// 构造一个异常。
    pub fn new(input: &str, error_code: u32, message: &str) -> Self {
        Self {
            input: input.to_string(),
            error_code,
            message: message.to_string(),
        }
    }
}

impl fmt::Display for ArgumentSyntaxException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "参数解析失败 (code={}): {} (input={})",
            self.error_code, self.message, self.input
        )
    }
}

impl std::error::Error for ArgumentSyntaxException {}

/// 命令结构非法异常（构建期）：如参数 id 重复、语法为空等。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IllegalCommandStructureException(pub String);

impl fmt::Display for IllegalCommandStructureException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "命令结构非法：{}", self.0)
    }
}

impl std::error::Error for IllegalCommandStructureException {}
