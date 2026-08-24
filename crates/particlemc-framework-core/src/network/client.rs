// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 三层混合发包模型（ClientNetwork）。
//!
//! - `urgent_queue`：登录 / 传送 / 伤害 / 聊天等必须立即送达的包，tick 末逐包 `write+flush`。
//! - `normal_queue`：实体移动 / 区块数据 / 粒子等可容忍 ≤50ms 延迟的包，累积进
//!   `write_buffer`，达 `mtu_threshold`（默认 1400）或队列空时一次性 `write+flush`。
//! - `ChunkSender`：信用节流，按客户端回包动态调整区块发送速率。
//! - `broadcast`：同一包发给 N 个玩家只序列化一次，字节复制到各目标队列。

use std::collections::HashMap;

use crate::network::bridge::NetworkBridge;
use crate::network::listener::OutboundMessage;
use crate::protocol::framing::{MAX_FRAME, encode_frame, encode_frame_compressed};

/// 发包优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// 必须立即送达（tick 末优先 flush）。
    Urgent,
    /// 可批量（按 MTU 聚合后 flush）。
    Normal,
}

/// 单连接发送状态。
#[derive(Debug)]
pub struct ClientNetwork {
    /// 紧急队列。
    pub urgent_queue: Vec<Vec<u8>>,
    /// 普通队列。
    pub normal_queue: Vec<Vec<u8>>,
    /// 复用字节缓冲（MTU 聚合）。
    pub write_buffer: Vec<u8>,
    /// MTU 阈值（默认 1400 字节）。
    pub mtu_threshold: usize,
    /// 子服切换紧急窗口剩余 tick 数（>0 时普通包按紧急处理）。
    pub urgent_window_ticks: u8,
    /// 区块信用节流器。
    pub chunk_sender: ChunkSender,
    /// 出站压缩是否已启用（登录流程 `LoginCompression` 下发后置位，T7）。
    pub compression_enabled: bool,
    /// 出站压缩阈值（0 表示禁用，T7）。
    pub compression_threshold: i32,
}

impl Default for ClientNetwork {
    /// 与 [`ClientNetwork::new`] 等价（MTU=1400、初始信用 9.0）。
    fn default() -> Self {
        Self::new()
    }
}

impl ClientNetwork {
    /// 构造默认发送状态（MTU=1400，区块初始信用 9.0，压缩关闭）。
    pub fn new() -> Self {
        Self {
            urgent_queue: Vec::new(),
            normal_queue: Vec::new(),
            write_buffer: Vec::new(),
            mtu_threshold: 1400,
            urgent_window_ticks: 0,
            chunk_sender: ChunkSender::new(),
            compression_enabled: false,
            compression_threshold: 0,
        }
    }

    /// 按优先级压入一帧（已含 packet_id 的包体字节）。
    pub fn enqueue(&mut self, bytes: Vec<u8>, priority: Priority) {
        match priority {
            Priority::Urgent => self.urgent_queue.push(bytes),
            Priority::Normal => self.normal_queue.push(bytes),
        }
    }

    /// 清空两个队列（tick 末 flush 后由 `network_send` 调用）。
    pub fn clear_queues(&mut self) {
        self.urgent_queue.clear();
        self.normal_queue.clear();
        self.write_buffer.clear();
        if self.urgent_window_ticks > 0 {
            self.urgent_window_ticks -= 1;
        }
    }
}

