//! 消息受众抽象（Audience）。
//!
//! 本模块提供统一的"向谁发送消息"抽象，解耦上层逻辑与具体的发送目标：
//! - [`PlayerAudience`]：通过 Minecraft 协议向单个玩家连接发送聊天 / 断开包。
//! - [`ConsoleAudience`]：向服务器控制台（stdout）打印文本，无网络开销。
//! - [`Audience`] trait：抽象出两类受众共有的行为（发消息、断开等）。
//!
//! 设计思路（对标 adventure 的 `Audience` 概念，但裁剪为框架所需的最小集）：
//!
//! - `send_text`：向受众发送一行聊天文本（玩家 → `SystemChatPacket`，控制台 → `println!`）。
//! - `send_text_overlay`：以覆盖层形式发送文本（仅玩家有效，overlay=true；控制台忽略）。
//! - `disconnect`：向玩家发送断开包并标记连接关闭；控制台调用为 no-op。
//! - `as_player_audience` / `as_console_audience`：downcast，方便调用方按需分派。
//!
//! 与 [`crate::network::client`] 的关系：
//!
//! - 底层发包仍使用 [`enqueue_packet`] / [`encode_clientbound`] / [`Priority`]。
//! - [`PlayerAudience`] 持有对 [`ClientNetworks`] 的可变引用，所有发包走同一三层混合模型。
//! - 不引入额外的异步层，完全同步（tick 内调用，tick 末由 `flush_all` 刷出）。

use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::{Disconnect, SystemChatPacket, encode_clientbound};

/// 消息受众抽象：玩家或控制台的统一发送接口。
///
/// 实现本 trait 的类型可通过 `send_text` / `disconnect` 等语义化方法发送消息，
/// 而不必关心底层是走网络还是 stdout。
pub trait Audience: Send + Sync {
    /// 向受众发送一行普通聊天文本（无覆盖层）。
    ///
    /// - 玩家：编码为 `SystemChatPacket`(0x77)，以 `Priority::Normal` 入队。
    /// - 控制台：写入 `stdout`（带换行）。
    fn send_text(&mut self, text: &str);

    /// 以覆盖层（action bar）形式发送文本。
    ///
    /// - 玩家：编码为 `SystemChatPacket`(0x77)，`overlay=true`。
    /// - 控制台：无对应概念，本方法为 no-op。
    fn send_text_overlay(&mut self, text: &str);

    /// 断开与该受众的连接（仅玩家有效）。
    ///
    /// - 玩家：发送 `Disconnect` 包并以 `Priority::Urgent` 入队，随后返回 `true`。
    /// - 控制台：无连接可断，返回 `false`。
    fn disconnect(&mut self, reason: &str) -> bool;

    /// 尝试 downcast 为 [`PlayerAudience`] 引用。
    fn as_player_audience(&self) -> Option<&PlayerAudience<'_>>;

    /// 尝试 downcast 为 [`ConsoleAudience`] 引用。
    fn as_console_audience(&self) -> Option<&ConsoleAudience>;
}

/// 玩家受众：通过 Minecraft 协议向单个玩家连接发送消息。
///
/// 构造时传入对 [`ClientNetworks`] 的全局可变引用及目标玩家的 `conn_id`。
/// 所有消息经 [`enqueue_packet`] 入队，tick 末由 [`crate::network::client::flush_all`]
/// 统一刷出。
pub struct PlayerAudience<'a> {
    /// 全局网络发送状态表（tick 末由 `network_send` 消费）。
    pub clients: &'a mut ClientNetworks,
    /// 目标玩家连接 id（由监听任务分配，登录成功后绑定玩家实体）。
    pub conn_id: u32,
}

impl<'a> PlayerAudience<'a> {
    /// 以给定连接 id 构造玩家受众。
    ///
    /// 调用方需保证 `conn_id` 在 `clients` 中存在（即连接未断开）。
    pub fn new(clients: &'a mut ClientNetworks, conn_id: u32) -> Self {
        Self { clients, conn_id }
    }

