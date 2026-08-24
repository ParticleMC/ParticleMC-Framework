//! 库存同步系统：登录/光标变化全量下发 + 脏槽增量下发。
//!
//! 每个 tick 对带 [`PlayerInventory`] 的玩家实体执行：
//! 1. 仅当 `full_sync` 置位（登录首 tick / 点击 / 关窗清空光标）时全量下发
//!    [`WindowItemsPacket`]（窗口序 46 项 + 光标），消费后清零——避免每 tick
//!    全量下发（既有缺陷，会导致真实 TCP 集成测试 `collect_until` 永不空闲）；
//! 2. 非 AIR 装备下发 [`EntityEquipmentPacket`]（主手 / 副手 / 盔甲）；
//! 3. 脏槽增量下发 [`SetSlotPacket`] 后清空脏集合。
//!
//! 实体 id 沿用 `entity_sync` 的约定：使用连接号（`conn_id`）作为对端可见实体标识；
//! 通过 [`ConnectionManager`] 反查 `entity → conn_id`（复用 `entity_sync::connection_of`）。
//!
//! 见 `.specs/implement-item-inventory/` 与 `.specs/complete-partial-framework-capabilities/`。

use crate::prelude::{Entity, Res, ResMut};

use crate::component::inventory::{
    PlayerInventory, convert_minestom_slot_to_window_slot, window_slot_to_minestom_slot,
};
use crate::component::player::Player;
use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::{
    EntityEquipmentPacket, EquipmentEntry, EquipmentSlot, SetSlotPacket, WindowItemsPacket,
    encode_clientbound,
};
use crate::resource::connection_manager::ConnectionManager;
use crate::system::entity_sync::connection_of;
use particlemc_framework_ecs::scheduler::InstanceScheduler;