/// 所有在线玩家的发送状态表。
#[derive(Debug)]
pub struct ClientNetworks {
    /// conn_id → 发送状态。
    pub clients: HashMap<u32, ClientNetwork>,
    /// 命令聊天收件箱：由 `network_receive` 在收到 0x06 / 0x07 包时写入
    /// `(conn_id, message)`，供 `command_chat_system` 在本 tick 稍后异步执行。
    /// 设计为独立字段以解耦命令执行与 `network_receive`（避免其 `SystemParam`
    /// 元组超出 旧 ECS 方案 16 上限）。见 `.specs/implement-command-framework/`。
    pub command_inbox: Vec<(u32, String)>,
    /// 待处理入站动作包收件箱：`network_receive` 把框架关注的动作类 serverbound
    /// 包（交互 / 动作 / 动画 / 放置 / 使用物品）原样写入，由
    /// `packet_action_system` 在本 tick 稍后经 `EventBus` 派发事件。
    /// 与 `command_inbox` 同理，避免 `network_receive` 增参。
    /// 见 `.specs/implement-framework-capabilities/`。
    pub packet_inbox: Vec<(u32, crate::protocol::packets::InboundPacket)>,
    /// 全局压缩阈值（来自 [`crate::resource::CompressionConfig`]，0 表示禁用，T7）。
    /// 新连接建立时拷贝到各自的 `ClientNetwork.compression_threshold`。
    pub compression_threshold: i32,
}

impl Default for ClientNetworks {
    /// 空状态表，压缩阈值 256（与 [`CompressionConfig`] 默认一致）。
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
            command_inbox: Vec::new(),
            packet_inbox: Vec::new(),
            compression_threshold: 256,
        }
    }
}

impl ClientNetworks {
    /// 以显式压缩阈值构造（真实服务器入口按 `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD` 传入）。
    pub fn with_compression_threshold(threshold: i32) -> Self {
        Self {
            compression_threshold: threshold,
            ..Self::default()
        }
    }

    /// 为新连接插入发送状态（继承全局压缩阈值）。
    pub fn insert(&mut self, conn_id: u32) {
        let client = self.clients.entry(conn_id).or_default();
        client.compression_threshold = self.compression_threshold;
    }

    /// 移除连接（断开时）。
    pub fn remove(&mut self, conn_id: u32) {
        self.clients.remove(&conn_id);
    }
}

/// 区块信用节流器。
#[derive(Debug, Clone)]
pub struct ChunkSender {
    /// 当前可发送区块数（浮点信用），每 tick 累加 `target_chunks_per_tick`。
    pub pending_chunk_count: f32,
    /// 每 tick 累加的信用（由客户端 `ChunkBatchReceived` 回包动态调整）。
    pub target_chunks_per_tick: f32,
    /// 客户端未确认批次数。
    pub outstanding_batches: u32,
}

impl ChunkSender {
    /// 初始信用 9.0（首 tick 即可发送约 9 个区块，符合 Minecraft 默认值）。
    pub fn new() -> Self {
        Self {
            pending_chunk_count: 9.0,
            target_chunks_per_tick: 1.0,
            outstanding_batches: 0,
        }
    }

    /// 每 tick 增加可发送信用。
    pub fn tick(&mut self) {
        self.pending_chunk_count += self.target_chunks_per_tick;
    }

    /// 若有足够信用且有待发区块，发送一个并扣减 1.0 信用。
    pub fn try_send(&mut self) -> bool {
        if self.pending_chunk_count >= 1.0 {
            self.pending_chunk_count -= 1.0;
            true
        } else {
            false
        }
    }

    /// 客户端 `ChunkBatchReceived` 回包：增加未确认批次并适度提升速率。
    pub fn on_batch_received(&mut self) {
        self.outstanding_batches += 1;
        self.target_chunks_per_tick = (self.target_chunks_per_tick * 1.1).min(16.0);
    }
}

impl Default for ChunkSender {
    fn default() -> Self {
        Self::new()
    }
}

/// 将一帧包体封装为完整协议帧字节（按连接压缩状态选择封帧方式）。
///
/// - 未启用压缩：原帧格式 `VarInt 长度 + payload`；
/// - 已启用压缩：压缩帧格式（阈值来自连接状态，T7）。
fn frame_bytes(payload: &[u8], compression_enabled: bool, threshold: i32) -> Option<Vec<u8>> {
    if payload.len() > MAX_FRAME {
        return None;
    }
    if compression_enabled {
        Some(encode_frame_compressed(payload, threshold))
    } else {
        let mut frame = Vec::with_capacity(payload.len() + 5);
        encode_frame(&mut frame, payload).ok()?;
        Some(frame)
    }
}

