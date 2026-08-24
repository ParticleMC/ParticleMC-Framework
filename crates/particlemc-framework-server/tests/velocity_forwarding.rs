//! Velocity modern forwarding 集成测试（真实 TCP，1.21.11 流程）。
//!
//! 场景一（velocity_forwarding_uses_real_uuid）：伪客户端携带正确签名的
//! Velocity modern forwarding blob 连入，登录成功后断言 `LoginSuccess` 携带的
//! UUID 等于转发 blob 中的真实 UUID（而非离线随机 / 客户端自报 UUID）。
//!
//! 场景二（enforce_proxy_rejects_direct）：`PARTICLE_MCFRAMEWORK_VELOCITY_ENFORCE=1` 时，
//! 无转发 blob 的直连握手会被拒绝：先收到 `LoginDisconnect`（登录阶段 clientbound
//! id 0x00，reason 含 "Velocity"），随后连接被服务端主动关闭（读到 EOF）。
//!
//! 两个测试共享进程环境变量，通过 `ENV_LOCK` 串行化并在末尾恢复原值。

// 集成测试允许使用 unwrap/expect（仅用于断言式测试代码）。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use particlemc_framework_core::protocol::byte_buf::ByteBuffer;
use particlemc_framework_core::protocol::packet::Packet;
use particlemc_framework_core::protocol::packets::LoginSuccess;
use particlemc_framework_server::run;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// 转发 blob 中携带的固定真实 UUID（16 字节，等价于 `Uuid::from_u128(0x1111…1111)`）。
const FORWARDED_UUID: [u8; 16] = [0x11; 16];

/// 本文件测试共享进程环境变量，串行化避免并行污染。
/// 若先前持有者在 panic 展开时退出（锁被毒化），仍接管以恢复环境一致性。
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 设置环境变量并记录旧值；`Drop` 时恢复（含 panic 展开路径）。
struct EnvVarRestore {
    name: &'static str,
    old: Option<String>,
}

impl EnvVarRestore {
    fn set(name: &'static str, value: &str) -> Self {
        let old = std::env::var(name).ok();
        unsafe {
            std::env::set_var(name, value);
        }
        Self { name, old }
    }
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => unsafe {
                std::env::set_var(self.name, v);
            },
            None => unsafe {
                std::env::remove_var(self.name);
            },
        }
    }
}

// ---- 帧读写辅助（与 fake_client_login.rs 相同的实现） ----

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

/// 发送一帧：VarInt 长度 + VarInt packet_id + body。
fn send_frame(stream: &mut TcpStream, packet_id: i32, body: &[u8]) {
    let mut payload = Vec::new();
    write_varint(&mut payload, packet_id);
    payload.extend_from_slice(body);

    let mut frame = Vec::new();
    write_varint(&mut frame, payload.len() as i32);
    frame.extend_from_slice(&payload);
    stream.write_all(&frame).expect("写入帧失败");
    stream.flush().expect("flush 失败");
}

