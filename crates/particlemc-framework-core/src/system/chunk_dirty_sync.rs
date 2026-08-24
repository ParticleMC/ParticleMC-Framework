//! tick 管线第六步：脏区块同步（信用节流）。
//!
//! 每 tick 推进各玩家 `ChunkSender` 的区块信用（[`crate::network::client::ChunkSender::tick`]）。
//! 真实的区块字节序列化与 `MapChunk` 发包属于后续阶段（需完整维度 / 注册表
//! 序列化器），本阶段先就位节流机制，避免向客户端发送未就绪的区块数据。

use crate::prelude::ResMut;

use crate::network::client::ClientNetworks;

/// 推进所有在线玩家的区块信用。
pub fn chunk_dirty_sync(mut clients: ResMut<ClientNetworks>) {
    for client in clients.clients.values_mut() {
        client.chunk_sender.tick();
    }
}
