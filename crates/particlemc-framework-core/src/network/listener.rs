// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 真实 TCP 监听与帧分发。
//!
//! [`ConnectionListener::start`] 绑定端口，每连接分配 `conn_id` 并建立读写任务：
//! 读任务按 Minecraft 帧格式（VarInt 长度 + 包体）解析出 `packet_id` 与包体，
//! 推入 `inbound` 通道；写任务从本连接的出站通道取字节写回 socket。
//!
//! 监听任务不持有 `World`；连接状态由监听侧维护（仅用于给上报帧打标签），
//! 真正的状态推进由 `system::network_receive` 在游戏侧落 `ConnectionManager`。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Receiver, Sender};

use crate::network::connection::ConnectionState;
use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::framing::{MAX_FRAME, decode_frame_compressed};
use crate::protocol::packet::Packet;
use crate::protocol::packets::Intention;

/// 监听任务上报给游戏侧的帧。
#[derive(Debug)]
pub enum RawFrame {
    /// 一帧入站数据包。
    Packet {
        /// 连接标识。
        conn_id: u32,
        /// 收到该包时连接所处的状态（用于派发）。
        state: ConnectionState,
        /// 包 id。
        packet_id: i32,
        /// 包体（不含 packet_id）。
        payload: Vec<u8>,
    },
    /// 连接关闭。
    Closed(u32),
}

/// 出站通道消息：数据帧或服务端主动关闭。
#[derive(Debug)]
pub enum OutboundMessage {
    /// 数据帧（已含帧头与长度前缀）。
    Frame(Vec<u8>),
    /// 启用该连接入站压缩（T7：`LoginCompression` 下发后由游戏侧发送，
    /// 写任务置位读侧压缩标志，使后续入站按压缩帧格式解帧）。
    EnableCompression,
    /// 服务端主动断开连接（写任务发送 FIN 后结束）。
    Close,
}

/// 出站通道表（conn_id → 发送端）。用 `Arc<Mutex>` 在异步写任务与同步 旧 ECS 方案
/// 系统间安全共享（功能等价于冻结契约中的 `NetworkBridge.outbound`）。
pub type OutboundMap = Arc<Mutex<HashMap<u32, Sender<OutboundMessage>>>>;

/// 连接状态表（监听侧维护，仅用于帧标签）。
type ConnStateMap = Arc<Mutex<HashMap<u32, ConnectionState>>>;

/// 真实连接监听器（取代骨架阶段的占位 trait）。
pub struct ConnectionListener;

impl ConnectionListener {
    /// 绑定 `addr` 并启动监听循环，返回 join handle。
    ///
    /// - `inbound_tx`：上报帧的发送端（接入 `NetworkBridge`）。
    /// - `outbound`：出站通道表；每连接建立时插入本连接的发送端，关闭时移除。
    pub async fn start(
        addr: SocketAddr,
        inbound_tx: Sender<RawFrame>,
        outbound: OutboundMap,
    ) -> std::io::Result<tokio::task::JoinHandle<()>> {
        let listener = TcpListener::bind(addr).await?;
        let next_id = Arc::new(AtomicU32::new(1));
        let conn_states: ConnStateMap = Arc::new(Mutex::new(HashMap::new()));
        let handle = tokio::spawn(async move {
            loop {
                let (socket, _peer) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let conn_id = next_id.fetch_add(1, Ordering::Relaxed);
                let (out_tx, out_rx) = mpsc::channel::<OutboundMessage>(256);
                if let Ok(mut map) = outbound.lock() {
                    map.insert(conn_id, out_tx);
                }
                if let Ok(mut map) = conn_states.lock() {
                    map.insert(conn_id, ConnectionState::Handshake);
                }
                let ib = inbound_tx.clone();
                let ob = outbound.clone();
                let cs = conn_states.clone();
                tokio::spawn(handle_connection(socket, conn_id, ib, out_rx, ob, cs));
            }
        });
        Ok(handle)
    }
}

/// 尝试从缓冲起始解析帧长度（VarInt），不足则返回 `None`。
fn try_parse_frame_length(data: &[u8]) -> Option<(usize, usize)> {
    let mut pos = 0;
    let mut result: i32 = 0;
    for i in 0..5 {
        if pos >= data.len() {
            return None;
        }
        let byte = data[pos];
        pos += 1;
        result |= i32::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            let len = usize::try_from(result).ok()?;
            return Some((len, pos));
        }
    }
    None
}

