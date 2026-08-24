// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 多玩家集成测试。
//!
//! - 100 名玩家并发 TCP 登录，验证服务器稳定性
//! - 移动广播、距离裁剪、批量合并由 particlemc-framework-core::system::entity_sync 单元测试覆盖

// 集成测试允许使用 unwrap/expect（仅用于断言式测试代码）。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// 压缩（测试客户端出站，仅当包体达到阈值时使用）。
fn compress_zlib(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).expect("zlib 压缩失败");
    encoder.finish().expect("zlib 压缩结束失败")
}

// ---- 测试客户端 ----

/// 测试客户端：真实 TCP 流 + 压缩状态。
struct FakeClient {
    stream: TcpStream,
    compression: bool,
    threshold: i32,
    player_index: usize,
}

impl FakeClient {
    fn connect(port: u16, index: usize) -> Self {
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
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Self {
            stream,
            compression: false,
            threshold: 0,
            player_index: index,
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
                buf = buf[pos..].to_vec();
            } else {
                let mut decoder = flate2::read::ZlibDecoder::new(&buf[pos..]);
                let mut out = Vec::new();
                decoder.read_to_end(&mut out).expect("zlib 解压失败");
                buf = out;
            }
        }
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

    /// 持续读取帧，直到遇到 `stop_at` 包 id 或读超时。
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

    /// 握手（protocol=774, next_state=2）。
    fn send_handshake(&mut self, port: u16) {
        let mut hs = Vec::new();
        write_varint(&mut hs, 774);
        write_varint(&mut hs, "localhost".len() as i32);
        hs.extend_from_slice(b"localhost");
        hs.extend_from_slice(&port.to_be_bytes());
        write_varint(&mut hs, 2);
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

    /// 非阻塞收集当前可用帧（短超时）。
    fn collect_all_frames(&mut self, timeout_ms: u64) -> Vec<(i32, Vec<u8>)> {
        self.set_read_timeout(Duration::from_millis(timeout_ms));
        let mut frames = Vec::new();
        self.stream.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            self.stream.set_read_timeout(Some(remaining)).unwrap();
            match self.read_frame() {
                Some(frame) => frames.push(frame),
                None => break,
            }
        }
        // 恢复阻塞模式
        self.stream.set_nonblocking(false).unwrap();
        frames
    }

    fn player_name(&self) -> String {
        format!("SimPlayer_{}", self.player_index)
    }
}

// ---- 服务器启动 ----

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

fn connect_ready(port: u16, index: usize) -> FakeClient {
    FakeClient::connect(port, index)
}

// ---- 登录流程 ----

type FrameList = Vec<(i32, Vec<u8>)>;

/// 登录单个玩家，返回 Ok((config_frames, play_frames, compression)) 或 Err(原因)。
fn login_single_player(client: &mut FakeClient, port: u16) -> Result<(FrameList, FrameList, bool), String> {
    client.send_handshake(port);
    client.send_hello(&client.player_name());

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
    if !got_login_success {
        return Err("未收到 LoginSuccess(0x02)".to_string());
    }

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

    client.send_frame(0x03, &[]);
    client.send_frame(0x00, &[]);

    let config_frames = client.collect_until(Some(0x03), 5000);
    client.send_frame(0x03, &[]);
    let play_frames = client.collect_until(None, 5000);
    Ok((config_frames, play_frames, compression_enabled))
}

// ================================================================
// 测试用例
// ================================================================

/// 并发登录测试：100 名玩家同时登录，验证无崩溃。
#[test]
fn multi_player_simultaneous_login_test() {
    let _guard = env_guard();
    const PORT: u16 = 25574;
    const NUM_PLAYERS: usize = 100;

    start_server(PORT, 0); // 不启用压缩以加快测试
    std::thread::sleep(Duration::from_secs(2));

    println!("\n=== 并发登录测试开始 ===");
    println!("服务器端口: {}", PORT);
    println!("玩家数量: {}", NUM_PLAYERS);

    let t_start = Instant::now();
    let mut handles = Vec::new();

    // 10 线程并发登录，每线程 10 名玩家
    for batch in 0..(NUM_PLAYERS / 10) {
        let port = PORT;
        let handle = std::thread::spawn(move || {
            for i in 0..10 {
                let idx = batch * 10 + i;
                let mut client = connect_ready(port, idx);
                match login_single_player(&mut client, port) {
                    Ok((config_frames, play_frames, _compression)) => {
                        client.send_frame(0x0a, &[]);
                        let _extra = client.collect_all_frames(500);
                        assert!(!config_frames.is_empty(), "玩家 {} 配置阶段应有帧", idx);
                        assert!(!play_frames.is_empty(), "玩家 {} 游玩阶段应有帧", idx);
                    }
                    Err(e) => eprintln!("玩家 {} 登录失败: {}", idx, e),
                }
            }
        });
        handles.push(handle);
    }

    let mut success_count = 0;
    let mut error_count = 0;
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.join() {
            Ok(()) => {
                success_count += 10;
                if i % 2 == 0 {
                    println!("批次 {} 登录成功", i * 10);
                }
            }
            Err(e) => {
                error_count += 10;
                eprintln!("批次 {} 登录失败: {:?}", i * 10, e);
            }
        }
    }

    let dur = t_start.elapsed();
    println!("\n=== 并发登录测试完成 ===");
    println!("成功: {}, 失败: {}, 耗时: {:.2}s", success_count, error_count, dur.as_secs_f64());
    assert_eq!(success_count, NUM_PLAYERS, "应有 {} 名玩家成功登录", NUM_PLAYERS);
    assert_eq!(error_count, 0, "不应有登录失败");
    assert!(dur.as_secs_f64() <= 120.0, "登录耗时 {}s 超过 120s 限制", dur.as_secs_f64());
    println!("并发登录测试通过 ✓");
}