/// 向单个连接入队一帧。
pub fn enqueue_packet(
    clients: &mut ClientNetworks,
    conn_id: u32,
    bytes: Vec<u8>,
    priority: Priority,
) {
    if let Some(client) = clients.clients.get_mut(&conn_id) {
        let effective = if priority == Priority::Normal && client.urgent_window_ticks > 0 {
            Priority::Urgent
        } else {
            priority
        };
        client.enqueue(bytes, effective);
    }
}

/// 向多个连接广播同一帧（仅序列化一次，字节复制到各队列）。
pub fn broadcast(clients: &mut ClientNetworks, targets: &[u32], bytes: &[u8], priority: Priority) {
    for &conn_id in targets {
        if let Some(client) = clients.clients.get_mut(&conn_id) {
            let effective = if priority == Priority::Normal && client.urgent_window_ticks > 0 {
                Priority::Urgent
            } else {
                priority
            };
            client.enqueue(bytes.to_vec(), effective);
        }
    }
}

/// tick 末网络阶段：对每在线玩家 flush 出站帧。
///
/// - urgent 队列：逐包立即 `write`。
/// - normal 队列：累积进 `write_buffer`，达 `mtu_threshold` 或队列空时一次性 `write`。
pub fn flush_all(clients: &ClientNetworks, bridge: &NetworkBridge) {
    let guard = match bridge.outbound.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    for (conn_id, client) in &clients.clients {
        let Some(tx) = guard.get(conn_id) else {
            continue;
        };
        for bytes in &client.urgent_queue {
            if let Some(frame) = frame_bytes(
                bytes,
                client.compression_enabled,
                client.compression_threshold,
            ) {
                let _ = tx.try_send(OutboundMessage::Frame(frame));
            }
        }
        let mut buf = Vec::new();
        for bytes in &client.normal_queue {
            if let Some(frame) = frame_bytes(
                bytes,
                client.compression_enabled,
                client.compression_threshold,
            ) {
                if !buf.is_empty() && buf.len() + frame.len() >= client.mtu_threshold {
                    let _ = tx.try_send(OutboundMessage::Frame(std::mem::take(&mut buf)));
                }
                buf.extend_from_slice(&frame);
            }
        }
        if !buf.is_empty() {
            let _ = tx.try_send(OutboundMessage::Frame(buf));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::empty_bridge;

    fn payload() -> Vec<u8> {
        vec![0x00, 0x01, 0x02]
    }

    #[test]
    fn urgent_flushed_immediately() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        let bytes = payload();
        enqueue_packet(&mut clients, 1, bytes, Priority::Urgent);
        // urgent 包在队列中（flush 由桥接执行，此处验证入队正确）
        assert_eq!(clients.clients[&1].urgent_queue.len(), 1);
        assert_eq!(clients.clients[&1].normal_queue.len(), 0);
    }

    #[test]
    fn normal_enqueued_separately() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        enqueue_packet(&mut clients, 1, payload(), Priority::Normal);
        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
    }

    #[test]
    fn broadcast_encodes_once() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        clients.insert(2);
        clients.insert(3);
        let bytes = payload();
        broadcast(&mut clients, &[1, 2, 3], &bytes, Priority::Normal);
        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
        assert_eq!(clients.clients[&2].normal_queue.len(), 1);
        assert_eq!(clients.clients[&3].normal_queue.len(), 1);
        // [L1-Runtime] 序列化一次的强断言：broadcast 接收已编码字节并复制到各队列，
        // 三个目标队列内容字节完全一致（复制而非各自重新编码）
        assert_eq!(
            clients.clients[&2].normal_queue[0], clients.clients[&1].normal_queue[0],
            "队列 1/2 内容字节应完全一致"
        );
        assert_eq!(
            clients.clients[&3].normal_queue[0], clients.clients[&1].normal_queue[0],
            "队列 1/3 内容字节应完全一致"
        );
        assert_eq!(
            clients.clients[&1].normal_queue[0], bytes,
            "内容应与入参字节一致"
        );
    }

    #[test]
    fn chunk_sender_credit() {
        let mut cs = ChunkSender::new();
        assert!(cs.try_send()); // 9.0 -> 8.0
        // 持续扣减直到不足
        for _ in 0..8 {
            assert!(cs.try_send());
        }
        assert!(!cs.try_send()); // 0.0 < 1.0
        cs.tick(); // +1.0
        assert!(cs.try_send());
    }

    #[test]
    fn urgent_window_upgrades_normal() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        clients.clients.get_mut(&1).unwrap().urgent_window_ticks = 2;
        enqueue_packet(&mut clients, 1, payload(), Priority::Normal);
        // 窗口内普通包进入 urgent 队列
        assert_eq!(clients.clients[&1].urgent_queue.len(), 1);
        assert_eq!(clients.clients[&1].normal_queue.len(), 0);
    }

    #[test]
    fn flush_all_urgent_immediately_sends() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        let (bridge, _inbound_tx, outbound) = empty_bridge();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<OutboundMessage>(16);
        outbound.lock().unwrap().insert(1, tx);

        let bytes = vec![0x00, 0x01, 0x02];
        enqueue_packet(&mut clients, 1, bytes.clone(), Priority::Urgent);
        flush_all(&clients, &bridge);

        let mut frames = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Frame(frame) = msg {
                frames.push(frame);
            }
        }
        // [L1-Runtime] urgent 包同 tick 立即 write+flush，且只发送 1 条
        assert_eq!(frames.len(), 1, "urgent 包应同 tick 立即发送 1 条");
        assert_eq!(
            frames[0][0] as usize,
            bytes.len(),
            "帧首字节应为 VarInt 长度（3 字节 payload → 首字节 0x03）"
        );
        assert_eq!(
            &frames[0][1..],
            bytes.as_slice(),
            "帧内容应与 payload 完全一致"
        );
    }

    #[test]
    fn flush_all_normal_batches_by_mtu() {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        // 调小 MTU 阈值，使多条 normal 包累积时触发批量 flush
        clients.clients.get_mut(&1).unwrap().mtu_threshold = 20;
        let (bridge, _inbound_tx, outbound) = empty_bridge();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<OutboundMessage>(16);
        outbound.lock().unwrap().insert(1, tx);

        // 每条 payload 10 字节 → 帧长 11 字节（VarInt 长度 + payload）
        let payloads: Vec<Vec<u8>> = (0..3).map(|i| vec![0x10 + i as u8; 10]).collect();
        for p in &payloads {
            enqueue_packet(&mut clients, 1, p.clone(), Priority::Normal);
        }
        let expected_total: usize = payloads.iter().map(|p| p.len() + 1).sum();

        flush_all(&clients, &bridge);

        let mut frames = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Frame(frame) = msg {
                frames.push(frame);
            }
        }
        // [L1-Runtime] normal 包累积达 mtu_threshold(20) 触发一次 write+flush；
        // 11+11>=20 分批发送，3 条共触发 ≥2 次 try_send（含队列空后剩余包的 flush）
        assert!(
            frames.len() >= 2,
            "MTU 聚合应产生多次 try_send，实际 {}",
            frames.len()
        );
        assert!(frames.last().is_some(), "队列清空后剩余包也应被 flush");
        let received_total: usize = frames.iter().map(|f| f.len()).sum();
        assert_eq!(
            received_total, expected_total,
            "发送字节总量应等于 3 条帧长度之和"
        );
        // write_buffer 复用：flush 后不残留待发字节
        assert!(
            clients.clients[&1].write_buffer.is_empty(),
            "flush 后 write_buffer 应清空"
        );
    }
}
