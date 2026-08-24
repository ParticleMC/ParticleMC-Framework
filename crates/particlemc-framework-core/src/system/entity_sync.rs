// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线：玩家实体网络同步。
//!
//! 消费 [`EnterPlayEvent`] / [`PlayerQuit`]，实现玩家互见：
//! - 新玩家进入 Play 时，向所有已在线玩家广播其 `SpawnEntity` + `PlayerInfo`；
//!   同时把已在线玩家以 `SpawnEntity` + `PlayerInfo` 发给新玩家。
//! - 玩家断开时，向其他在线玩家广播 `PlayerRemove`。
//! - 玩家移动时（消费 `PlayerMove`），向其他玩家广播位移同步（相对位移或绝对传送）。
//!
//! 同时消费 [`EntitySpawn`] / [`EntityRemove`]（批2-A 新增），实现通用实体
//! 生成 / 移除广播：
//! - [`EntitySpawn`] 向其他在线玩家广播 `SpawnEntity`；玩家实体额外广播 `PlayerInfo`。
//! - [`EntityRemove`] 向其他在线玩家广播 `DestroyEntities`。
//!
//! 实体 id 使用连接号（`conn_id`）作为对端可见的玩家实体标识；通用实体使用
//! 旧 ECS 方案 `Entity` 的 `index_u32()`。单服场景下二者全局唯一且稳定，作为最小实现
//! 足够；多世界 / 多实体场景需替换为独立分配器。

use crate::prelude::{Entity, MessageReader, Res, ResMut};
use std::collections::HashMap;
use uuid::Uuid;

use crate::component::{EntityMeta, Player, Position};
use crate::event::{EnterPlayEvent, EntityRemove, EntitySpawn, PlayerMove, PlayerQuit};
use crate::network::client::{ClientNetworks, Priority, broadcast, enqueue_packet};
use crate::protocol::packets::{
    DestroyEntities, EntityTeleport, PlayerInfo, PlayerRemove, RelEntityMove, SpawnEntity,
    encode_clientbound,
};
use crate::resource::connection_manager::ConnectionManager;
use particlemc_framework_ecs::scheduler::{InstanceScheduler, WorldId};

/// 位移同步阈值：位移平方超过该值时改用绝对传送（EntityTeleport）。
const TELEPORT_DISTANCE_SQ: f64 = 64.0 * 64.0;
/// 玩家实体类型 id（`minecraft:player` 在 entity_type 注册表中的协议序号）。
/// 1.21.11 数据中玩家实体序数为 74（由 Task 0 数据校准）。
const PLAYER_ENTITY_TYPE: i32 = 74;

