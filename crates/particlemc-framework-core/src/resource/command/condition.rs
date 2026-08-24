// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令条件闸门（见 `.specs/implement-command-framework/`）。

use crate::resource::command::sender::CommandSender;

/// 命令条件：执行前判定发送者是否可用该命令（权限 / 可见性闸门）。
///
/// `can_use` 返回 false 时管理器返回 `CommandResult::Cancelled` 并拒绝执行，
/// 不调用执行器。命令级（`Command::condition`）与语法级（`CommandSyntax::condition`）
/// 均可设置，二者任一拒绝即取消。
pub trait CommandCondition: Send + Sync {
    /// 返回发送者是否可使用；`command_string` 为原始命令串（可选上下文）。
    fn can_use(&self, sender: &dyn CommandSender, command_string: Option<&str>) -> bool;
}
