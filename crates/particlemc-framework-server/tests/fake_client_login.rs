// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 伪客户端集成测试：真实 TCP 连入，按 1.21.11 离线流程完成登录并进入游玩。
//!
//! 覆盖协议 774 全流程：
//! Handshake → Hello → LoginSuccess → LoginAcknowledged → ClientInformation
//! → [RegistryData×N + UpdateTags + FeatureFlags] → FinishConfiguration(S2C)
//! → FinishConfiguration(C2S) → [Login(play) + Position + UpdateHealth + PlayerInfo]
//! → [ChunkBatchStart → (map_chunk)×N → ChunkBatchFinished] → 连接保持。
//! 另含双玩家测试，验证互见（SpawnEntity + PlayerInfoUpdate）。
//!
//! T7 压缩启用：测试客户端支持压缩帧格式。`start_server` 通过环境变量
//! `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD` 控制服务端阈值：
//! - 256：断言收到 LoginCompression(0x03) 并启用压缩，后续帧（含 zlib 解压）正常；
//! - 0：断言不发送 LoginCompression、帧格式原样。
//!
//! 环境变量为进程级共享状态，测试经 `env_guard` 串行化。

// 集成测试允许使用 unwrap/expect（仅用于断言式测试代码）。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use particlemc_framework_server::run;

/// 本文件测试共享进程环境变量，串行化避免并行污染。
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---- VarInt / 压缩辅助 ----

fn write_varint(buf: &mut Vec<u8>, mut value: i32) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// 从字节切片 `pos` 处解析 VarInt，成功返回值并推进 `pos`。
fn read_varint_from(data: &[u8], pos: &mut usize) -> Option<i32> {
    let mut result = 0i32;
    let mut shift = 0;
    while *pos < data.len() {
        let b = data[*pos];
        *pos += 1;
        result |= i32::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
    None
}

/// zlib 压缩（测试客户端出站，仅当包体达到阈值时使用）。
fn compress_zlib(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).expect("zlib 压缩失败");
    encoder.finish().expect("zlib 压缩结束失败")
}

// ---- 测试客户端（支持压缩） ----

/// 测试客户端：真实 TCP 流 + 压缩状态。
struct FakeClient {
    stream: TcpStream,
    /// 收到 `LoginCompression`(0x03) 且阈值 > 0 后置位，后续收发走压缩帧格式。
    compression: bool,
    /// 服务端通告的压缩阈值。
    threshold: i32,
    /// 实际解压过的帧数（data_len > 0），供断言「后续帧可解压」。
    decompressed_frames: usize,
}

