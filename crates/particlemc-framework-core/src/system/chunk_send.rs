//! tick 管线：玩家进入 Play 后的出区块批次发送。
//!
//! 消费 [`EnterPlayEvent`]，以玩家所在区块为中心发送 3×3 区块批次：
//! `ChunkBatchStart` → 逐区块 `MapChunk` → `ChunkBatchFinished`。
//! 区块数据经 [`crate::instance::chunk_serializer`] 做调色板编码后直接入队，
//! 由 `network_send` 在 tick 末统一 flush。
//!
//! 与 `ChunkSender` 的联动：`network_receive` 在收到客户端 `ChunkBatchReceived`
//! 回包后调用 [`ChunkSender::on_batch_received`] 推进信用；本系统只负责
//! 首次出区块（进入 Play 时的一次性批次），后续区块由应用侧按需驱动。

use crate::prelude::{Entity, MessageReader, Res, ResMut};
use std::collections::HashMap;

use crate::component::{InstanceRef, Position};
use crate::event::EnterPlayEvent;
use crate::instance::ChunkStore;
use crate::instance::chunk_serializer::serialize_chunk;
use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::{
    ChunkBatchFinished, ChunkBatchStart, Heightmap, MapChunk, encode_clientbound,
};
use crate::resource::registries::BlockRegistry;
use particlemc_framework_ecs::scheduler::{InstanceScheduler, WorldId};

/// 出生区块半径（以玩家所在区块为中心，半径内发送 3×3 区块）。
const SPAWN_CHUNK_RADIUS: i32 = 1;

/// 玩家进入 Play 后发送出生区块批次。
pub fn chunk_send(
    enter_events: MessageReader<EnterPlayEvent>,
    mut clients: ResMut<ClientNetworks>,
    block_registry: Res<BlockRegistry>,
    scheduler: Res<InstanceScheduler>,
) {
    // R11.2：玩家实体已迁入实例 World，跨 World 收集其坐标与所属实例。Player
    // 进入 Play 时所在实例即 `InstanceRef` 指向的实例 World。
    let mut player_locs: HashMap<Entity, (Position, WorldId)> = HashMap::new();
    for wid in scheduler.world_ids() {
        if let Some(guard) = scheduler.lock_world(wid) {
            for (e, pos, inst) in guard
                .query::<(Entity, &Position, &InstanceRef), ()>()
                .iter()
            {
                player_locs.insert(e, (*pos, inst.0));
            }
            drop(guard);
        }
    }
    for event in enter_events.read() {
        let Some((position, instance_wid)) = player_locs.get(&event.entity) else {
            continue;
        };
        // R11：区块数据随实例 World 存放于 `ChunkStore`，经 scheduler 跨世界读取。
        let Some(guard) = scheduler.lock_world(*instance_wid) else {
            continue;
        };
        let Some(store) = guard.resource::<ChunkStore>() else {
            continue;
        };

        // 玩家所在区块坐标（Minecraft 区块 = 16×16 方格，向下取整）。
        let center_x = position.x.div_euclid(16.0) as i32;
        let center_z = position.z.div_euclid(16.0) as i32;

        // 批次开始（空包，urgent 确保顺序）。
        enqueue_packet(
            &mut clients,
            event.conn_id,
            encode_clientbound(&ChunkBatchStart),
            Priority::Urgent,
        );

        // 以玩家为中心发送 (2R+1)² 个区块；未加载的区块跳过（不 panic）。
        let mut sent = 0i32;
        for dx in -SPAWN_CHUNK_RADIUS..=SPAWN_CHUNK_RADIUS {
            for dz in -SPAWN_CHUNK_RADIUS..=SPAWN_CHUNK_RADIUS {
                let cx = center_x + dx;
                let cz = center_z + dz;
                let Some(chunk) = store.get_chunk(cx, cz) else {
                    continue;
                };
                let serialized = serialize_chunk(chunk, &block_registry);
                let map_chunk = MapChunk {
                    chunk_x: cx,
                    chunk_z: cz,
                    heightmaps: decode_heightmaps(&serialized.heightmaps),
                    chunk_data: serialized.data,
                    block_entities: Vec::new(),
                    sky_light_mask: Vec::new(),
                    block_light_mask: Vec::new(),
                    empty_sky_light_mask: Vec::new(),
                    empty_block_light_mask: Vec::new(),
                    sky_light: Vec::new(),
                    block_light: Vec::new(),
                };
                enqueue_packet(
                    &mut clients,
                    event.conn_id,
                    encode_clientbound(&map_chunk),
                    Priority::Urgent,
                );
                sent = sent.saturating_add(1);
            }
        }

        // 批次完成（batchSize = 已发送区块数）。
        enqueue_packet(
            &mut clients,
            event.conn_id,
            encode_clientbound(&ChunkBatchFinished { batch_size: sent }),
            Priority::Urgent,
        );
        drop(guard);
    }
}

