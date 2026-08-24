//! 消息子系统（对齐 Java `net.minestom.server.message` 语义，T14）。
//!
//! Java Minestom 的 `Messenger.sendMessage` 以 [`SystemChatPacket`](crate::protocol::packets::play::SystemChatPacket)
//! 承载消息（adventure `Component` NBT 序列化，`overlay=false`）。本模块对齐：
//! [`Messenger::send_message`] 构造 0x77 `SystemChatPacket`，经
//! [`encode_clientbound`](crate::protocol::packets::encode_clientbound) 编码后按
//! `Urgent` 优先级入队（见 `crate::network::client` 三层发包模型），
//! [`Messenger::broadcast_message`] 对全部在线连接广播。
//!
//! `ChatType`（消息类型，含 id + 名称）对齐 Minecraft 1.21.11 `chat_type`
//! 注册表序位（Java `ChatTypeImpl` 经 `DynamicRegistry` 从数据包加载，id 为
//! 注册表出现序位，见 `resources/data/generic/chat_type.toml`：
//! emote_command=0、team_msg_command_incoming=1、team_msg_command_outgoing=2、
//! chat=3、msg_command_incoming=4、msg_command_outgoing=5、say_command=6）。
//! v1 `SystemChatPacket` 线格式不承载 chat type 字段，故 `send_message` 的
//! `chat_type` 参数当前不参与编码（保留签名以对齐 Java 语义，供后续扩展）。
//!
//! 变更标识符：`complete-missing-subsystems`（R14）。

use crate::network::{ClientNetworks, Priority, broadcast, enqueue_packet};
use crate::protocol::packets::encode_clientbound;
use crate::protocol::packets::play::SystemChatPacket;
use crate::text_component::Component;

/// 消息类型（`chat_type` 注册表条目）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatType {
    /// 注册表序位 id（对齐 `chat_type.toml` 出现顺序）。
    pub id: u32,
    /// 命名空间名称（如 `minecraft:chat`）。
    pub name: String,
}

impl ChatType {
    /// `minecraft:chat`（id 3）：普通聊天。
    pub fn chat() -> Self {
        Self::from_registry(3, "minecraft:chat")
    }

    /// `minecraft:say_command`（id 6）：服务器公告（`/say`）。
    pub fn say_command() -> Self {
        Self::from_registry(6, "minecraft:say_command")
    }

    /// `minecraft:team_msg_command_incoming`（id 1）：队伍消息（接收方视角）。
    pub fn team_msg_incoming() -> Self {
        Self::from_registry(1, "minecraft:team_msg_command_incoming")
    }

    /// `minecraft:team_msg_command_outgoing`（id 2）：队伍消息（发送方视角）。
    pub fn team_msg_outgoing() -> Self {
        Self::from_registry(2, "minecraft:team_msg_command_outgoing")
    }

    /// `minecraft:msg_command_incoming`（id 4）：私聊消息（接收方视角）。
    pub fn msg_command_incoming() -> Self {
        Self::from_registry(4, "minecraft:msg_command_incoming")
    }

    /// 以 id 与名称构造。
    pub fn from_registry(id: u32, name: &str) -> Self {
        Self {
            id,
            name: name.to_owned(),
        }
    }
}

/// 内置 `chat_type` 注册表（id ⇄ 名称双向查询）。
#[derive(Debug, Clone, Default)]
pub struct ChatTypeRegistry {
    /// 内置条目（id → 类型），保持插入序。
    entries: Vec<ChatType>,
}

impl ChatTypeRegistry {
    /// 装配全部内置消息类型（Chat/Say/Team 等 5 项，id 对齐 `chat_type.toml`）。
    pub fn builtin() -> Self {
        Self {
            entries: vec![
                ChatType::team_msg_incoming(),
                ChatType::team_msg_outgoing(),
                ChatType::chat(),
                ChatType::msg_command_incoming(),
                ChatType::say_command(),
            ],
        }
    }

    /// 按 id 查询消息类型。
    pub fn by_id(&self, id: u32) -> Option<&ChatType> {
        self.entries.iter().find(|ct| ct.id == id)
    }

    /// 按名称查询消息类型。
    pub fn by_name(&self, name: &str) -> Option<&ChatType> {
        self.entries.iter().find(|ct| ct.name == name)
    }

    /// 条目数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 迭代全部内置消息类型。
    pub fn iter(&self) -> impl Iterator<Item = &ChatType> {
        self.entries.iter()
    }
}

/// 消息类型装饰（前缀 / 后缀组件）。
///
/// v1 仅承载数据（不参与 `SystemChatPacket` 编码），对齐 Java
/// `ChatTypeDecoration`（其含 parameters + translation_key）的最小等价。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatTypeDecoration {
    /// 消息前缀组件。
    pub prefix: Option<Component>,
    /// 消息后缀组件。
    pub suffix: Option<Component>,
}

