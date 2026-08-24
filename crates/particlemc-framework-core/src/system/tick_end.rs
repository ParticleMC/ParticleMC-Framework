// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线第七步：占位阶段。
//!
//! 实际出站发包统一经 `ClientNetwork` 队列在 `network_send` 中 flush，本阶段
//! 不再产生任何出站事件；仅保留系统以兼容既有调度链。

use crate::prelude::Commands;

/// tick 末占位（出站已交由 `network_send` 处理）。
pub fn tick_end(_commands: Commands) {}