/// 从序列化高度图字节解码为协议 `Heightmap` 结构。
///
/// `serialize_chunk` 输出的 `heightmaps` 为完整 wire 字节（VarInt 计数 + 每项
/// VarInt 类型 + VarInt long 数 + 大端 u64）。此处解析出类型与数据数组，
/// 组装回 [`Heightmap`] 供 `MapChunk` 编码。解析失败（数据异常）返回空列表，
/// 客户端以「无高度图」处理。
fn decode_heightmaps(bytes: &[u8]) -> Vec<Heightmap> {
    let mut heightmaps = Vec::new();
    let mut pos = 0usize;
    let Some(count) = read_varint(bytes, &mut pos) else {
        return heightmaps;
    };
    for _ in 0..count {
        let Some(map_type) = read_varint(bytes, &mut pos) else {
            break;
        };
        let Some(long_count) = read_varint(bytes, &mut pos) else {
            break;
        };
        let long_count = usize::try_from(long_count).unwrap_or(0);
        let mut data = Vec::with_capacity(long_count);
        let mut ok = true;
        for _ in 0..long_count {
            if pos + 8 > bytes.len() {
                ok = false;
                break;
            }
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes[pos..pos + 8]);
            pos += 8;
            data.push(i64::from_be_bytes(raw));
        }
        if !ok {
            break;
        }
        heightmaps.push(Heightmap { map_type, data });
    }
    heightmaps
}

