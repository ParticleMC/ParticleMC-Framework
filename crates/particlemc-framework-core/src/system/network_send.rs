// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线末段：将 `ClientNetwork` 队列中的出站帧刷写到 socket。
//!
//! 经 `NetworkBridge.outbound` 把每在线玩家 `ClientNetwork` 中的 urgent / normal
//! 队列按三层模型刷出，随后清空队列（含 urgent 紧急窗口递减）。

use crate::prelude::{Res, ResMut};

use crate::network::bridge::NetworkBridge;
use crate::network::client::{ClientNetworks, flush_all};

/// tick 末网络阶段：对每个在线玩家 flush 出站帧，然后清空其队列。
pub fn network_send(mut clients: ResMut<ClientNetworks>, bridge: Res<NetworkBridge>) {
    flush_all(&clients, &bridge);
    for client in clients.clients.values_mut() {
        client.clear_queues();
    }
}
