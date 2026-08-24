// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 网络桥接：`tokio` 异步监听 ↔ `旧 ECS 方案` 同步游戏循环的连接点。
//!
//! `inbound` 由监听任务持有发送端；`outbound` 由监听任务在连接建立时插入
//! 本连接的发送端，关闭时移除。`network_send` 在 tick 末经 `outbound` 将
//! `ClientNetwork` 队列中的字节写给对应连接。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use tokio::sync::mpsc::{Receiver, Sender};

use crate::network::listener::{OutboundMap, RawFrame};

/// 游戏侧持有的网络桥接资源。
pub struct NetworkBridge {
    /// 入站帧接收端（监听 → 游戏）。
    pub inbound: Receiver<RawFrame>,
    /// 出站通道表（游戏 → 监听，按 conn_id 路由）。
    pub outbound: OutboundMap,
}

impl NetworkBridge {
    /// 构造桥接。首次实现中为函数式构造；保留 `new` 以便插件装配。
    pub fn new(inbound: Receiver<RawFrame>, outbound: OutboundMap) -> Self {
        Self { inbound, outbound }
    }
}

/// 构造一个空桥接（用于测试或手动装配）。
pub fn empty_bridge() -> (NetworkBridge, Sender<RawFrame>, OutboundMap) {
    let (tx, rx) = tokio::sync::mpsc::channel(1024);
    let outbound: OutboundMap = Arc::new(Mutex::new(HashMap::new()));
    (NetworkBridge::new(rx, outbound.clone()), tx, outbound)
}

/// 占位默认桥接：构造空入站通道（发送端立即丢弃），出站表为空。
///
/// 仅用于满足 `Res<NetworkBridge>` 系统参数的 `init_resource` 惰性初始化约束；
/// 实际运行由 `App` 经 `empty_bridge`/`new` 注入真实桥接资源。
impl Default for NetworkBridge {
    fn default() -> Self {
        let (_tx, rx) = tokio::sync::mpsc::channel(1024);
        NetworkBridge {
            inbound: rx,
            outbound: OutboundMap::default(),
        }
    }
}