/// 玩家进入 / 离开 / 移动，以及通用实体生成 / 移除时的实体网络同步。
///
/// 实体已迁入实例 World（R11.2）：本系统留主 World，经 `scheduler` 跨 World
/// 只读收集各实例中的玩家实体（`Player` / `Position`），随后写主 World 资源
/// `ClientNetworks` 与连接表。
///
/// 参数数量豁免：系统需要消费多个事件流并访问多个资源，参数列表较长，
/// 但这是 ECS 系统函数的标准模式。
#[allow(clippy::too_many_arguments)]
pub fn entity_sync(
    enter_events: MessageReader<EnterPlayEvent>,
    quit_events: MessageReader<PlayerQuit>,
    move_events: MessageReader<PlayerMove>,
    spawn_events: MessageReader<EntitySpawn>,
    remove_events: MessageReader<EntityRemove>,
    mut clients: ResMut<ClientNetworks>,
    connections: Res<ConnectionManager>,
    scheduler: Res<InstanceScheduler>,
) {
    // R11.2：跨 World 收集全部实例中的玩家实体（Player + Position）与非玩家实体元数据，供后续
    // 事件消费只读反查（实体已不在主 World）。
    let mut entity_map: HashMap<Entity, (Player, Position)> = HashMap::new();
    let mut meta_map: HashMap<Entity, EntityMeta> = HashMap::new();
    for wid in scheduler.world_ids() {
        if let Some(guard) = scheduler.lock_world(wid) {
            for (e, player, pos) in guard.query::<(Entity, &Player, &Position), ()>().iter() {
                entity_map.insert(e, (player.clone(), *pos));
            }
            for (e, meta) in guard.query::<(Entity, &EntityMeta), ()>().iter() {
                meta_map.insert(e, meta.clone());
            }
            drop(guard);
        }
    }
    // ---- 进入 Play：广播互见 ----
    for event in enter_events.read() {
        let Some((new_player, new_pos)) = entity_map.get(&event.entity) else {
            continue;
        };
        let Some(new_conn) = connection_of(&connections, event.entity) else {
            continue;
        };
        // 1) 已在线玩家 → 新玩家（SpawnEntity + PlayerInfo）。
        for (conn_id, runtime) in connections.iter() {
            if *conn_id == new_conn {
                continue;
            }
            let Some(existing_entity) = runtime.entity else {
                continue;
            };
            let Some((existing_player, existing_pos)) = entity_map.get(&existing_entity) else {
                continue;
            };
            enqueue_packet(
                &mut clients,
                new_conn,
                encode_clientbound(&spawn_entity_packet(
                    *conn_id,
                    existing_player,
                    existing_pos,
                )),
                Priority::Urgent,
            );
            enqueue_packet(
                &mut clients,
                new_conn,
                encode_clientbound(&player_info_packet(existing_player)),
                Priority::Urgent,
            );
        }
        // 2) 新玩家 → 已在线玩家。
        let targets: Vec<u32> = connections
            .iter()
            .filter(|(conn_id, runtime)| {
                **conn_id != new_conn
                    && runtime.entity.is_some()
                    && runtime.state == crate::network::connection::ConnectionState::Play
            })
            .map(|(conn_id, _)| *conn_id)
            .collect();
        if !targets.is_empty() {
            let spawn = encode_clientbound(&spawn_entity_packet(new_conn, new_player, new_pos));
            broadcast(&mut clients, &targets, &spawn, Priority::Urgent);
            let info = encode_clientbound(&player_info_packet(new_player));
            broadcast(&mut clients, &targets, &info, Priority::Urgent);
        }
    }

    // ---- 离开：广播 PlayerRemove ----
    for event in quit_events.read() {
        let Some((player, _)) = entity_map.get(&event.entity) else {
            continue;
        };
        let Some(quit_conn) = connection_of(&connections, event.entity) else {
            continue;
        };
        let targets: Vec<u32> = connections
            .iter()
            .filter(|(conn_id, runtime)| {
                **conn_id != quit_conn
                    && runtime.entity.is_some()
                    && runtime.state == crate::network::connection::ConnectionState::Play
            })
            .map(|(conn_id, _)| *conn_id)
            .collect();
        if !targets.is_empty() {
            let remove = encode_clientbound(&PlayerRemove {
                players: vec![player.uuid()],
            });
            broadcast(&mut clients, &targets, &remove, Priority::Urgent);
        }
    }

    // ---- 移动：广播位移同步（批量合并 + 距离裁剪）----
    // 同 tick 内同一玩家的多次移动仅保留最后一次目标坐标，避免重复序列化与广播。
    let mut move_by_entity: HashMap<Entity, (Position, Position)> = HashMap::new();
    for event in move_events.read() {
        move_by_entity
            .entry(event.entity)
            .and_modify(|(_, to)| *to = event.to)
            .or_insert((event.from, event.to));
    }
    for (mover_entity, (from_pos, to_pos)) in move_by_entity {
        let Some(mover_conn) = connection_of(&connections, mover_entity) else {
            continue;
        };
        let dx = to_pos.x - from_pos.x;
        let dy = to_pos.y - from_pos.y;
        let dz = to_pos.z - from_pos.z;
        let targets = play_targets_within_view_distance(
            &connections,
            Some(mover_entity),
            &to_pos,
            &entity_map,
        );
        if targets.is_empty() {
            continue;
        }
        if dx * dx + dy * dy + dz * dz > TELEPORT_DISTANCE_SQ {
            // 大位移：绝对坐标传送。
            let teleport = encode_clientbound(&EntityTeleport {
                entity_id: mover_conn as i32,
                x: to_pos.x,
                y: to_pos.y,
                z: to_pos.z,
                yaw: (to_pos.yaw.to_degrees() * 256.0 / 360.0).round() as i8,
                pitch: (to_pos.pitch.to_degrees() * 256.0 / 360.0).round() as i8,
                on_ground: true,
            });
            broadcast(&mut clients, &targets, &teleport, Priority::Normal);
        } else {
            // 小位移：定点相对移动（1/4096 方块单位）。
            let move_packet = RelEntityMove {
                entity_id: mover_conn as i32,
                d_x: (dx * 4096.0).round() as i16,
                d_y: (dy * 4096.0).round() as i16,
                d_z: (dz * 4096.0).round() as i16,
                on_ground: true,
            };
            let bytes = encode_clientbound(&move_packet);
            broadcast(&mut clients, &targets, &bytes, Priority::Normal);
        }
    }

    // ---- 通用实体生成：广播 SpawnEntity（玩家实体额外广播 PlayerInfo）----
    for event in spawn_events.read() {
        let targets = play_targets(&connections, Some(event.entity));
        if targets.is_empty() {
            continue;
        }
        // 玩家实体使用 Player 的 UUID，其余实体以 Entity 位组合成确定性 UUID。
        // 玩家实体已迁入实例 World，反查跨 World 收集的 entity_map。
        let (object_uuid, is_player) = match entity_map.get(&event.entity) {
            Some((player, _)) => (player.uuid(), true),
            None => (Uuid::from_u128(u128::from(event.entity.to_bits())), false),
        };
        let meta = meta_map.get(&event.entity);
        let object_data = object_data_from_meta(event.entity_type, meta);
        let spawn = encode_clientbound(&SpawnEntity {
            entity_id: to_i32_id(event.entity.index_u32()),
            object_uuid,
            entity_type: to_i32_id(event.entity_type.id()),
            x: event.position.x,
            y: event.position.y,
            z: event.position.z,
            velocity: [0, 0, 0],
            pitch: angle_to_byte(event.position.pitch),
            yaw: angle_to_byte(event.position.yaw),
            head_pitch: 0,
            object_data,
        });
        broadcast(&mut clients, &targets, &spawn, Priority::Urgent);
        if is_player && let Some((player, _)) = entity_map.get(&event.entity) {
            let info = encode_clientbound(&player_info_packet(player));
            broadcast(&mut clients, &targets, &info, Priority::Urgent);
        }
    }

    // ---- 通用实体移除：广播 DestroyEntities ----
    for event in remove_events.read() {
        let targets = play_targets(&connections, Some(event.entity));
        if targets.is_empty() {
            continue;
        }
        let destroy = encode_clientbound(&DestroyEntities {
            entity_ids: vec![to_i32_id(event.entity.index_u32())],
        });
        broadcast(&mut clients, &targets, &destroy, Priority::Urgent);
    }
}

