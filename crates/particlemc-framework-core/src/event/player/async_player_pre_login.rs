// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 异步玩家预登录事件（feature-gated）。

#[cfg(feature = "async-events")]
use crate::event::r#trait::{AsyncEvent, CancellableEvent, Event};
#[cfg(feature = "async-events")]
use crate::prelude::{Message, SocketAddr};

/// 异步玩家预登录事件。
#[cfg(feature = "async-events")]
#[derive(Message, Debug, Clone)]
pub struct AsyncPlayerPreLogin {
    /// 用户名。
    pub username: String,
    /// 客户端 IP 地址。
    pub ip: SocketAddr,
    /// 是否已取消。
    pub cancelled: bool,
}

#[cfg(feature = "async-events")]
impl Event for AsyncPlayerPreLogin {}

#[cfg(feature = "async-events")]
impl AsyncEvent for AsyncPlayerPreLogin {}

#[cfg(feature = "async-events")]
impl CancellableEvent for AsyncPlayerPreLogin {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
