//! 属性同步系统（R8）：消费 [`AttributeInbox`] 收件箱，向玩家广播
//! `EntityAttributes`(0x81) 更新包。
//!
//! 收件箱解耦模式与 `command_inbox` / `packet_inbox` 一致：应用侧或后续系统
//! 把 `(entity, attr_id)` 压入 [`AttributeInbox`]，本系统在每个 tick 消费
//! 并对每个变更实体构造单属性更新包：
//! - 仅 `client_sync == true` 的属性进入包体；
//! - `attribute_id` 用注册表 id、`value` 用 [`AttributeInstance::value`]、
//!   `modifiers` 按线格式转换；
//! - 实体 id 沿用连接号（`conn_id`）。
//!
//! 登录流程不主动下发：收件箱初始为空，玩家生成挂载
//! [`crate::component::Attributes`] 组件不产生任何事件，天然不广播，
//! 故不破坏既有 `fake_client_login` 包序列断言。
//!
//! 见 `.specs/complete-partial-framework-capabilities/`（R8）。

use crate::prelude::{Entity, Query, Res, ResMut};

use crate::component::Attributes;
use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::encode_clientbound;
use crate::protocol::packets::play::{
    AttributeModifier as WireModifier, AttributeProperty, EntityAttributes,
};
use crate::resource::connection_manager::ConnectionManager;
use crate::system::entity_sync::connection_of;

/// 属性同步收件箱：`(实体, 属性注册表 id)` 变更事件列表。
///
/// 由应用侧或未来系统写入（如 `Health` 变化联动 `max_health` 同步）；
/// [`attribute_sync`] 系统在本 tick 消费后清空。
#[derive(Default, Debug)]
pub struct AttributeInbox {
    /// 待同步的 `(entity, attr_id)` 事件。
    pub events: Vec<(Entity, u32)>,
}