/// 依据收到的包更新监听侧连接状态（仅影响后续帧标签）。
fn update_state(
    cs: &ConnStateMap,
    conn_id: u32,
    state: ConnectionState,
    packet_id: i32,
    body: &[u8],
) {
    let next = match state {
        ConnectionState::Handshake => {
            if packet_id == 0x00 {
                // 注意：此处必须用「不含 packet_id」的包体解码（与 dispatch 一致），
                // 否则首字节被当作协议版本、后续字节错位，解码失败导致状态永不推进。
                if let Ok(intention) = Intention::decode(&mut ByteBuffer::new(body.to_vec())) {
                    if intention.next_state == 1 {
                        ConnectionState::Status
                    } else if intention.next_state == 2 {
                        ConnectionState::Login
                    } else {
                        state
                    }
                } else {
                    state
                }
            } else {
                state
            }
        }
        ConnectionState::Login => {
            if packet_id == 0x03 {
                ConnectionState::Configuration
            } else {
                state
            }
        }
        ConnectionState::Configuration => {
            if packet_id == 0x03 {
                ConnectionState::Play
            } else {
                state
            }
        }
        _ => state,
    };
    if next != state
        && let Ok(mut map) = cs.lock()
    {
        map.insert(conn_id, next);
    }
}

/// 处理单个连接：读任务按帧解析并上报，写任务从出站通道取字节。
async fn handle_connection(
    socket: TcpStream,
    conn_id: u32,
    inbound_tx: Sender<RawFrame>,
    mut out_rx: Receiver<OutboundMessage>,
    outbound: OutboundMap,
    conn_states: ConnStateMap,
) {
    let (mut rd, mut wr) = socket.into_split();
    // 入站压缩标志：由写任务收到 `EnableCompression` 时置位，读任务据此解帧。
    // 采用 AtomicBool 供读写两个独立任务安全共享。
    let compression = Arc::new(AtomicBool::new(false));
    let write_compression = compression.clone();
    let write_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            match msg {
                OutboundMessage::Frame(bytes) => {
                    if wr.write_all(&bytes).await.is_err() {
                        break;
                    }
                    if wr.flush().await.is_err() {
                        break;
                    }
                }
                OutboundMessage::EnableCompression => {
                    write_compression.store(true, Ordering::Relaxed);
                }
                OutboundMessage::Close => {
                    // 服务端主动断开：发送 FIN（保证位于已写数据之后），随后结束写任务。
                    let _ = wr.shutdown().await;
                    break;
                }
            }
        }
    });

    let mut read_buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        // 处理缓冲区内所有完整帧
        let mut progress = true;
        while progress {
            progress = false;
            if let Some((payload_len, header_len)) = try_parse_frame_length(&read_buf) {
                if payload_len > MAX_FRAME {
                    // 帧过大：丢弃整段缓冲避免死循环
                    read_buf.clear();
                    break;
                }
                let total = header_len + payload_len;
                if read_buf.len() >= total {
                    let frame = read_buf[header_len..total].to_vec();
                    // 压缩已启用时按压缩帧格式解帧（`VarInt 数据长度 + 数据`）。
                    let decoded = if compression.load(Ordering::Relaxed) {
                        let mut pos = 0;
                        decode_frame_compressed(&frame, &mut pos).ok()
                    } else {
                        Some(frame)
                    };
                    match decoded {
                        Some(payload) => process_frame(
                            conn_id,
                            &payload,
                            &inbound_tx,
                            &conn_states,
                            &mut read_buf,
                            total,
                        ),
                        None => {
                            // 畸形帧（解压失败）：丢弃该帧，避免死循环。
                            read_buf.drain(..total);
                        }
                    }
                    progress = true;
                }
            }
        }
        match rd.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => read_buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }

    let _ = inbound_tx.send(RawFrame::Closed(conn_id)).await;
    if let Ok(mut map) = outbound.lock() {
        map.remove(&conn_id);
    }
    if let Ok(mut map) = conn_states.lock() {
        map.remove(&conn_id);
    }
    write_task.abort();
}