impl ChatTypeDecoration {
    /// 构造装饰（前缀 / 后缀可选）。
    pub fn new(prefix: Option<Component>, suffix: Option<Component>) -> Self {
        Self { prefix, suffix }
    }
}

/// 消息发送器（无状态命名空间）。
pub struct Messenger;

impl Messenger {
    /// 向单个连接发送一条系统聊天消息（`SystemChatPacket` 0x77，overlay=false）。
    ///
    /// 文本取自 `component` 的纯文本（[`Component::plain_text`]，v1 简化承载）；
    /// 编码后以 `Urgent` 优先级入队。`chat_type` 当前不参与编码（见模块文档）。
    pub fn send_message(
        clients: &mut ClientNetworks,
        conn_id: u32,
        component: &Component,
        _chat_type: &ChatType,
    ) {
        let bytes = system_chat_bytes(component);
        enqueue_packet(clients, conn_id, bytes, Priority::Urgent);
    }

    /// 向全部在线连接广播一条系统聊天消息。
    pub fn broadcast_message(
        clients: &mut ClientNetworks,
        component: &Component,
        _chat_type: &ChatType,
    ) {
        let bytes = system_chat_bytes(component);
        let targets: Vec<u32> = clients.clients.keys().copied().collect();
        broadcast(clients, &targets, &bytes, Priority::Urgent);
    }
}

/// 编码 `SystemChatPacket`（0x77）为完整帧负载（packet_id + 包体）。
fn system_chat_bytes(component: &Component) -> Vec<u8> {
    let packet = SystemChatPacket {
        message: component.plain_text(),
        overlay: false,
    };
    encode_clientbound(&packet)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::network::ClientNetwork;

    /// 构造带两个在线连接的 `ClientNetworks`。
    fn networks_with(conn_ids: &[u32]) -> ClientNetworks {
        let mut networks = ClientNetworks::default();
        for &conn_id in conn_ids {
            networks.clients.insert(conn_id, ClientNetwork::new());
        }
        networks
    }

    #[test]
    fn send_message_enqueues_0x77_system_chat() {
        let mut networks = networks_with(&[1]);
        Messenger::send_message(
            &mut networks,
            1,
            &Component::text("hello"),
            &ChatType::chat(),
        );
        let client = networks.clients.get(&1).expect("连接应存在");
        assert_eq!(client.urgent_queue.len(), 1);
        let frame = client.urgent_queue.first().expect("应入队一帧");
        // packet_id 0x77 为单字节 VarInt（< 0x80），帧首字节即包 id。
        assert_eq!(frame.first(), Some(&0x77));
        // normal_queue 不受影响。
        assert!(client.normal_queue.is_empty());
    }

    #[test]
    fn broadcast_reaches_all_connections() {
        let mut networks = networks_with(&[1, 2, 3]);
        Messenger::broadcast_message(
            &mut networks,
            &Component::text("announcement"),
            &ChatType::say_command(),
        );
        for conn_id in [1u32, 2, 3] {
            let client = networks.clients.get(&conn_id).expect("连接应存在");
            assert_eq!(client.urgent_queue.len(), 1, "conn {conn_id} 应收到广播");
            let frame = client.urgent_queue.first().expect("应入队一帧");
            assert_eq!(frame.first(), Some(&0x77));
        }
        // 不存在的连接不受影响（broadcast 静默跳过）。
        assert!(!networks.clients.contains_key(&99));
    }

    #[test]
    fn component_plain_text_carried_in_payload() {
        let mut networks = networks_with(&[1]);
        let component = Component::text("系统提示: 维护中");
        Messenger::send_message(&mut networks, 1, &component, &ChatType::chat());
        // 解码回 SystemChatPacket 校验消息文本承载。
        let frame = networks
            .clients
            .get(&1)
            .and_then(|c| c.urgent_queue.first())
            .expect("应入队一帧");
        // 包体 = 首字节 packet_id(0x77) + NBT(0x0a + {text:"..."}) + overlay(0)。
        // 直接断言 message 文本出现在字节流中（v1 简化校验承载）。
        let text = component.plain_text();
        let mut found = false;
        for window in frame.windows(text.len()) {
            if window == text.as_bytes() {
                found = true;
                break;
            }
        }
        assert!(found, "包体应包含消息文本 `{text}`");
    }

    #[test]
    fn chat_type_registry_lookup() {
        let registry = ChatTypeRegistry::builtin();
        assert_eq!(registry.len(), 5);
        assert_eq!(registry.by_id(3).expect("chat id=3").name, "minecraft:chat");
        assert_eq!(
            registry.by_id(6).expect("say id=6").name,
            "minecraft:say_command"
        );
        assert!(
            registry
                .by_name("minecraft:team_msg_command_incoming")
                .is_some()
        );
        assert_eq!(registry.by_id(0), None); // emote_command 未内置
        assert_eq!(registry.by_name("minecraft:missing"), None);
    }
}
