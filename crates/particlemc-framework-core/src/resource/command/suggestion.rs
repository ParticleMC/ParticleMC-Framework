// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令补全回调 API（框架侧，见 `.specs/implement-command-framework/`）。
//!
//! 网络下发（Tab 补全包）超出范围，仅实现回调类型与收集入口。

use crate::resource::command::context::CommandContext;
use crate::resource::command::sender::CommandSender;

/// 单个补全候选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionEntry {
    /// 候选替换文本。
    pub value: String,
    /// 悬停提示（可选）。
    pub tooltip: Option<String>,
}

/// 补全结果集合。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suggestion {
    /// 候选列表。
    pub entries: Vec<SuggestionEntry>,
}

impl Suggestion {
    /// 构造空补全。
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// 追加一条无提示候选。
    pub fn push(&mut self, value: &str) {
        self.entries.push(SuggestionEntry {
            value: value.to_string(),
            tooltip: None,
        });
    }

    /// 追加一条带提示候选。
    pub fn push_with_tooltip(&mut self, value: &str, tooltip: &str) {
        self.entries.push(SuggestionEntry {
            value: value.to_string(),
            tooltip: Some(tooltip.to_string()),
        });
    }

    /// 是否无候选。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 补全回调：参数可注册自己的补全逻辑（框架侧）。
///
/// `emit` 预留用于回发诊断文本（当前网络下发超出范围，不主动使用）。
pub trait SuggestionCallback: Send + Sync {
    /// 收集补全候选。
    fn apply(
        &self,
        _sender: &dyn CommandSender,
        _context: &CommandContext,
        _suggestion: &mut Suggestion,
        _emit: &mut dyn FnMut(&str),
    );
}