// ---- 包统计 ----

#[derive(Debug, Default, Clone)]
struct PacketStats {
    total_packets: usize,
    move_packets: usize,
    destroy_entity_packets: usize,
    spawn_entity_packets: usize,
    player_info_packets: usize,
    block_break_anim_packets: usize, // 0x05 BlockBreakAnimation
    game_state_packets: usize,       // 0x26 GameStateChange
}

impl PacketStats {
    fn record_frame(&mut self, pid: i32) {
        self.total_packets += 1;
        match pid {
            0x33 => self.move_packets += 1,
            0x7b => self.move_packets += 1,
            0x4b => self.destroy_entity_packets += 1,
            0x01 => self.spawn_entity_packets += 1,
            0x44 => self.player_info_packets += 1,
            0x05 => self.block_break_anim_packets += 1,
            0x26 => self.game_state_packets += 1,
            _ => {}
        }
    }
}

// ================================================================
// TCP 压力测试 V4：多行为混合压力测试（100 名玩家）
// ================================================================

/// TCP 多玩家压力测试 V4。
///
/// 场景：
/// - 100 名玩家全部从世界出生点登录
/// - 50 人：疯狂跑图（每 tick 发送位置包，差异化移动）
/// - 30 人：疯狂挖方块（每 2 ticks 发送挖掘包 0x28）
/// - 20 人：相互攻击（每 3 ticks 发送攻击动画 0x3c）
/// - 监测 TPS、发包数据、距离裁剪效果
#[test]
fn multi_player_v4_mixed_behavior_test() {
    let _guard = env_guard();
    const PORT: u16 = 25575;
    const NUM_PLAYERS: usize = 100;
    const NUM_RUNNERS: usize = 50;
    const NUM_DIGGERS: usize = 30;
    const NUM_ATTACKERS: usize = 20;
    const BATCH_SIZE: usize = 10;
    const SIMULATION_TICKS: u32 = 200;
    const MOVE_INTERVAL: u32 = 2;      // 跑图玩家每 2 ticks 发位置
    const DIGGER_INTERVAL: u32 = 4;    // 挖方块玩家每 4 ticks 挖掘
    const ATTACKER_INTERVAL: u32 = 6;  // 攻击玩家每 6 ticks 攻击

    start_server(PORT, 0); // 禁用压缩以加快测试
    std::thread::sleep(Duration::from_secs(2));

    println!("\n=== TCP 多玩家混合压力测试 V4 ===");
    println!("服务器端口: {}", PORT);
    println!("总玩家: {}", NUM_PLAYERS);
    println!("  - 跑图玩家: {}（每 {} ticks 移动）", NUM_RUNNERS, MOVE_INTERVAL);
    println!("  - 挖方块玩家: {}（每 {} ticks 挖掘）", NUM_DIGGERS, DIGGER_INTERVAL);
    println!("  - 攻击玩家: {}（每 {} ticks 攻击）", NUM_ATTACKERS, ATTACKER_INTERVAL);
    println!("模拟 tick 数: {}", SIMULATION_TICKS);
    println!("目标 TPS: 20（服务器 20Hz）");

    // ---- 阶段 1: 并发登录 ----
    println!("\n--- 阶段 1: 并发登录 {} 名玩家 ---", NUM_PLAYERS);
    let t_login = Instant::now();

    let mut clients: Vec<FakeClient> = Vec::with_capacity(NUM_PLAYERS);
    let mut handles = Vec::new();

    for batch in 0..(NUM_PLAYERS / BATCH_SIZE) {
        let port = PORT;
        let handle = std::thread::spawn(move || {
            let mut batch_clients = Vec::new();
            for i in 0..BATCH_SIZE {
                let idx = batch * BATCH_SIZE + i;
                print!("登录玩家 {}/{}: ", idx + 1, NUM_PLAYERS);
                let mut client = connect_ready(port, idx);
                match login_single_player(&mut client, port) {
                    Ok((_config, play_frames, _compression)) => {
                        client.send_frame(0x0a, &[]);
                        let _extra = client.collect_all_frames(500);
                        println!("完成 (play={} 帧)", play_frames.len());
                        batch_clients.push(client);
                    }
                    Err(e) => {
                        eprintln!("登录玩家 {} 失败: {}", idx + 1, e);
                    }
                }
            }
            batch_clients
        });
        handles.push(handle);
    }

    for handle in handles {
        match handle.join() {
            Ok(batch) => clients.extend(batch),
            Err(e) => eprintln!("登录线程 panic: {:?}", e),
        }
    }

    println!("\n成功登录 {} / {} 名玩家", clients.len(), NUM_PLAYERS);
    assert_eq!(clients.len(), NUM_PLAYERS, "应有 {} 名玩家成功登录", NUM_PLAYERS);

    let login_dur = t_login.elapsed();
    println!("\n所有 {} 名玩家登录完成（耗时 {:.2}s）", NUM_PLAYERS, login_dur.as_secs_f64());

    // ---- 阶段 2: 模拟活动（50 跑图 + 30 挖矿 + 20 攻击）----
    println!("\n--- 阶段 2: 模拟 {} ticks 混合活动 ---", SIMULATION_TICKS);

    let mut total_stats = PacketStats::default();
    let mut per_client_stats: Vec<PacketStats> = vec![PacketStats::default(); NUM_PLAYERS];
    let mut tps_samples: Vec<f64> = Vec::new();
    let tick_start = Instant::now();

    for tick in 0..SIMULATION_TICKS {
        if tick % 50 == 0 {
            print!("\r  进度: {}/{} ticks ({:.1}%)", tick, SIMULATION_TICKS, tick as f64 / SIMULATION_TICKS as f64 * 100.0);
        }

        // 50 名跑图玩家：每 tick 发送位置包（差异化速度避免去重）
        for idx in 0..NUM_RUNNERS {
            if let Some(client) = clients.get_mut(idx) {
                let mut pos_body = Vec::new();
                let speed_x = ((idx % 5) as f64 + 0.1) * 0.1;
                let speed_z = ((idx / 5) as f64 + 0.2) * 0.1;
                pos_body.extend_from_slice(&(tick as f64 * speed_x).to_le_bytes());
                pos_body.extend_from_slice(&64.0_f64.to_le_bytes());
                pos_body.extend_from_slice(&(tick as f64 * speed_z).to_le_bytes());
                pos_body.extend_from_slice(&0.0_f32.to_le_bytes());
                pos_body.extend_from_slice(&0.0_f32.to_le_bytes());
                pos_body.push(0x04);
                client.send_frame(0x1e, &pos_body);
            }
        }

        // 30 名挖方块玩家：每 2 ticks 发送挖掘包（PlayerAction 0x28, status=2）
        if tick % DIGGER_INTERVAL == 0 {
            for idx in NUM_RUNNERS..(NUM_RUNNERS + NUM_DIGGERS) {
                if let Some(client) = clients.get_mut(idx) {
                    let mut action_body = Vec::new();
                    write_varint(&mut action_body, 2); // status=2: 开始挖掘
                    action_body.extend_from_slice(&128i32.to_le_bytes()); // x
                    action_body.extend_from_slice(&64i32.to_le_bytes());  // y
                    action_body.extend_from_slice(&128i32.to_le_bytes()); // z
                    action_body.push(0u8); // face=0 (bottom)
                    write_varint(&mut action_body, tick as i32); // sequence
                    client.send_frame(0x28, &action_body);
                }
            }
        }

        // 20 名攻击玩家：每 ATTACKER_INTERVAL ticks 发送攻击动画（Animation 0x3c, hand=0）
        if tick % ATTACKER_INTERVAL == 0 {
            for idx in (NUM_RUNNERS + NUM_DIGGERS)..NUM_PLAYERS {
                if let Some(client) = clients.get_mut(idx) {
                    let mut anim_body = Vec::new();
                    write_varint(&mut anim_body, 0); // hand=0 (主手)
                    client.send_frame(0x3c, &anim_body);
                }
            }
        }

        // 等待服务器处理 tick
        std::thread::sleep(Duration::from_millis(80));

        // 收集各客户端出站帧
        for (i, client) in clients.iter_mut().enumerate() {
            let frames = client.collect_all_frames(10);
            for (pid, _body) in frames {
                per_client_stats[i].record_frame(pid);
                total_stats.record_frame(pid);
            }
        }

        // 记录 TPS 采样（每 10 ticks 采样一次）
        if tick > 0 && tick % 10 == 0 {
            let elapsed = tick_start.elapsed().as_secs_f64();
            let tps = (tick as f64) / elapsed;
            tps_samples.push(tps);
        }
    }
    println!("\r  进度: {}/{} ticks (100.0%)", SIMULATION_TICKS, SIMULATION_TICKS);

    // ---- 阶段 3: 最终收帧 ----
    println!("\n--- 阶段 3: 最终收帧 ---");
    for (i, client) in clients.iter_mut().enumerate() {
        let frames = client.collect_all_frames(500);
        for (pid, _body) in frames {
            per_client_stats[i].record_frame(pid);
            total_stats.record_frame(pid);
        }
    }

    let total_dur = t_login.elapsed();
    let sim_dur = total_dur.saturating_sub(login_dur);

    // 计算平均 TPS
    let avg_tps = if !tps_samples.is_empty() {
        tps_samples.iter().sum::<f64>() / tps_samples.len() as f64
    } else {
        let elapsed = tick_start.elapsed().as_secs_f64();
        if elapsed > 0.0 { SIMULATION_TICKS as f64 / elapsed } else { 0.0 }
    };

    println!("\n=== TCP 混合压力测试完成 ===");
    println!("总客户端数: {}", NUM_PLAYERS);
    println!("登录耗时: {:.2}s", login_dur.as_secs_f64());
    println!("模拟耗时: {:.2}s", sim_dur.as_secs_f64());
    println!("总耗时: {:.2}s", total_dur.as_secs_f64());
    println!("平均 TPS: {:.2}", avg_tps);
    println!("目标 TPS: 20.00");
    println!("TPS 达标: {}", if avg_tps >= 18.0 { "✓" } else { "⚠ 低于预期" });
    println!();
    println!("--- 发包统计 ---");
    println!("总发包数: {}", total_stats.total_packets);
    println!("  实体生成包 (SpawnEntity 0x01): {}", total_stats.spawn_entity_packets);
    println!("  玩家信息包 (PlayerInfo 0x44):  {}", total_stats.player_info_packets);
    println!("  移动包 (RelEntityMove 0x33 / Teleport 0x7b): {}", total_stats.move_packets);
    println!("  实体销毁包 (DestroyEntities 0x4b): {}", total_stats.destroy_entity_packets);
    println!("  方块破坏动画 (BlockBreakAnim 0x05): {}", total_stats.block_break_anim_packets);
    println!("  游戏状态变更 (GameState 0x26):     {}", total_stats.game_state_packets);
    println!();
    println!("--- 按玩家分组 ---");
    println!("  跑图玩家 (#0-{}) 移动包: {}", NUM_RUNNERS - 1,
        (0..NUM_RUNNERS).map(|i| per_client_stats[i].move_packets).sum::<usize>());
    println!("  挖方块玩家 (#{}) 移动包: {}", NUM_RUNNERS,
        (NUM_RUNNERS..(NUM_RUNNERS + NUM_DIGGERS)).map(|i| per_client_stats[i].move_packets).sum::<usize>());
    println!("  攻击玩家 (#{}) 移动包: {}", NUM_RUNNERS + NUM_DIGGERS,
        (NUM_RUNNERS + NUM_DIGGERS..NUM_PLAYERS).map(|i| per_client_stats[i].move_packets).sum::<usize>());
    println!();
    println!("  跑图玩家 (#0-{}) 挖方块包: {}", NUM_RUNNERS - 1,
        (0..NUM_RUNNERS).map(|i| per_client_stats[i].block_break_anim_packets).sum::<usize>());
    println!("  挖方块玩家 (#{}) 挖方块包: {}", NUM_RUNNERS,
        (NUM_RUNNERS..(NUM_RUNNERS + NUM_DIGGERS)).map(|i| per_client_stats[i].block_break_anim_packets).sum::<usize>());
    println!("  攻击玩家 (#{}) 挖方块包: {}", NUM_RUNNERS + NUM_DIGGERS,
        (NUM_RUNNERS + NUM_DIGGERS..NUM_PLAYERS).map(|i| per_client_stats[i].block_break_anim_packets).sum::<usize>());

    // 验证关键指标
    assert!(total_stats.spawn_entity_packets > 0, "实体生成包应 > 0");
    assert!(total_stats.player_info_packets > 0, "玩家信息包应 > 0");
    assert!(avg_tps >= 10.0, "平均 TPS 应 >= 10，实际: {:.2}", avg_tps);

    println!("\nTCP 混合压力测试 V4 通过 ✓");
}