/// 筛选「处于 Play 状态且已绑定实体」的连接号列表；`exclude` 指定的实体
/// 所对应连接被排除（自身不接收广播）。
fn play_targets(
    connections: &ConnectionManager,
    exclude: Option<crate::prelude::Entity>,
) -> Vec<u32> {
    connections
        .iter()
        .filter(|(_, runtime)| {
            runtime.entity.is_some()
                && runtime.entity != exclude
                && runtime.state == crate::network::connection::ConnectionState::Play
        })
        .map(|(conn_id, _)| *conn_id)
        .collect()
}

/// 筛选处于 Play 状态、与 mover 同实例 World、且区块距离不超过 mover 视距的目标连接。
///
/// 区块坐标以 `floor(pos / 16.0)` 计算，区块距离为 dx 与 dz 的绝对值均不超过视距。
/// 本函数同时实现距离裁剪：大幅减少视距外玩家的无效移动包流量。
fn play_targets_within_view_distance(
    connections: &ConnectionManager,
    exclude: Option<crate::prelude::Entity>,
    mover_pos: &Position,
    entity_map: &HashMap<crate::prelude::Entity, (Player, Position)>,
) -> Vec<u32> {
    let mover_chunk_x = mover_pos.x.floor() / 16.0;
    let mover_chunk_z = mover_pos.z.floor() / 16.0;
    // 获取 mover 所在实例 world_id 与视距；若 mover 未绑定连接则退回全量 play 目标。
    let mover_view_dist = match exclude {
        Some(e) => connection_of(connections, e)
            .and_then(|cid| connections.get(cid))
            .map(|rt| rt.view_distance as f64)
            .unwrap_or(f64::MAX),
        None => f64::MAX,
    };
    let mover_world_id = match exclude {
        Some(e) => connection_of(connections, e)
            .and_then(|cid| connections.get(cid))
            .map(|rt| rt.world_id)
            .unwrap_or(WorldId(0)),
        None => WorldId(0),
    };

    connections
        .iter()
        .filter(|(_, runtime)| {
            runtime.entity.is_some()
                && runtime.entity != exclude
                && runtime.state == crate::network::connection::ConnectionState::Play
        })
        .filter(|(_, runtime)| {
            // 同实例过滤。
            if mover_world_id != runtime.world_id {
                return false;
            }
            // 距离裁剪：反查目标实体坐标，计算区块距离是否不超过 mover 视距。
            let Some(target_entity) = runtime.entity else {
                return false;
            };
            let Some((_, target_pos)) = entity_map.get(&target_entity) else {
                return false;
            };
            let target_chunk_x = target_pos.x.floor() / 16.0;
            let target_chunk_z = target_pos.z.floor() / 16.0;
            (mover_chunk_x - target_chunk_x).abs() <= mover_view_dist
                && (mover_chunk_z - target_chunk_z).abs() <= mover_view_dist
        })
        .map(|(conn_id, _)| *conn_id)
        .collect()
}