/// 库存同步系统：登录全量下发 + 脏槽增量下发。
///
/// 玩家实体已迁入实例 World（R11.2）：本系统留主 World，遍历各实例 World
/// 收集其玩家实体与库存（跨 World 只读），随后写主 World 资源 `ClientNetworks`。
pub fn inventory_sync(
    scheduler: Res<InstanceScheduler>,
    connections: Res<ConnectionManager>,
    mut clients: ResMut<ClientNetworks>,
) {
    for wid in scheduler.world_ids() {
        let Some(mut guard) = scheduler.lock_world(wid) else {
            continue;
        };
        for (entity, _player, inv) in guard
            .world()
            .query_mut::<(Entity, &Player, &mut PlayerInventory), ()>()
            .iter_mut()
        {
            // 反查该玩家实体对应的连接号；无则跳过（不 panic）。
            let Some(conn_id) = connection_of(&connections, entity) else {
                continue;
            };

            // 1) 全量下发 WindowItemsPacket（窗口序 46 项 + 光标）——仅当 full_sync
            //    置位时（登录首 tick / 点击 / 关窗清空光标），消费后清零。
            //    避免每 tick 全量下发导致真实客户端/集成测试永不空闲（既有缺陷，
            //    见 `.specs/complete-partial-framework-capabilities/`）。
            if inv.full_sync {
                let mut window_items = Vec::with_capacity(46);
                for w in 0..46i32 {
                    let internal = window_slot_to_minestom_slot(w);
                    // internal ∈ [0,45]，恒合法；缩窄用 TryFrom，失败兜底为 0（不会触发）。
                    let idx = usize::try_from(internal).unwrap_or(0);
                    window_items.push(inv.get(idx));
                }
                let window_pkt = WindowItemsPacket {
                    window_id: 0,
                    state_id: 0,
                    items: window_items,
                    carried_item: inv.cursor.clone(),
                };
                enqueue_packet(
                    &mut clients,
                    conn_id,
                    encode_clientbound(&window_pkt),
                    Priority::Normal,
                );
                inv.full_sync = false;
            }

            // 2) 装备下发 EntityEquipmentPacket（仅非 AIR 项；空则跳过以免协议报错）。
            let mut equipments = Vec::new();
            let held = inv.get(usize::from(inv.held_slot));
            if !held.is_air() {
                equipments.push(EquipmentEntry {
                    slot: EquipmentSlot::MainHand,
                    item: held,
                });
            }
            let off = inv.get(45);
            if !off.is_air() {
                equipments.push(EquipmentEntry {
                    slot: EquipmentSlot::OffHand,
                    item: off,
                });
            }
            let armor = [
                (EquipmentSlot::Helmet, 41),
                (EquipmentSlot::Chestplate, 42),
                (EquipmentSlot::Leggings, 43),
                (EquipmentSlot::Boots, 44),
            ];
            for (slot_enum, s) in armor {
                // s ∈ {41,42,43,44}，恒合法；缩窄用 TryFrom，失败兜底为 0（不会触发）。
                let it = inv.get(usize::try_from(s).unwrap_or(0));
                if !it.is_air() {
                    equipments.push(EquipmentEntry {
                        slot: slot_enum,
                        item: it,
                    });
                }
            }
            if !equipments.is_empty() {
                let eq_pkt = EntityEquipmentPacket {
                    entity_id: conn_id as i32,
                    equipments,
                };
                enqueue_packet(
                    &mut clients,
                    conn_id,
                    encode_clientbound(&eq_pkt),
                    Priority::Normal,
                );
            }

            // 3) 脏槽增量下发 SetSlotPacket，随后清空脏集合。
            let dirty: Vec<u8> = inv.dirty.iter().copied().collect();
            inv.dirty.clear();
            for s in dirty {
                let window_slot = convert_minestom_slot_to_window_slot(i32::from(s));
                // 窗口槽 ∈ [0,45]，恒可落入 i16；缩窄用 TryFrom，失败兜底为 0（不会触发）。
                let slot_i16 = i16::try_from(window_slot).unwrap_or(0);
                let pkt = SetSlotPacket {
                    window_id: 0,
                    state_id: 0,
                    slot: slot_i16,
                    item: inv.get(usize::from(s)),
                };
                enqueue_packet(
                    &mut clients,
                    conn_id,
                    encode_clientbound(&pkt),
                    Priority::Normal,
                );
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use crate::app::App;
    use uuid::Uuid;

    use crate::component::inventory::PlayerInventory;
    use crate::component::{Health, InstanceRef, Player, Position, Velocity};
    use crate::item_stack::ItemStack;
    use crate::network::bridge::empty_bridge;
    use crate::network::client::ClientNetworks;
    use crate::network::connection::ConnectionState;
    use crate::network::listener::OutboundMessage;
    use crate::plugin::McServerPlugin;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::framing::decode_frame;
    use crate::resource::connection_manager::ConnectionManager;
    use crate::test_support::{
        current_test_instance, ensure_test_instance, spawn_into_instance, with_instance_entity,
    };

    /// 构造测试 App：装配插件 + 空桥接 + 手动 20Hz 步进，预热一帧。
    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());
        let (bridge, _frame_tx, _outbound) = empty_bridge();
        app.world_mut().insert_resource(bridge);
        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(50),
            ));
        // 首帧为热身帧（旧 ECS 方案 的 `Time<Real>` 首次 update 不产 delta），空转一次。
        app.update();
        app
    }

    /// 把 conn → entity 绑定到 `ConnectionManager` 并注册出站通道，返回出站接收端。
    fn bind_client(
        app: &mut App,
        conn_id: u32,
        entity: crate::prelude::Entity,
    ) -> tokio::sync::mpsc::Receiver<OutboundMessage> {
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(64);
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .clients
            .insert(conn_id, crate::network::client::ClientNetwork::new());
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
        out_rx
    }

    /// 生成带完整组件（含 `PlayerInventory`）的玩家实体，落入惰性单例实例 World。
    ///
    /// 多名玩家共享同一实例 World（[`ensure_test_instance`]），避免各 World 首个
    /// spawn 的 `Entity` id 冲突；`InstanceRef` 指向玩家自身所在实例。
    fn spawn_player_with_inventory(
        app: &mut App,
        name: &str,
        x: f64,
        z: f64,
    ) -> crate::prelude::Entity {
        let inst = ensure_test_instance(app);
        spawn_into_instance(
            app,
            inst,
            (
                Player::new(Uuid::new_v4(), name),
                Position::new(x, 64.0, z),
                Health::new(20.0, 20.0),
                Velocity::zero(),
                InstanceRef(inst),
                PlayerInventory::new(),
            ),
        )
    }

    /// 读出出站通道全部已 flush 的完整帧，剥掉帧长度前缀后返回 payload 列表。
    ///
    /// 注意：普通优先级包会被 [`flush_all`] 按 MTU 聚合进单个 `OutboundMessage::Frame`
    /// （多帧串联），需循环 `decode_frame` 解尽消息内全部帧，否则只拿到首帧。
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

    /// L1 集成：全量 WindowItems(0x12,46 槽) + 装备 EntityEquipment(0x64) + 脏槽 SetSlot(0x14,窗口序36)。
    #[test]
    fn inventory_full_and_dirty_sync() {
        let mut app = build_app();
        let entity = spawn_player_with_inventory(&mut app, "Tester", 8.0, 8.0);
        let mut rx = bind_client(&mut app, 1, entity);

        // 内部槽 0（窗口序 36、held_slot=0 的主手）放钻石：触发脏槽 + 主手装备。
        let inst = current_test_instance(&app);
        with_instance_entity::<PlayerInventory, _>(&mut app, inst, entity, |inv| {
            inv.set(0, ItemStack::new(264, 1));
        });

        // 首次 update：inventory_sync → enqueue_packet → flush_all。
        app.update();
        let payloads = capture(&mut rx);
        let ids: Vec<i32> = payloads.iter().map(|p| payload_id(p)).collect();

        // 1) 全量下发 WindowItemsPacket(0x12)，46 槽。
        assert!(
            ids.contains(&0x12),
            "应下发 WindowItemsPacket(0x12)，实际 {ids:?}"
        );
        let window = payloads
            .iter()
            .find(|p| payload_id(p) == 0x12)
            .expect("WindowItemsPacket 存在");
        let mut buf = ByteBuffer::new(window.to_vec());
        let _ = buf.get_varint(); // packet_id
        let _ = buf.get_varint(); // window_id
        let _ = buf.get_varint(); // state_id
        let count = buf.get_varint().expect("应可读到槽数 VarInt");
        assert_eq!(count, 46, "WindowItemsPacket 应含 46 槽，实际 {count}");

        // 2) 装备下发 EntityEquipmentPacket(0x64)（主手为钻石）。
        assert!(
            ids.contains(&0x64),
            "应下发 EntityEquipmentPacket(0x64)，实际 {ids:?}"
        );

        // 3) 脏槽增量下发 SetSlotPacket(0x14)，窗口序 36（内部槽 0）。
        assert!(
            ids.contains(&0x14),
            "应下发 SetSlotPacket(0x14)，实际 {ids:?}"
        );
        let set_slot = payloads
            .iter()
            .find(|p| payload_id(p) == 0x14)
            .expect("SetSlotPacket 存在");
        let mut buf = ByteBuffer::new(set_slot.to_vec());
        let _ = buf.get_varint(); // packet_id
        let _ = buf.get_varint(); // window_id
        let _ = buf.get_varint(); // state_id
        let slot = buf.get_i16().expect("应可读到 SetSlot.slot");
        assert_eq!(slot, 36, "SetSlot 窗口序应为 36（内部槽 0），实际 {slot}");

        // 再次变更内部槽 0（窗口序 36）为不同数量钻石，触发第二次脏槽下发。
        with_instance_entity::<PlayerInventory, _>(&mut app, inst, entity, |inv| {
            inv.set(0, ItemStack::new(264, 2));
        });
        app.update();
        let payloads = capture(&mut rx);
        let ids: Vec<i32> = payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            ids.contains(&0x14),
            "再次 update 应再次下发 SetSlotPacket(0x14)，实际 {ids:?}"
        );
        let set_slot = payloads
            .iter()
            .find(|p| payload_id(p) == 0x14)
            .expect("第二次 SetSlotPacket 存在");
        let mut buf = ByteBuffer::new(set_slot.to_vec());
        let _ = buf.get_varint();
        let _ = buf.get_varint();
        let _ = buf.get_varint();
        let slot = buf.get_i16().expect("应可读到 SetSlot.slot");
        assert_eq!(slot, 36, "第二次 SetSlot 窗口序应为 36，实际 {slot}");
    }
}