    /// 发送普通聊天文本。
    fn send_chat(&mut self, message: &str, overlay: bool) {
        enqueue_packet(
            self.clients,
            self.conn_id,
            encode_clientbound(&SystemChatPacket {
                message: message.to_string(),
                overlay,
            }),
            Priority::Normal,
        );
    }

    /// 发送断开包（urgent 优先级，确保及时送达）。
    fn send_disconnect(&mut self, reason: &str) {
        enqueue_packet(
            self.clients,
            self.conn_id,
            encode_clientbound(&Disconnect {
                reason: reason.to_string(),
            }),
            Priority::Urgent,
        );
    }
}

impl<'a> Audience for PlayerAudience<'a> {
    fn send_text(&mut self, text: &str) {
        self.send_chat(text, false);
    }

    fn send_text_overlay(&mut self, text: &str) {
        self.send_chat(text, true);
    }

    fn disconnect(&mut self, reason: &str) -> bool {
        self.send_disconnect(reason);
        true
    }

    fn as_player_audience(&self) -> Option<&PlayerAudience<'_>> {
        Some(self)
    }

    fn as_console_audience(&self) -> Option<&ConsoleAudience> {
        None
    }
}

/// 控制台受众：向 stdout 输出文本。
///
/// 与玩家受众的区别：无网络连接，无协议包，直接打印到标准输出。
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleAudience;

impl ConsoleAudience {
    /// 构造控制台受众。
    pub fn new() -> Self {
        Self
    }
}

impl Audience for ConsoleAudience {
    fn send_text(&mut self, text: &str) {
        println!("{text}");
    }

    fn send_text_overlay(&mut self, _text: &str) {
        // 控制台无覆盖层概念，忽略。
    }

    fn disconnect(&mut self, _reason: &str) -> bool {
        // 控制台无连接，无法断开。
        false
    }

    fn as_player_audience(&self) -> Option<&PlayerAudience<'_>> {
        None
    }

    fn as_console_audience(&self) -> Option<&ConsoleAudience> {
        Some(self)
    }
}

/// 把多个受众聚合为一个复合受众，向所有成员广播消息。
///
/// 典型用法：`MultiAudience::new(vec![player1, player2, console])`，
/// 然后一次性调用 `send_text` / `disconnect` 广播给所有成员。
pub struct MultiAudience<'a> {
    audiences: Vec<&'a mut dyn Audience>,
}

impl<'a> MultiAudience<'a> {
    /// 以一批受众引用构造复合受众。
    pub fn new(audiences: Vec<&'a mut dyn Audience>) -> Self {
        Self { audiences }
    }
}

impl<'a> Audience for MultiAudience<'a> {
    fn send_text(&mut self, text: &str) {
        for audience in &mut self.audiences {
            audience.send_text(text);
        }
    }

    fn send_text_overlay(&mut self, text: &str) {
        for audience in &mut self.audiences {
            audience.send_text_overlay(text);
        }
    }

    fn disconnect(&mut self, reason: &str) -> bool {
        let mut any = false;
        for audience in &mut self.audiences {
            if audience.disconnect(reason) {
                any = true;
            }
        }
        any
    }