/// 解析单帧（packet_id + 包体），更新状态并上报。
fn process_frame(
    conn_id: u32,
    frame: &[u8],
    inbound_tx: &Sender<RawFrame>,
    conn_states: &ConnStateMap,
    read_buf: &mut Vec<u8>,
    total: usize,
) {
    let state = {
        let guard = conn_states.lock();
        match guard {
            Ok(map) => map
                .get(&conn_id)
                .copied()
                .unwrap_or(ConnectionState::Handshake),
            Err(_) => ConnectionState::Handshake,
        }
    };
    let mut fb = ByteBuffer::new(frame.to_vec());
    let packet_id = match fb.get_varint() {
        Ok(v) => v,
        Err(_) => {
            read_buf.drain(..total);
            return;
        }
    };
    let body = fb.as_slice()[fb.position()..].to_vec();
    update_state(conn_states, conn_id, state, packet_id, &body);
    let _ = inbound_tx.try_send(RawFrame::Packet {
        conn_id,
        state,
        packet_id,
        payload: body,
    });
    read_buf.drain(..total);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write as _;
    use std::net::{SocketAddr, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;
    use crate::protocol::packet::Packet;
    use crate::protocol::packets::Intention;

    // ---- 测试辅助：VarInt / 帧编码 ----

    /// 写入 VarInt（与生产实现逻辑一致，测试独立实现避免依赖内部细节）。
    fn write_varint(buf: &mut Vec<u8>, value: i32) {
        let mut value = value as u32;
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
                buf.push(byte);
            } else {
                buf.push(byte);
                break;
            }
        }
    }

    /// 编码 Handshake 包体：VarInt(protocol_version) + String + u16 port + VarInt(next_state)。
    fn encode_handshake_body(server_address: &str, port: u16, next_state: i32) -> Vec<u8> {
        let mut body = Vec::new();
        write_varint(&mut body, 774); // 协议版本 1.21.11
        write_varint(&mut body, server_address.len() as i32); // String 长度（字节数）
        body.extend_from_slice(server_address.as_bytes());
        body.extend_from_slice(&port.to_be_bytes()); // u16 大端
        write_varint(&mut body, next_state);
        body
    }

    /// 编码完整 Handshake 帧：VarInt 长度 + VarInt packet_id + 包体。
    fn encode_handshake_frame(server_address: &str, port: u16, next_state: i32) -> Vec<u8> {
        let mut packet = Vec::new();
        write_varint(&mut packet, 0x00); // Handshake packet_id
        packet.extend_from_slice(&encode_handshake_body(server_address, port, next_state));
        let mut frame = Vec::new();
        write_varint(&mut frame, packet.len() as i32); // 帧长度
        frame.extend_from_slice(&packet);
        frame
    }

    /// 探测一个可用端口：绑定 `127.0.0.1:0` 拿实际地址后立即释放。
    /// 存在极小竞态（端口可能被他人占用），测试场景可接受。
    fn free_addr() -> SocketAddr {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);
        addr
    }

    /// 空出站通道表。
    fn empty_outbound() -> OutboundMap {
        Arc::new(Mutex::new(HashMap::new()))
    }

    // ---- 测试用例 ----

    /// 客户端发送一帧 Handshake → 监听任务经 `inbound_tx` 上报对应 `RawFrame::Packet`。
    #[test]
    fn listener_receives_handshake_frame() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
            let addr = free_addr();
            let handle = ConnectionListener::start(addr, inbound_tx, empty_outbound())
                .await
                .unwrap();

            // 同步连接并发送一帧 Handshake（本机回环握手由内核完成，不依赖 accept 先行）
            let mut stream = TcpStream::connect(addr).unwrap();
            stream
                .write_all(&encode_handshake_frame("localhost", 25565, 2))
                .unwrap();
            stream.flush().unwrap();

            let received = tokio::time::timeout(Duration::from_secs(5), inbound_rx.recv())
                .await
                .expect("5 秒内未收到入站帧")
                .expect("入站通道提前关闭");
            match received {
                RawFrame::Packet {
                    conn_id,
                    state,
                    packet_id,
                    payload,
                } => {
                    assert_eq!(conn_id, 1, "首个连接的 conn_id 应为 1");
                    assert_eq!(
                        state,
                        ConnectionState::Handshake,
                        "收包前状态应为 Handshake"
                    );
                    assert_eq!(packet_id, 0x00, "packet_id 应为 0x00");

                    // 字节级比对：payload 应等于 Handshake 包体
                    let expected = encode_handshake_body("localhost", 25565, 2);
                    assert_eq!(payload, expected, "payload 应与 Handshake 包体逐字节一致");

                    // 解码比对：确认字段可被生产解码器还原
                    let mut bb = ByteBuffer::new(payload);
                    let intention = Intention::decode(&mut bb).unwrap();
                    assert_eq!(intention.protocol_version, 774);
                    assert_eq!(intention.server_address, "localhost");
                    assert_eq!(intention.port, 25565);
                    assert_eq!(intention.next_state, 2);
                }
                other => panic!("期望 RawFrame::Packet，实际收到: {other:?}"),
            }

            handle.abort();
        });
    }

    /// 对端断开 → 监听任务上报 `RawFrame::Closed` 且不 panic。
    #[test]
    fn listener_reports_closed_on_disconnect() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let (inbound_tx, mut inbound_rx) = mpsc::channel(16);
            let addr = free_addr();
            let handle = ConnectionListener::start(addr, inbound_tx, empty_outbound())
                .await
                .unwrap();

            // 连接后立即断开
            let stream = TcpStream::connect(addr).unwrap();
            drop(stream);

            let received = tokio::time::timeout(Duration::from_secs(5), inbound_rx.recv())
                .await
                .expect("5 秒内未收到关闭帧")
                .expect("入站通道提前关闭");
            match received {
                RawFrame::Closed(conn_id) => {
                    assert_eq!(conn_id, 1, "首个连接的 conn_id 应为 1");
                }
                other => panic!("期望 RawFrame::Closed，实际收到: {other:?}"),
            }

            handle.abort();
        });
    }
}