/// 读取 VarInt（协议格式），返回 (值, 新游标)；失败返回 `None`。
fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<i32> {
    let mut result: i32 = 0;
    for i in 0..5 {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        result |= i32::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Some(result);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use crate::app::App;
    use uuid::Uuid;

    use super::*;
    use crate::component::Player;
    use crate::instance::{Chunk, SECTION_VOLUME};
    use crate::network::bridge::empty_bridge;
    use crate::network::client::ClientNetworks;
    use crate::network::listener::{OutboundMessage, RawFrame};
    use crate::plugin::McServerPlugin;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::framing::decode_frame;
    use crate::protocol::packet::Packet;
    use crate::protocol::packets::ChunkBatchStart;
    use crate::resource::connection_manager::ConnectionManager;
    use crate::resource::registries::BlockRegistry;
    use crate::test_support::{build_test_instance, spawn_into_instance};

    /// 构造测试 App：插件 + 空桥接 + 手动时间步进。
    fn build_app() -> (App, tokio::sync::mpsc::Sender<RawFrame>) {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());
        let (bridge, frame_tx, _outbound) = empty_bridge();
        app.world_mut().insert_resource(bridge);
        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(50),
            ));
        app.update();
        (app, frame_tx)
    }

    /// 捕获出站帧并返回 payload 列表。
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

    fn payload_id(payload: &[u8]) -> Option<i32> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        buf.get_varint().ok()
    }

    #[test]
    fn chunk_send_sends_batch_around_player() {
        let (mut app, _frame_tx) = build_app();

        // 注册一个默认实例（含 3×3 区块）并生成玩家（位于区块 0,0 中心）。
        let stone = {
            let reg = app.world().resource::<BlockRegistry>().unwrap();
            reg.0.get_id("minecraft:stone").unwrap_or(1)
        };
        let inst = build_test_instance(&mut app, |store| {
            for cx in -1..=1 {
                for cz in -1..=1 {
                    let mut chunk = Chunk::new(cx, cz, 4);
                    for section in 0..4usize {
                        for index in 0..SECTION_VOLUME {
                            chunk.set_block(section, index, stone);
                        }
                    }
                    store.load_chunk(chunk);
                }
            }
        });

        // 玩家实体（位于实例 World `inst` 内，InstanceRef 指向自身所在实例）。
        let player_entity = spawn_into_instance(
            &mut app,
            inst,
            (
                Player::new(Uuid::nil(), "Tester"),
                Position::new(8.0, 64.0, 8.0),
                InstanceRef(inst),
            ),
        );

        // conn → entity 绑定 + 出站通道
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(64);
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .clients
            .insert(1, crate::network::client::ClientNetwork::new());
        {
            let bridge = app
                .world()
                .resource::<crate::network::bridge::NetworkBridge>()
                .unwrap();
            bridge.outbound.lock().unwrap().insert(1, out_tx);
        }
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(player_entity);

        // 写入 EnterPlayEvent 并推进两帧（消息跨帧时序）。
        app.world_mut().write(EnterPlayEvent {
            conn_id: 1,
            entity: player_entity,
        });
        app.update();
        app.update();

        let payloads = capture(&mut out_rx);
        let ids: Vec<i32> = payloads.iter().filter_map(|p| payload_id(p)).collect();

        // 断言顺序：ChunkBatchStart(0x0c) → MapChunk×N → ChunkBatchFinished(0x0b)
        assert_eq!(ids[0], ChunkBatchStart.packet_id());
        assert!(ids[1..].contains(&0x2c), "应包含 MapChunk，实际 {ids:?}");
        assert_eq!(*ids.last().unwrap(), 0x0b, "末包应为 ChunkBatchFinished");
        // 3×3 全部加载 → 9 个 MapChunk
        let map_chunks = ids.iter().filter(|&&id| id == 0x2c).count();
        assert_eq!(map_chunks, 9, "应发送 9 个区块，实际 {map_chunks}");
    }

    #[test]
    fn chunk_send_empty_instance_sends_zero_batch() {
        let (mut app, _frame_tx) = build_app();

        let inst = build_test_instance(&mut app, |_store| {});
        let player_entity = spawn_into_instance(
            &mut app,
            inst,
            (
                Player::new(Uuid::nil(), "Empty"),
                Position::new(8.0, 64.0, 8.0),
                InstanceRef(inst),
            ),
        );

        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(64);
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .clients
            .insert(1, crate::network::client::ClientNetwork::new());
        {
            let bridge = app
                .world()
                .resource::<crate::network::bridge::NetworkBridge>()
                .unwrap();
            bridge.outbound.lock().unwrap().insert(1, out_tx);
        }
        app.world_mut()
            .resource_mut::<ConnectionManager>()
            .unwrap()
            .open(1, None)
            .entity = Some(player_entity);

        app.world_mut().write(EnterPlayEvent {
            conn_id: 1,
            entity: player_entity,
        });
        app.update();
        app.update();

        let payloads = capture(&mut out_rx);
        let ids: Vec<i32> = payloads.iter().filter_map(|p| payload_id(p)).collect();
        // 空实例：ChunkBatchStart → ChunkBatchFinished(0)，无 MapChunk。
        assert_eq!(ids[0], ChunkBatchStart.packet_id());
        assert_eq!(*ids.last().unwrap(), 0x0b);
        assert!(!ids.contains(&0x2c), "空实例不应发送 MapChunk");
    }
}