impl FakeClient {
    fn connect(port: u16) -> Self {
        let addr = format!("127.0.0.1:{port}")
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap();
        let mut stream = None;
        for _ in 0..100 {
            if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
                stream = Some(s);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let stream = stream.expect("服务器未在超时内就绪");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Self {
            stream,
            compression: false,
            threshold: 0,
            decompressed_frames: 0,
        }
    }

    fn set_read_timeout(&mut self, dur: Duration) {
        self.stream.set_read_timeout(Some(dur)).unwrap();
    }

    fn read_varint(&mut self) -> Option<i32> {
        let mut result = 0i32;
        let mut shift = 0;
        loop {
            let mut byte = [0u8; 1];
            if self.stream.read(&mut byte).ok()? != 1 {
                return None;
            }
            let b = byte[0];
            result |= i32::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift >= 35 {
                return None;
            }
        }
        Some(result)
    }

    /// 读取一帧，返回 (packet_id, body)。超时 / 断开返回 None。
    ///
    /// 压缩启用后按压缩帧格式解帧：`VarInt 数据长度 + 数据`，`0` 未压缩原样，
    /// `>0` 为 zlib 压缩数据（解压后返回）。
    fn read_frame(&mut self) -> Option<(i32, Vec<u8>)> {
        let len = self.read_varint()?;
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let mut read = 0;
        while read < buf.len() {
            match self.stream.read(&mut buf[read..]) {
                Ok(0) => return None,
                Ok(n) => read += n,
                Err(_) => return None,
            }
        }
        if self.compression {
            let mut pos = 0;
            let data_len = read_varint_from(&buf, &mut pos).unwrap_or(0);
            if data_len == 0 {
                // 未压缩：数据原样
                buf = buf[pos..].to_vec();
            } else {
                // zlib 解压
                let mut decoder = flate2::read::ZlibDecoder::new(&buf[pos..]);
                let mut out = Vec::new();
                decoder.read_to_end(&mut out).expect("zlib 解压失败");
                self.decompressed_frames += 1;
                buf = out;
            }
        }
        // 解析 packet_id
        let mut pos = 0;
        let mut pid = 0i32;
        let mut shift = 0;
        loop {
            if pos >= buf.len() {
                return None;
            }
            let b = buf[pos];
            pos += 1;
            pid |= i32::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let body = buf[pos..].to_vec();
        Some((pid, body))
    }

    /// 发送一帧（packet_id + body）。压缩启用后走压缩帧格式。
    fn send_frame(&mut self, packet_id: i32, body: &[u8]) {
        let mut payload = Vec::new();
        write_varint(&mut payload, packet_id);
        payload.extend_from_slice(body);

        let mut frame = Vec::new();
        if self.compression {
            let mut inner = Vec::new();
            let threshold = usize::try_from(self.threshold).unwrap_or(0);
            let compressed = if payload.len() >= threshold {
                Some(compress_zlib(&payload))
            } else {
                None
            };
            if compressed.as_ref().is_some_and(|c| c.len() < payload.len()) {
                let c = compressed.unwrap();
                write_varint(&mut inner, i32::try_from(payload.len()).unwrap_or(0));
                inner.extend_from_slice(&c);
            } else {
                // 未压缩：数据长度字段 = 0
                write_varint(&mut inner, 0);
                inner.extend_from_slice(&payload);
            }
            write_varint(&mut frame, i32::try_from(inner.len()).unwrap_or(0));
            frame.extend_from_slice(&inner);
        } else {
            write_varint(&mut frame, i32::try_from(payload.len()).unwrap_or(0));
            frame.extend_from_slice(&payload);
        }
        self.stream.write_all(&frame).expect("写入帧失败");
        self.stream.flush().expect("flush 失败");
    }

    /// 持续读取帧，直到遇到 `stop_at` 包 id（该帧计入并返回）或读超时。
    /// `stop_at = None` 时仅按超时收集全部已到达帧。
    fn collect_until(&mut self, stop_at: Option<i32>, timeout_ms: u64) -> Vec<(i32, Vec<u8>)> {
        self.set_read_timeout(Duration::from_millis(timeout_ms));
        let mut frames = Vec::new();
        while let Some((pid, body)) = self.read_frame() {
            frames.push((pid, body));
            if Some(pid) == stop_at {
                break;
            }
        }
        frames
    }

    /// 握手（protocol=774, next_state=2）。`forwarding = None` 表示离线直连。
    fn send_handshake(&mut self, port: u16, forwarding: Option<&[u8]>) {
        let mut hs = Vec::new();
        write_varint(&mut hs, 774);
        write_varint(&mut hs, "localhost".len() as i32);
        hs.extend_from_slice(b"localhost");
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 2);
        if let Some(blob) = forwarding {
            write_varint(&mut hs, 1);
            write_varint(&mut hs, blob.len() as i32);
            hs.extend_from_slice(blob);
        }
        self.send_frame(0x00, &hs);
    }

    fn send_hello(&mut self, name: &str) {
        let mut hello = Vec::new();
        write_varint(&mut hello, name.len() as i32);
        hello.extend_from_slice(name.as_bytes());
        hello.push(1u8);
        hello.extend_from_slice(&[0u8; 16]);
        self.send_frame(0x00, &hello);
    }
}

// ---- 服务器启动 ----

/// 以显式压缩阈值启动服务器（设置 `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD` 环境变量，
/// `run` 读取该环境变量决定是否下发 LoginCompression）。调用方须持有 `env_guard`。
fn start_server(port: u16, threshold: i32) {
    unsafe {
        std::env::set_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", threshold.to_string());
    }
    let addr = format!("127.0.0.1:{port}")
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    std::thread::spawn(move || {
        let _ = run(addr);
    });
}

/// 等待监听就绪并返回测试客户端。
fn connect_ready(port: u16) -> FakeClient {
    FakeClient::connect(port)
}

// ---- 登录流程辅助 ----

/// 收集到的 (packet_id, body) 帧列表。
type FrameList = Vec<(i32, Vec<u8>)>;

/// 完成单玩家登录，返回 (配置阶段帧, 游玩阶段帧, 是否启用压缩)。
///
/// 配置阶段收集到 FinishConfiguration(S2C, 0x03) 为止（含该帧）；
/// 游玩阶段按超时收集全部已到达帧。LoginSuccess 之后探测 LoginCompression(0x03)，
/// 收到且阈值 > 0 则启用压缩（服务端阈值 0 时该读取短超时返回 None）。
fn login_single_player(client: &mut FakeClient, port: u16) -> (FrameList, FrameList, bool) {
    client.send_handshake(port, None);
    client.send_hello("Tester");

    // 读取 LoginSuccess (0x02)
    let mut got_login_success = false;
    for _ in 0..50 {
        match client.read_frame() {
            Some((0x02, _)) => {
                got_login_success = true;
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert!(got_login_success, "未收到 LoginSuccess(0x02)");

    // LoginSuccess 之后探测 LoginCompression（0x03，T7）：收到即启用压缩。
    // 未启用压缩时服务端在收到 ClientInformation 前不再发包，短超时后返回 None。
    let mut compression_enabled = false;
    client.set_read_timeout(Duration::from_millis(300));
    if let Some((0x03, body)) = client.read_frame() {
        let mut pos = 0;
        let threshold =
            read_varint_from(&body, &mut pos).expect("LoginCompression 包体应为 VarInt 阈值");
        assert!(threshold > 0, "LoginCompression 阈值应 > 0");
        client.compression = true;
        client.threshold = threshold;
        compression_enabled = true;
    }
    client.set_read_timeout(Duration::from_secs(5));

    // LoginAcknowledged (0x03, serverbound) → Configuration
    client.send_frame(0x03, &[]);
    // ClientInformation (0x00, serverbound) → 触发服务端下发注册表同步 + FinishConfiguration(S2C)
    client.send_frame(0x00, &[]);

    // 收集配置阶段帧：直到 FinishConfiguration(S2C, 0x03)
    let config_frames = client.collect_until(Some(0x03), 5000);
    // FinishConfiguration (C2S, 0x03, serverbound) → Play
    client.send_frame(0x03, &[]);

    // 收集游玩阶段帧：按超时收集（含 Login/Position/UpdateHealth/PlayerInfo/ChunkBatch×）
    let play_frames = client.collect_until(None, 5000);
    (config_frames, play_frames, compression_enabled)
}

fn first_index_of(frames: &[(i32, Vec<u8>)], id: i32) -> Option<usize> {
    frames.iter().position(|(pid, _)| *pid == id)
}

fn last_index_of(frames: &[(i32, Vec<u8>)], id: i32) -> Option<usize> {
    frames.iter().rposition(|(pid, _)| *pid == id)
}

// ---- 测试用例 ----

/// L1-Runtime：单玩家按 774 全流程登录进入游玩（压缩启用 threshold=256），
/// 且配置/游玩阶段包序列与顺序正确。
#[test]
fn fake_client_completes_login_and_play_774() {
    let _guard = env_guard();
    const PORT: u16 = 25569;
    start_server(PORT, 256);
    let mut client = connect_ready(PORT);

    let (config_frames, play_frames, _compression) = login_single_player(&mut client, PORT);

    // === 配置阶段顺序：RegistryData(0x07)×N → UpdateTags(0x0d) → FeatureFlags(0x0c) → FinishConfiguration(0x03) ===
    let n_registry = config_frames.iter().filter(|(id, _)| *id == 0x07).count();
    assert!(
        n_registry >= 15,
        "配置阶段应同步 ≥15 个 RegistryData(0x07)，实际 {n_registry}"
    );
    let idx_registry = first_index_of(&config_frames, 0x07).expect("应含 RegistryData");
    let idx_tags = first_index_of(&config_frames, 0x0d).expect("应含 UpdateTags(0x0d)");
    let idx_features = first_index_of(&config_frames, 0x0c).expect("应含 FeatureFlags(0x0c)");
    let idx_finish = last_index_of(&config_frames, 0x03).expect("应含 FinishConfiguration(0x03)");
    assert!(
        idx_registry < idx_tags && idx_tags < idx_features && idx_features < idx_finish,
        "配置阶段顺序应为 RegistryData → UpdateTags → FeatureFlags → FinishConfiguration"
    );

    // === 游玩阶段：Login(0x30) + Position(0x46) + UpdateHealth(0x66) + PlayerInfo(0x44) 均存在且有序 ===
    let idx_login = first_index_of(&play_frames, 0x30).expect("应含 Login(0x30)");
    let idx_position = first_index_of(&play_frames, 0x46).expect("应含 Position(0x46)");
    let idx_health = first_index_of(&play_frames, 0x66).expect("应含 UpdateHealth(0x66)");
    let idx_info = first_index_of(&play_frames, 0x44).expect("应含 PlayerInfo(0x44)");
    assert!(
        idx_login < idx_position && idx_position < idx_health && idx_health < idx_info,
        "游玩阶段顺序应为 Login → Position → UpdateHealth → PlayerInfo"
    );

    // === 区块批次：ChunkBatchStart(0x0c) → (map_chunk 0x2c)×N → ChunkBatchFinished(0x0b) ===
    let idx_batch_start = first_index_of(&play_frames, 0x0c).expect("应含 ChunkBatchStart(0x0c)");
    let idx_batch_finished =
        first_index_of(&play_frames, 0x0b).expect("应含 ChunkBatchFinished(0x0b)");
    assert!(
        idx_batch_start < idx_batch_finished,
        "ChunkBatchStart(0x0c) 应早于 ChunkBatchFinished(0x0b)"
    );
    // 若存在 map_chunk(0x2c)，必须位于 Start 与 Finished 之间（集成测试无实例时为 0 个，由 unit 测试覆盖）。
    let map_chunks: Vec<usize> = play_frames
        .iter()
        .enumerate()
        .filter(|(_, (id, _))| *id == 0x2c)
        .map(|(i, _)| i)
        .collect();
    for i in &map_chunks {
        assert!(
            *i > idx_batch_start && *i < idx_batch_finished,
            "map_chunk(0x2c) 应位于 ChunkBatchStart 与 ChunkBatchFinished 之间"
        );
    }

    // === 回 ChunkBatchReceived(0x0a) 后连接保持 ===
    client.send_frame(0x0a, &[]);
    client.set_read_timeout(Duration::from_millis(1500));
    let first = client.read_frame();
    if let Some((id, _)) = first {
        assert_ne!(id, 0x00, "ChunkBatchReceived 后收到异常包");
    } else {
        // 读超时（存活）：用写入探测区分超时与断开
        client
            .stream
            .set_write_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let write_ok = client.stream.write_all(&[0x00]).is_ok();
        assert!(write_ok, "ChunkBatchReceived 后连接被关闭（应仍存活）");
    }
}

/// L1-Runtime：双玩家互见。玩家 B 登录后，玩家 A 收到 B 的 SpawnEntity(0x01) + PlayerInfo(0x44)。
#[test]
fn two_players_see_each_other() {
    let _guard = env_guard();
    const PORT: u16 = 25570;
    start_server(PORT, 256);

    // 玩家 A 登录完成
    let mut a = connect_ready(PORT);
    let (_cfg_a, _play_a, _comp_a) = login_single_player(&mut a, PORT);

    // 玩家 B 登录完成
    let mut b = connect_ready(PORT);
    let (_cfg_b, _play_b, _comp_b) = login_single_player(&mut b, PORT);

    // B 登录后，A 应收到 B 的 SpawnEntity + PlayerInfoUpdate（互见广播）
    let a_frames = a.collect_until(None, 5000);
    assert!(
        a_frames.iter().any(|(id, _)| *id == 0x01),
        "玩家 A 应收到 B 的 SpawnEntity(0x01)"
    );
    assert!(
        a_frames.iter().any(|(id, _)| *id == 0x44),
        "玩家 A 应收到 B 的 PlayerInfoUpdate(0x44)"
    );
}

/// T7：threshold=256 时断言收到 LoginCompression(0x03) 并启用压缩，后续帧可解压
/// （存在真实 zlib 解压的帧），配置/游玩阶段包序列完整。
#[test]
fn fake_client_login_enables_compression_at_256() {
    let _guard = env_guard();
    const PORT: u16 = 25571;
    start_server(PORT, 256);
    let mut client = connect_ready(PORT);

    let (config_frames, play_frames, compression_enabled) = login_single_player(&mut client, PORT);

    // 收到 LoginCompression 并启用压缩
    assert!(
        compression_enabled,
        "threshold=256 时应收到 LoginCompression 并启用压缩"
    );
    // 后续帧可解压：存在 data_len > 0 的 zlib 压缩帧被成功解压
    assert!(
        client.decompressed_frames > 0,
        "threshold=256 时应有帧被实际 zlib 解压"
    );
    // 解压后包序列正确
    assert!(
        config_frames.iter().any(|(id, _)| *id == 0x07),
        "配置阶段应含 RegistryData(0x07)"
    );
    assert!(
        config_frames.iter().any(|(id, _)| *id == 0x03),
        "配置阶段应含 FinishConfiguration(0x03)"
    );
    assert!(
        play_frames.iter().any(|(id, _)| *id == 0x30),
        "游玩阶段应含 Login(0x30)"
    );
}

/// T7：threshold=0 时不发送 LoginCompression、无压缩帧、帧格式原样。
#[test]
fn fake_client_login_no_compression_when_threshold_zero() {
    let _guard = env_guard();
    const PORT: u16 = 25572;
    start_server(PORT, 0);
    let mut client = connect_ready(PORT);

    let (config_frames, play_frames, compression_enabled) = login_single_player(&mut client, PORT);

    // 未发送 LoginCompression
    assert!(
        !compression_enabled,
        "threshold=0 时不应收到 LoginCompression"
    );
    // 无任何压缩帧
    assert_eq!(
        client.decompressed_frames, 0,
        "threshold=0 时不应有 zlib 解压帧"
    );
    // 帧格式原样：配置/游玩阶段包序列正常
    assert!(
        config_frames.iter().any(|(id, _)| *id == 0x07),
        "配置阶段应含 RegistryData(0x07)"
    );
    assert!(
        play_frames.iter().any(|(id, _)| *id == 0x30),
        "游玩阶段应含 Login(0x30)"
    );
}
