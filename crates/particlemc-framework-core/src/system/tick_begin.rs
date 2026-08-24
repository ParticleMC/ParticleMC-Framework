//! tick 管线第一步：标记一个逻辑 tick 的开始。
//!
//! 负责清理入站帧缓冲区（`NetworkBridge::inbound`），防止上一 tick 遗留的
//! 帧堆积；同时预留扩展点供后续 tick 开始逻辑接入。

use crate::prelude::{Commands, ResMut};

use crate::network::bridge::NetworkBridge;

/// 标记一个逻辑 tick 的开始，并清空入站帧缓冲区。
pub fn tick_begin(mut bridge: ResMut<NetworkBridge>, _commands: Commands) {
    // 清空上一 tick 累积的入站帧，避免重复处理。
    while bridge.inbound.try_recv().is_ok() {}
}