/// 消费收件箱并广播属性更新包。无绑定连接 / 无属性实例 / 非 client_sync
/// 属性均被跳过（不 panic）。
pub fn attribute_sync(
    players: Query<(Entity, &Attributes)>,
    mut inbox: ResMut<AttributeInbox>,
    connections: Res<ConnectionManager>,
    mut clients: ResMut<ClientNetworks>,
) {
    // 取出整段收件箱（move 出来，避免与后续可变借用冲突）。
    let events = std::mem::take(&mut inbox.events);
    for (entity, attr_id) in events {
        let Some(conn_id) = connection_of(&connections, entity) else {
            continue;
        };
        let Ok((_, attributes)) = players.get(entity) else {
            continue;
        };
        let Some(instance) = attributes.get(attr_id) else {
            continue;
        };
        if !instance.attribute.client_sync {
            continue;
        }
        let modifiers: Vec<WireModifier> = instance
            .modifiers()
            .iter()
            .map(|m| WireModifier {
                modifier_id: m.id.clone(),
                amount: m.amount,
                operation: m.operation.wire_value(),
            })
            .collect();
        let packet = EntityAttributes {
            entity_id: i32::try_from(conn_id).unwrap_or(0),
            properties: vec![AttributeProperty {
                attribute_id: i32::try_from(attr_id).unwrap_or(0),
                value: instance.value(),
                modifiers,
            }],
        };
        enqueue_packet(
            &mut clients,
            conn_id,
            encode_clientbound(&packet),
            Priority::Normal,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use crate::app::App;

    use super::*;
    use crate::component::Player;
    use crate::network::bridge::empty_bridge;
    use crate::network::client::ClientNetwork;
    use crate::network::connection::ConnectionState;
    use crate::network::listener::OutboundMessage;
    use crate::plugin::McServerPlugin;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::framing::decode_frame;
    use crate::protocol::packet::Packet;
    use crate::resource::attribute::{
        Attribute, AttributeInstance, AttributeModifier, AttributeOperation,
    };
    use crate::resource::connection_manager::ConnectionManager;

    fn attr(id: u32, name: &str, client_sync: bool) -> Attribute {
        Attribute {
            name: name.to_string(),
            id,
            default_value: 10.0,
            min_value: 0.0,
            max_value: 100.0,
            client_sync,
        }
    }

    /// 构造测试 App（装配插件 + 空桥接 + 手动 20Hz 步进），预热一帧。
    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());
        let (bridge, _frame_tx, _outbound) = empty_bridge();
        app.world_mut().insert_resource(bridge);
        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(50),
            ));
        // 首帧为热身帧（旧 ECS 方案 `Time<Real>` 首次 update 不产 delta），空转一次。
        app.update();
        app
    }

    /// 生成带 `Attributes` 与 `Player` 组件的实体，绑定 conn → entity，
    /// 注册出站通道，返回 (entity, 出站接收端)。
    fn spawn_player(
        app: &mut App,
        conn_id: u32,
    ) -> (Entity, tokio::sync::mpsc::Receiver<OutboundMessage>) {
        let entity = app
            .world_mut()
            .spawn_bundle((
                Player::new(uuid::Uuid::new_v4(), "Tester"),
                Attributes::default(),
            ))
            .id();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(64);
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .clients
            .insert(conn_id, ClientNetwork::new());
        {
            let bridge = app
                .world()
                .resource::<crate::network::bridge::NetworkBridge>()
                .unwrap();
            bridge.outbound.lock().unwrap().insert(conn_id, out_tx);
        }
        let cm = app.world_mut().resource_mut::<ConnectionManager>().unwrap();
        let rt = cm.open(conn_id, None);
        rt.entity = Some(entity);
        rt.state = ConnectionState::Play;
        (entity, out_rx)
    }

    /// 读出出站通道全部已 flush 的完整帧，剥掉帧长度前缀后返回 payload 列表。
    fn capture(rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Frame(frame) = msg {
                let mut pos = 0usize;
                while pos < frame.len() {
                    match decode_frame(&frame, &mut pos) {
                        Ok(payload) => out.push(payload),
                        Err(_) => break,
                    }
                }
            }
        }
        out
    }

    /// 解析 payload 首字段（packet_id VarInt）。
    fn payload_id(payload: &[u8]) -> i32 {
        let mut buf = ByteBuffer::new(payload.to_vec());
        buf.get_varint().unwrap_or(-1)
    }

    /// 解码 `EntityAttributes`(0x81) 包体，非该包返回 `None`。
    fn decode_attributes(payload: &[u8]) -> Option<EntityAttributes> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        if buf.get_varint().ok()? != 0x81 {
            return None;
        }
        EntityAttributes::decode(&mut buf).ok()
    }

    #[test]
    fn inbox_event_broadcasts_single_attribute_packet() {
        let mut app = build_app();
        let (entity, mut rx) = spawn_player(&mut app, 1);
        // 给 max_health（client_sync=true）添加 ADD +5 修饰器。
        let mut attributes = Attributes::default();
        let mut instance = AttributeInstance::new(attr(19, "minecraft:max_health", true));
        instance.add_modifier(AttributeModifier {
            id: "minecraft:test_mod".into(),
            amount: 5.0,
            operation: AttributeOperation::Add,
        });
        attributes.insert(instance);
        let _ = app.world_mut().insert(entity, attributes);

        // 压入收件箱事件并推进一个 tick。
        app.world_mut()
            .resource_mut::<AttributeInbox>()
            .unwrap()
            .events
            .push((entity, 19));
        app.update();

        // 0x81 包字段正确：entity_id=1、attribute_id=19、value=15、modifiers 1 条。
        let payloads = capture(&mut rx);
        let ids: Vec<i32> = payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            ids.contains(&0x81),
            "应下发 EntityAttributes(0x81)，实际 {ids:?}"
        );
        let decoded = payloads
            .iter()
            .find_map(|p| decode_attributes(p))
            .expect("0x81 包可解码");
        assert_eq!(decoded.entity_id, 1);
        assert_eq!(decoded.properties.len(), 1);
        let prop = &decoded.properties[0];
        assert_eq!(prop.attribute_id, 19);
        assert_eq!(prop.value, 15.0);
        assert_eq!(prop.modifiers.len(), 1);
        assert_eq!(prop.modifiers[0].modifier_id, "minecraft:test_mod");
        assert_eq!(prop.modifiers[0].amount, 5.0);
        assert_eq!(prop.modifiers[0].operation, 0);

        // 收件箱消费后清空。
        assert!(
            app.world()
                .resource::<AttributeInbox>()
                .unwrap()
                .events
                .is_empty()
        );
    }

    #[test]
    fn non_client_sync_attribute_is_not_broadcast() {
        let mut app = build_app();
        let (entity, mut rx) = spawn_player(&mut app, 1);
        let mut attributes = Attributes::default();
        attributes.insert(AttributeInstance::new(attr(
            13,
            "minecraft:follow_range",
            false,
        )));
        let _ = app.world_mut().insert(entity, attributes);

        app.world_mut()
            .resource_mut::<AttributeInbox>()
            .unwrap()
            .events
            .push((entity, 13));
        app.update();

        let payloads = capture(&mut rx);
        assert!(
            !payloads.iter().any(|p| payload_id(p) == 0x81),
            "client_sync=false 属性不应下发 0x81"
        );
        // 收件箱仍被消费清空（事件已处理，只是被过滤）。
        assert!(
            app.world()
                .resource::<AttributeInbox>()
                .unwrap()
                .events
                .is_empty()
        );
    }

    #[test]
    fn login_spawn_does_not_broadcast_attributes() {
        // 收件箱初始为空：即使玩家实体已挂载 Attributes，也不产生任何 0x81。
        let mut app = build_app();
        let (entity, mut rx) = spawn_player(&mut app, 1);
        let mut attributes = Attributes::default();
        attributes.insert(AttributeInstance::new(attr(
            19,
            "minecraft:max_health",
            true,
        )));
        let _ = app.world_mut().insert(entity, attributes);

        app.update();
        let payloads = capture(&mut rx);
        assert!(
            !payloads.iter().any(|p| payload_id(p) == 0x81),
            "登录/生成时不应主动下发属性包"
        );
    }

    #[test]
    fn unknown_entity_or_missing_instance_is_skipped() {
        let mut app = build_app();
        // 无绑定的孤立实体事件：跳过不 panic。
        let orphan = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<AttributeInbox>()
            .unwrap()
            .events
            .push((orphan, 19));
        app.update();
        assert!(
            app.world()
                .resource::<AttributeInbox>()
                .unwrap()
                .events
                .is_empty()
        );
    }
}