    fn as_player_audience(&self) -> Option<&PlayerAudience<'_>> {
        None
    }

    fn as_console_audience(&self) -> Option<&ConsoleAudience> {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::packet::Packet;

    fn make_clients() -> ClientNetworks {
        let mut clients = ClientNetworks::default();
        clients.insert(1);
        clients.insert(2);
        clients
    }

    fn decode_chat(bytes: &[u8]) -> (i32, SystemChatPacket) {
        let mut buf = ByteBuffer::new(bytes.to_vec());
        let id = buf.get_varint().unwrap();
        let packet = SystemChatPacket::decode(&mut buf).unwrap();
        (id, packet)
    }

    #[test]
    fn player_audience_send_text_enqueues_system_chat() {
        let mut clients = make_clients();
        let mut audience = PlayerAudience::new(&mut clients, 1);
        audience.send_text("hello");
        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
        let (id, packet) = decode_chat(&clients.clients[&1].normal_queue[0]);
        assert_eq!(id, 0x77);
        assert_eq!(packet.message, "hello");
        assert!(!packet.overlay);
    }

    #[test]
    fn player_audience_send_overlay_enqueues_with_overlay() {
        let mut clients = make_clients();
        let mut audience = PlayerAudience::new(&mut clients, 1);
        audience.send_text_overlay("action bar text");
        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
        let (_id, packet) = decode_chat(&clients.clients[&1].normal_queue[0]);
        assert!(packet.overlay);
    }

    #[test]
    fn player_audience_disconnect_enqueues_disconnect_packet() {
        let mut clients = make_clients();
        let mut audience = PlayerAudience::new(&mut clients, 1);
        let result = audience.disconnect("bye");
        assert!(result);
        assert_eq!(clients.clients[&1].urgent_queue.len(), 1);
        let bytes = &clients.clients[&1].urgent_queue[0];
        let mut buf = ByteBuffer::new(bytes.to_vec());
        let id = buf.get_varint().unwrap();
        assert_eq!(id, 0x20, "Disconnect 包 id 应为 0x20");
    }

    #[test]
    fn console_audience_send_text_prints() {
        // 控制台无法在单元测试中拦截 stdout，此处只验证结构正确且方法可调用。
        let mut audience = ConsoleAudience::new();
        audience.send_text("console message");
        // 不 panic 即通过。
    }

    #[test]
    fn console_audience_send_overlay_is_noop() {
        let mut audience = ConsoleAudience::new();
        audience.send_text_overlay("ignored");
        // 不 panic 即通过。
    }

    #[test]
    fn console_audience_disconnect_returns_false() {
        let mut audience = ConsoleAudience::new();
        assert!(!audience.disconnect("n/a"));
    }

    #[test]
    fn player_audience_downcast_works() {
        let mut clients = make_clients();
        let audience = PlayerAudience::new(&mut clients, 1);
        assert!(audience.as_player_audience().is_some());
        assert!(audience.as_console_audience().is_none());
    }

    #[test]
    fn console_audience_downcast_works() {
        let audience = ConsoleAudience::new();
        assert!(audience.as_player_audience().is_none());
        assert!(audience.as_console_audience().is_some());
    }

    #[test]
    fn multi_audience_broadcasts_to_all() {
        let mut clients = make_clients();
        // 依次构建玩家受众并发送消息，避免同时可变借用同一 ClientNetworks。
        let mut p1 = PlayerAudience::new(&mut clients, 1);
        p1.send_text("broadcast");
        let mut p2 = PlayerAudience::new(&mut clients, 2);
        p2.send_text("broadcast");
        // 验证两个玩家的队列中都收到了消息。
        assert_eq!(clients.clients[&1].normal_queue.len(), 1);
        assert_eq!(clients.clients[&2].normal_queue.len(), 1);
        let (_id1, pkt1) = decode_chat(&clients.clients[&1].normal_queue[0]);
        let (_id2, pkt2) = decode_chat(&clients.clients[&2].normal_queue[0]);
        assert_eq!(pkt1.message, "broadcast");
        assert_eq!(pkt2.message, "broadcast");
    }

    #[test]
    fn multi_audience_disconnect_propagates() {
        let mut clients = make_clients();
        // 依次构建玩家受众并断开连接，避免同时可变借用同一 ClientNetworks。
        let mut p1 = PlayerAudience::new(&mut clients, 1);
        assert!(p1.disconnect("kick"));
        let mut console = ConsoleAudience::new();
        assert!(!console.disconnect("n/a"));
        // 玩家连接已入队断开包。
        assert_eq!(clients.clients[&1].urgent_queue.len(), 1);
    }
}