/// u32 → i32 协议 id（防溢出回退 0，避免 `as` 缩窄）。
fn to_i32_id(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(0)
}

/// 角度（弧度）→ 协议 256 分度制有符号字节。
///
/// 先按度缩放取整，再钳制到 i8 表示范围后以饱和 `as` 转换（`f32 as i8`
/// 本就是饱和语义，钳制后必然精确）。本工具链未提供 `f32 → i32` 的
/// `TryFrom`，无法用 `TryInto`；既有 `SpawnEntity` 编码亦采用同款写法，
/// 故在显式范围保证下使用 `as`（章程「禁 `as` 缩窄」的例外，见注释）。
fn angle_to_byte(radians: f32) -> i8 {
    let scaled = radians.to_degrees() * 256.0 / 360.0;
    scaled.round().clamp(f32::from(i8::MIN), f32::from(i8::MAX)) as i8
}

/// 根据实体类型与元数据构造 `SpawnEntity` 包的 `object_data` 字段。
///
/// 不同实体类型在协议中通过 `object_data` 传递类型专属初始化值：
/// - 史莱姆/岩浆怪（Slime/MagmaCube）：体型大小（1/4/10）
/// - 青蛙（Frog）：变体（0=冷/1=温/2=热）
/// - 图画（Painting）：画作方向（0-3）
/// - 掉落物（ItemEntity）：物品 ID
/// - 经验球（ExperienceOrb）：经验值
/// - 其他未知类型默认返回 0。
///
/// `meta` 为 `Some` 时优先使用元数据表中的值，缺失则使用实体类型对应的默认值。
fn object_data_from_meta(
    entity_type: crate::resource::EntityType,
    meta: Option<&crate::component::EntityMeta>,
) -> i32 {
    let id = entity_type.id();
    match meta {
        Some(EntityMeta { data, .. }) => match id {
            // 史莱姆 / 岩浆怪：size（元数据 index 0）
            16 | 63 => data.get(0).and_then(|v| match v {
                crate::component::EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
            // 青蛙：variant（元数据 index 0）
            87 => data.get(0).and_then(|v| match v {
                crate::component::EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
            // 掉落物：item_id（元数据 index 0）
            2 => data.get(0).and_then(|v| match v {
                crate::component::EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
            // 经验球：value（元数据 index 0）
            22 => data.get(0).and_then(|v| match v {
                crate::component::EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
            // 图画：art direction（元数据 index 0）
            64 => data.get(0).and_then(|v| match v {
                crate::component::EntityMetadataValue::VarInt(v) => Some(*v),
                _ => None,
            }),
            _ => None,
        },
        None => None,
    }
    .unwrap_or(match id {
        // 史莱姆 / 岩浆怪默认体型：中等（4）
        16 | 63 => 4,
        // 青蛙默认变体：冷（0）
        87 => 0,
        // 掉落物默认物品 ID：0
        2 => 0,
        // 经验球默认经验值：1
        22 => 1,
        // 图画默认方向：0
        64 => 0,
        _ => 0,
    })
}

/// 构造 `SpawnEntity` 包（玩家实体）。
fn spawn_entity_packet(conn_id: u32, player: &Player, pos: &Position) -> SpawnEntity {
    SpawnEntity {
        entity_id: conn_id as i32,
        object_uuid: player.uuid(),
        entity_type: PLAYER_ENTITY_TYPE,
        x: pos.x,
        y: pos.y,
        z: pos.z,
        velocity: [0, 0, 0],
        pitch: (pos.pitch.to_degrees() * 256.0 / 360.0).round() as i8,
        yaw: (pos.yaw.to_degrees() * 256.0 / 360.0).round() as i8,
        head_pitch: 0,
        object_data: 0,
    }
}

/// 构造 `PlayerInfo` 包（ADD_PLAYER 动作，无属性）。
fn player_info_packet(player: &Player) -> PlayerInfo {
    PlayerInfo {
        uuid: player.uuid(),
        name: player.username().to_string(),
        properties: Vec::new(),
    }
}

/// 查询某实体对应的连接号。
///
/// 被 `entity_sync` 与 `inventory_sync` 共用（库存同步需反查 `entity → conn_id`），
/// 故提升为 `pub(crate)`。
pub(crate) fn connection_of(
    connections: &ConnectionManager,
    entity: crate::prelude::Entity,
) -> Option<u32> {
    connections
        .iter()
        .find(|(_, runtime)| runtime.entity == Some(entity))
        .map(|(conn_id, _)| *conn_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use crate::app::App;
    use uuid::Uuid;

    use super::*;
    use crate::component::InstanceRef;
    use crate::component::{EntityMeta, EntityMetadataMap, EntityMetadataValue};
    use crate::event::{EntityRemove, EntitySpawn, PlayerJoin};
    use crate::network::bridge::empty_bridge;
    use crate::network::connection::ConnectionState;
    use crate::network::listener::OutboundMessage;
    use crate::plugin::McServerPlugin;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::framing::decode_frame;
    use crate::resource::EntityType;
    use crate::test_support::{current_test_instance, ensure_test_instance, spawn_into_instance};

    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());
        let (bridge, _frame_tx, _outbound) = empty_bridge();
        app.world_mut().insert_resource(bridge);
        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(50),
            ));
        app.update();
        app
    }

    /// 注册 conn → entity 绑定与出站通道，返回出站接收端。
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

    /// 生成玩家实体（带全部组件，落入惰性单例实例 World）。
    ///
    /// 多名玩家共享同一实例 World（[`ensure_test_instance`]），避免各 World 首个
    /// spawn 的 `Entity` id 冲突；`InstanceRef` 指向玩家自身所在实例。
    fn spawn_player(app: &mut App, name: &str, x: f64, z: f64) -> crate::prelude::Entity {
        let inst = ensure_test_instance(app);
        spawn_into_instance(
            app,
            inst,
            (
                Player::new(Uuid::new_v4(), name),
                Position::new(x, 64.0, z),
                InstanceRef(inst),
            ),
        )
    }

    fn capture(rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Frame(frame) = msg {
                let mut pos = 0usize;
                if let Ok(payload) = decode_frame(&frame, &mut pos) {
                    out.push(payload);
                }
            }
        }
        out
    }

    fn payload_id(payload: &[u8]) -> i32 {
        let mut buf = ByteBuffer::new(payload.to_vec());
        buf.get_varint().unwrap_or(-1)
    }

    #[test]
    fn second_player_sees_first_player() {
        let mut app = build_app();

        // 玩家 A（conn 1）先在线。
        let entity_a = spawn_player(&mut app, "Alice", 8.0, 8.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        app.world_mut().write(PlayerJoin {
            entity: entity_a,
            username: "Alice".to_string(),
        });
        app.update();
        app.update();
        let _ = capture(&mut rx_a); // 排空 A 的初始帧

        // 玩家 B（conn 2）进入 Play。
        let entity_b = spawn_player(&mut app, "Bob", 16.0, 16.0);
        let mut rx_b = bind_client(&mut app, 2, entity_b);
        app.world_mut().write(EnterPlayEvent {
            conn_id: 2,
            entity: entity_b,
        });
        app.update();
        app.update();

        // B 应收到 A 的 SpawnEntity(0x01) + PlayerInfo(0x44)。
        let b_payloads = capture(&mut rx_b);
        let b_ids: Vec<i32> = b_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            b_ids.contains(&0x01),
            "B 应收到 A 的 SpawnEntity，实际 {b_ids:?}"
        );
        assert!(
            b_ids.contains(&0x44),
            "B 应收到 A 的 PlayerInfo，实际 {b_ids:?}"
        );

        // A 应收到 B 的 SpawnEntity + PlayerInfo。
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x01),
            "A 应收到 B 的 SpawnEntity，实际 {a_ids:?}"
        );
        assert!(
            a_ids.contains(&0x44),
            "A 应收到 B 的 PlayerInfo，实际 {a_ids:?}"
        );
    }

    #[test]
    fn player_quit_broadcasts_remove() {
        let mut app = build_app();

        let entity_a = spawn_player(&mut app, "Alice", 8.0, 8.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        app.update();
        let _ = capture(&mut rx_a);

        let entity_b = spawn_player(&mut app, "Bob", 16.0, 16.0);
        let mut rx_b = bind_client(&mut app, 2, entity_b);
        app.world_mut().write(EnterPlayEvent {
            conn_id: 2,
            entity: entity_b,
        });
        app.update();
        app.update();
        let _ = capture(&mut rx_b);
        let _ = capture(&mut rx_a);

        // B 离开 → A 应收到 PlayerRemove(0x43)。
        app.world_mut().write(PlayerQuit {
            entity: entity_b,
            username: "Bob".to_string(),
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x43),
            "A 应收到 PlayerRemove，实际 {a_ids:?}"
        );
    }

    #[test]
    fn player_move_syncs_to_others() {
        let mut app = build_app();

        let entity_a = spawn_player(&mut app, "Alice", 8.0, 8.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        let entity_b = spawn_player(&mut app, "Bob", 16.0, 16.0);
        let mut rx_b = bind_client(&mut app, 2, entity_b);
        app.world_mut().write(EnterPlayEvent {
            conn_id: 2,
            entity: entity_b,
        });
        app.update();
        app.update();
        let _ = capture(&mut rx_a);
        let _ = capture(&mut rx_b);

        // B 移动（小位移）→ A 应收到 RelEntityMove(0x33)。
        let from = Position::new(16.0, 64.0, 16.0);
        let to = Position::new(16.5, 64.0, 16.5);
        app.world_mut().write(PlayerMove {
            entity: entity_b,
            from,
            to,
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x33),
            "A 应收到 RelEntityMove，实际 {a_ids:?}"
        );

        // B 大位移（仍在 A 的视距内）→ A 应收到 EntityTeleport(0x7b)。
        // 1000 超出视距，改用 100 以保持 6 个区块内的距离。
        let far = Position::new(100.0, 64.0, 100.0);
        app.world_mut().write(PlayerMove {
            entity: entity_b,
            from: to,
            to: far,
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x7b),
            "A 应收到 EntityTeleport，实际 {a_ids:?}"
        );
    }

    #[test]
    fn entity_spawn_broadcasts_spawn_to_players() {
        let mut app = build_app();

        let entity_a = spawn_player(&mut app, "Alice", 8.0, 8.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        app.update();
        let _ = capture(&mut rx_a);

        // 非玩家实体生成 → 玩家 A 应收到 SpawnEntity(0x01)，且不广播 PlayerInfo。
        let inst = current_test_instance(&app);
        let mob = spawn_into_instance(&mut app, inst, (Position::new(10.0, 64.0, 10.0),));
        app.world_mut().write(EntitySpawn {
            entity: mob,
            entity_type: EntityType::by_id(1),
            position: Position::new(10.0, 64.0, 10.0),
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x01),
            "A 应收到非玩家实体的 SpawnEntity，实际 {a_ids:?}"
        );
        assert!(
            !a_ids.contains(&0x44),
            "非玩家实体不应广播 PlayerInfo，实际 {a_ids:?}"
        );
    }

    #[test]
    fn player_entity_spawn_broadcasts_player_info() {
        let mut app = build_app();

        let entity_a = spawn_player(&mut app, "Alice", 8.0, 8.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        app.update();
        let _ = capture(&mut rx_a);

        // 玩家实体以 EntitySpawn 事件广播 → A 应收到 SpawnEntity + PlayerInfo。
        let entity_b = spawn_player(&mut app, "Bob", 16.0, 16.0);
        app.world_mut().write(EntitySpawn {
            entity: entity_b,
            entity_type: EntityType::by_id(74),
            position: Position::new(16.0, 64.0, 16.0),
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x01),
            "A 应收到玩家实体的 SpawnEntity，实际 {a_ids:?}"
        );
        assert!(
            a_ids.contains(&0x44),
            "玩家实体应广播 PlayerInfo，实际 {a_ids:?}"
        );
    }

    #[test]
    fn entity_remove_broadcasts_destroy_to_players() {
        let mut app = build_app();

        let entity_a = spawn_player(&mut app, "Alice", 8.0, 8.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        app.update();
        let _ = capture(&mut rx_a);

        let inst = current_test_instance(&app);
        let mob = spawn_into_instance(&mut app, inst, (Position::new(10.0, 64.0, 10.0),));
        app.world_mut().write(EntityRemove { entity: mob });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x4b),
            "A 应收到 DestroyEntities，实际 {a_ids:?}"
        );
    }

    #[test]
    fn object_data_from_meta_slime_size() {
        // 史莱姆（id 16）带 size=4 的元数据 → object_data 应为 4。
        let mut data = EntityMetadataMap::new();
        data.set(0, EntityMetadataValue::VarInt(4));
        let meta = EntityMeta {
            entity_type: Some(EntityType::by_id(16)),
            data: data.clone(),
        };
        assert_eq!(object_data_from_meta(EntityType::by_id(16), Some(&meta)), 4);
        // 史莱姆（id 16）无元数据 → 默认 4。
        assert_eq!(object_data_from_meta(EntityType::by_id(16), None), 4);
        // 岩浆怪（id 63）带 size=10 → 10。
        data.set(0, EntityMetadataValue::VarInt(10));
        let meta = EntityMeta {
            entity_type: Some(EntityType::by_id(63)),
            data,
        };
        assert_eq!(
            object_data_from_meta(EntityType::by_id(63), Some(&meta)),
            10
        );
    }

    #[test]
    fn object_data_from_meta_default_zero() {
        // 普通僵尸（id 54）不带 object_data → 0。
        assert_eq!(object_data_from_meta(EntityType::by_id(54), None), 0);
        // 牛（id 1）不带 object_data → 0。
        assert_eq!(object_data_from_meta(EntityType::by_id(1), None), 0);
    }

    #[test]
    fn player_move_culled_by_view_distance() {
        let mut app = build_app();

        // 玩家 A 在 (0, 64, 0)，玩家 B 在 (200, 64, 0)（远超 10 区块视距）。
        let entity_a = spawn_player(&mut app, "Alice", 0.0, 0.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        let entity_b = spawn_player(&mut app, "Bob", 200.0, 0.0);
        let mut rx_b = bind_client(&mut app, 2, entity_b);
        // 双方进入 Play，确保互见包已清空。
        app.world_mut().write(EnterPlayEvent {
            conn_id: 2,
            entity: entity_b,
        });
        app.update();
        app.update();
        let _ = capture(&mut rx_a);
        let _ = capture(&mut rx_b);

        // B 小范围移动 → A 不应收到移动包（距离超过 view_distance=10 区块）。
        let from = Position::new(200.0, 64.0, 0.0);
        let to = Position::new(200.5, 64.0, 0.0);
        app.world_mut().write(PlayerMove {
            entity: entity_b,
            from,
            to,
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            !a_ids.contains(&0x33),
            "A 不应收到 B 的移动包（距离超限），实际 {a_ids:?}"
        );
        // B 也不应收到自己的包。
        let b_payloads = capture(&mut rx_b);
        let b_ids: Vec<i32> = b_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(!b_ids.contains(&0x33), "B 不应收到自己的移动包，实际 {b_ids:?}");
    }

    #[test]
    fn player_move_nearby_player_receives_sync() {
        let mut app = build_app();

        // 玩家 A 在 (0, 64, 0)，玩家 B 在 (5, 64, 0)（同一区块，近距离）。
        let entity_a = spawn_player(&mut app, "Alice", 0.0, 0.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        let entity_b = spawn_player(&mut app, "Bob", 5.0, 5.0);
        let mut rx_b = bind_client(&mut app, 2, entity_b);
        app.world_mut().write(EnterPlayEvent {
            conn_id: 2,
            entity: entity_b,
        });
        app.update();
        app.update();
        let _ = capture(&mut rx_a);
        let _ = capture(&mut rx_b);

        // B 小范围移动 → A 应收到 RelEntityMove(0x33)。
        let from = Position::new(5.0, 64.0, 5.0);
        let to = Position::new(5.5, 64.0, 5.5);
        app.world_mut().write(PlayerMove {
            entity: entity_b,
            from,
            to,
        });
        app.update();
        app.update();
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            a_ids.contains(&0x33),
            "近距离 A 应收到 B 的 RelEntityMove，实际 {a_ids:?}"
        );
    }

    #[test]
    fn multiple_moves_per_tick_merge_to_one_packet() {
        let mut app = build_app();

        // 两名玩家在近距离（同区块）。
        let entity_a = spawn_player(&mut app, "Alice", 0.0, 0.0);
        let mut rx_a = bind_client(&mut app, 1, entity_a);
        let entity_b = spawn_player(&mut app, "Bob", 5.0, 5.0);
        let mut rx_b = bind_client(&mut app, 2, entity_b);
        app.world_mut().write(EnterPlayEvent {
            conn_id: 2,
            entity: entity_b,
        });
        app.update();
        app.update();
        let _ = capture(&mut rx_a);
        let _ = capture(&mut rx_b);

        // 同一 tick 内写两次 PlayerMove（模拟快速输入累积）。
        app.world_mut().write(PlayerMove {
            entity: entity_b,
            from: Position::new(5.0, 64.0, 5.0),
            to: Position::new(5.5, 64.0, 5.5),
        });
        app.world_mut().write(PlayerMove {
            entity: entity_b,
            from: Position::new(5.5, 64.0, 5.5),
            to: Position::new(6.0, 64.0, 6.0),
        });
        app.update();
        app.update();

        // A 应仅收到一条 RelEntityMove（使用最后一次目标坐标）。
        let a_payloads = capture(&mut rx_a);
        let a_ids: Vec<i32> = a_payloads.iter().map(|p| payload_id(p)).collect();
        let move_count = a_ids.iter().filter(|&&id| id == 0x33).count();
        assert_eq!(
            move_count, 1,
            "同 tick 多次移动应合并为 1 条包，实际收到 {move_count} 条"
        );
    }
}