/// 读取一帧，返回 (packet_id, body)。超时 / 断开返回 None。
fn read_frame(stream: &mut TcpStream) -> Option<(i32, Vec<u8>)> {
    let len = read_varint(stream)?;
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    let mut read = 0;
    while read < buf.len() {
        match stream.read(&mut buf[read..]) {
            Ok(0) => return None,
            Ok(n) => read += n,
            Err(_) => return None,
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
    Some((pid, buf[pos..].to_vec()))
}

fn read_varint(stream: &mut TcpStream) -> Option<i32> {
    let mut result = 0i32;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        if stream.read(&mut byte).ok()? != 1 {
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

/// 解析包体中的首段 String（VarInt 长度 + UTF-8 字节）。
fn read_string(body: &[u8]) -> String {
    let mut pos = 0;
    let len = read_varint_from(body, &mut pos).unwrap_or(0);
    if len <= 0 || pos + len as usize > body.len() {
        return String::new();
    }
    String::from_utf8(body[pos..pos + len as usize].to_vec()).unwrap_or_default()
}

// ---- Velocity blob 构造 ----

/// 构造 Velocity modern forwarding blob：
/// `version(1) + signature(HMAC-SHA256(secret, version ++ payload)) + payload`，
/// `payload = uuid(16) + name(String) + timestamp(i64) + properties(Vec)`。
fn make_velocity_blob(secret: &[u8], uuid: &[u8; 16], name: &str) -> Vec<u8> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut payload = ByteBuffer::with_capacity(64);
    payload.put_bytes(uuid);
    payload.put_string(name);
    payload.put_i64(now);
    payload.put_varint(0); // 0 properties
    let payload_bytes = payload.into_inner();

    // 签名覆盖 version ++ payload
    let mut data = Vec::with_capacity(payload_bytes.len() + 1);
    data.push(1u8); // version
    data.extend_from_slice(&payload_bytes);
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(&data);
    let sig = mac.finalize().into_bytes().to_vec();

    let mut blob = Vec::with_capacity(1 + sig.len() + payload_bytes.len());
    blob.push(1u8);
    blob.extend_from_slice(&sig);
    blob.extend_from_slice(&payload_bytes);
    blob
}

// ---- 服务器启动辅助 ----

/// 独立线程启动服务器（run 内部自建 tokio 运行时并进入 20Hz 主循环）。
fn start_server(port: u16) {
    let addr = format!("127.0.0.1:{port}")
        .to_socket_addrs()
        .unwrap()
        .next()
        .unwrap();
    std::thread::spawn(move || {
        let _ = run(addr);
    });
}

/// 等待监听就绪并返回连接（读超时 5s）。
fn connect_ready(port: u16) -> TcpStream {
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
    stream
}

// ---- 测试用例 ----

/// L1-Runtime：伪客户端携带正确签名的 Velocity 转发 blob 连入，
/// 登录成功后 `LoginSuccess` 携带的 UUID 必须等于转发 blob 中的真实 UUID。
#[test]
fn velocity_forwarding_uses_real_uuid() {
    let _guard = env_guard();
    let _secret = EnvVarRestore::set("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET", "test-secret-123");
    // T7：该测试只读 LoginSuccess，显式关闭压缩以保持帧格式原样（不涉压缩路径）。
    let _compression = EnvVarRestore::set("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", "0");

    const PORT: u16 = 25567;
    start_server(PORT);
    let mut stream = connect_ready(PORT);

    // 1) Handshake（next_state=2 + 现代转发 blob），携带固定真实 UUID
    let blob = make_velocity_blob(b"test-secret-123", &FORWARDED_UUID, "ProxyPlayer");
    let mut hs = Vec::new();
    write_varint(&mut hs, 774); // 协议版本 1.21.11
    write_varint(&mut hs, "localhost".len() as i32);
    hs.extend_from_slice(b"localhost");
    hs.extend_from_slice(&PORT.to_be_bytes());
    write_varint(&mut hs, 2); // Login
    write_varint(&mut hs, 1); // 有转发
    write_varint(&mut hs, blob.len() as i32);
    hs.extend_from_slice(&blob);
    send_frame(&mut stream, 0x00, &hs);

    // 2) Hello（name 任意；真实身份应来自转发 blob）
    let mut hello = Vec::new();
    let name = "LocalName";
    write_varint(&mut hello, name.len() as i32);
    hello.extend_from_slice(name.as_bytes());
    hello.push(1u8);
    hello.extend_from_slice(&[0u8; 16]);
    send_frame(&mut stream, 0x00, &hello);

    // 3) 读取 LoginSuccess（id 0x02），解码 UUID
    let mut success_body = None;
    for _ in 0..50 {
        match read_frame(&mut stream) {
            Some((0x02, body)) => {
                success_body = Some(body);
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    let body = success_body.expect("未收到 LoginSuccess");
    let mut buf = ByteBuffer::new(body);
    let login = LoginSuccess::decode(&mut buf).expect("LoginSuccess 解码失败");

    // [L1-Runtime] 玩家 UUID 等于转发 uuid（而非 Hello 自报的零 uuid）
    assert_eq!(
        login.uuid.as_bytes(),
        &FORWARDED_UUID,
        "LoginSuccess UUID 应与转发 blob 中的真实 UUID 一致"
    );
    // 用户名也来自转发身份
    assert_eq!(login.username, "ProxyPlayer");
}

/// L1-Runtime：enforce_proxy=true 时，无转发 blob 的直连被 `LoginDisconnect` 关闭。
#[test]
fn enforce_proxy_rejects_direct() {
    let _guard = env_guard();
    let _secret = EnvVarRestore::set("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET", "test-secret-123");
    let _enforce = EnvVarRestore::set("PARTICLE_MCFRAMEWORK_VELOCITY_ENFORCE", "1");
    // T7：该测试仅验证 LoginDisconnect 与 EOF，显式关闭压缩保持帧格式原样。
    let _compression = EnvVarRestore::set("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", "0");

    const PORT: u16 = 25568;
    start_server(PORT);
    let mut stream = connect_ready(PORT);

    // 1) 直连握手：无转发 blob
    let mut hs = Vec::new();
    write_varint(&mut hs, 774);
    write_varint(&mut hs, "localhost".len() as i32);
    hs.extend_from_slice(b"localhost");
    hs.extend_from_slice(&PORT.to_be_bytes());
    write_varint(&mut hs, 2);
    send_frame(&mut stream, 0x00, &hs);

    // 2) 按流程发送 Hello
    let mut hello = Vec::new();
    let name = "Tester";
    write_varint(&mut hello, name.len() as i32);
    hello.extend_from_slice(name.as_bytes());
    hello.push(1u8);
    hello.extend_from_slice(&[0u8; 16]);
    send_frame(&mut stream, 0x00, &hello);

    // 3) 断言收到 LoginDisconnect（登录阶段 clientbound id 0x00）
    let mut got_disconnect = false;
    let mut reason = String::new();
    for _ in 0..50 {
        match read_frame(&mut stream) {
            Some((0x00, body)) => {
                got_disconnect = true;
                reason = read_string(&body);
                break;
            }
            Some(_) => continue,
            None => break,
        }
    }
    assert!(got_disconnect, "enforce 模式下直连未收到 LoginDisconnect");
    assert!(
        reason.contains("Velocity"),
        "LoginDisconnect 理由应含 Velocity，实际：{reason}"
    );

    // 4) 连接随后被服务端主动关闭：读取应返回 EOF（None）
    let next = read_frame(&mut stream);
    assert!(
        next.is_none(),
        "LoginDisconnect 后连接应被关闭（EOF），实际收到额外数据：{next:?}"
    );
}
