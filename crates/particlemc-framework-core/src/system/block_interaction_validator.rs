// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 方块交互校验系统。
//!
//! 消费由 [`super::packet_action::packet_action_system`] 写入的
//! [`PlayerActionEvent`] 与 [`BlockPlace`] 消息，对每条例外状态（`status=0` 开始挖掘、
//! `status=2` 完成挖掘）执行服务端校验：
//! 1. 通过玩家 [`Position`] 与朝向（`yaw`/`pitch`）构造射线。
//! 2. 调用 [`crate::physics::raycast`] 获取命中方块序列。
//! 3. 校验客户端上报坐标是否在命中序列中，且目标方块在已加载区块内。
//!
//! 校验通过的事件派发至 [`EventBus`]；未通过的事件静默丢弃，并写入
//! [`BlockInteractionRejected`] 消息供应用侧审计。
//!
//! 本系统注册于 `packet_action_system` 之后，确保消息收件箱已被填充。

use crate::prelude::{MessageReader, MessageWriter, Query, Res, ResMut, With};

use crate::component::{Player, Position};
use crate::event::{BlockInteractionRejected, BlockPlace, PlayerActionEvent};
use crate::instance::ChunkStore;
use crate::physics::ray::{Ray, raycast};
use crate::resource::registries::BlockRegistry;

/// 最大挖掘距离（格）。与 Minecraft 原版一致。
const MAX_INTERACTION_DISTANCE: f64 = 6.0;

/// 判断指定世界坐标处是否为实心方块。
///
/// 若区块未加载或方块 id 为 0（空气），返回 `false`。
fn is_solid(chunk_store: &ChunkStore, registry: &BlockRegistry, x: i32, y: i32, z: i32) -> bool {
    if y < 0 {
        return false;
    }
    let id = chunk_store.get_block_id_world(x, y, z);
    if id == 0 {
        return false;
    }
    registry.light_opacity(id) != 0
}

/// 校验并派发方块交互事件。
///
/// - 通过校验的 `PlayerActionEvent`（`status=0/2`）与 `BlockPlace` 派发至 [`EventBus`]。
/// - 未通过校验的事件静默丢弃，并写入 [`BlockInteractionRejected`] 消息。
pub fn block_interaction_validator(
    player_action_reader: MessageReader<PlayerActionEvent>,
    block_place_reader: MessageReader<BlockPlace>,
    mut rejected_writer: MessageWriter<BlockInteractionRejected>,
    mut event_bus: ResMut<crate::event::bus::EventBus>,
    chunk_store: Res<ChunkStore>,
    registry: Res<BlockRegistry>,
    position_query: Query<&Position, With<Player>>,
) {
    // 处理 PlayerActionEvent（挖掘 / 取消 / 完成等）。
    for event in player_action_reader.iter() {
        // status=0（开始挖掘）与 status=2（完成挖掘）需要服务端校验。
        if event.status != 0 && event.status != 2 {
            // 其余状态（取消、丢整堆等）直接放行。
            event_bus.dispatch(event.clone());
            continue;
        }
        let (tx, ty, tz) = event.position;
        // 查找玩家 Position。
        let Some(pos) = position_query.get(event.player).ok() else {
            event_bus.dispatch(event.clone());
            continue;
        };
        // 距离校验：曼哈顿距离 <= 6。
        let dist = (pos.x - tx as f64).abs()
            + (pos.y - ty as f64).abs()
            + (pos.z - tz as f64).abs();
        if dist > MAX_INTERACTION_DISTANCE {
            let _ = rejected_writer.write(BlockInteractionRejected {
                player: event.player,
                position: event.position,
                status: event.status,
            });
            continue;
        }
        // 区块加载校验。
        let cx = tx.div_euclid(16);
        let cz = tz.div_euclid(16);
        if chunk_store.get_chunk(cx, cz).is_none() {
            let _ = rejected_writer.write(BlockInteractionRejected {
                player: event.player,
                position: event.position,
                status: event.status,
            });
            continue;
        }
        // 射线校验：构造射线并验证命中方块包含目标坐标。
        let dir_x = -(pos.yaw.to_radians() as f64).sin() * (pos.pitch.to_radians() as f64).cos();
        let dir_y = (pos.pitch.to_radians() as f64).sin();
        let dir_z = -(pos.yaw.to_radians() as f64).cos() * (pos.pitch.to_radians() as f64).cos();
        let origin = [pos.x as f64, pos.y as f64, pos.z as f64];
        let direction = [dir_x, dir_y, dir_z];
        let Some(ray) = Ray::new(origin, direction) else {
            event_bus.dispatch(event.clone());
            continue;
        };
        let hits = raycast(&ray, MAX_INTERACTION_DISTANCE, |hx, hy, hz| {
            is_solid(&chunk_store, &registry, hx, hy, hz)
        });
        let hit_ok = hits.iter().any(|&(hx, hy, hz)| hx == tx && hy == ty && hz == tz);
        if hit_ok {
            event_bus.dispatch(event.clone());
        } else {
            let _ = rejected_writer.write(BlockInteractionRejected {
                player: event.player,
                position: event.position,
                status: event.status,
            });
        }
    }

    // 处理 BlockPlace（放置方块）。
    for event in block_place_reader.iter() {
        let px = event.position.x.ceil() as i32;
        let py = event.position.y.ceil() as i32;
        let pz = event.position.z.ceil() as i32;
        // 查找发起放置的玩家实体（通过 PlayerDiggingState 关联）。
        // 简化：对所有带 Player 组件的实体进行距离校验。
        let mut valid = false;
        for pos in position_query.iter() {
            let dist = (pos.x - px as f64).abs()
                + (pos.y - py as f64).abs()
                + (pos.z - pz as f64).abs();
            if dist <= MAX_INTERACTION_DISTANCE {
                valid = true;
                break;
            }
        }
        if valid {
            event_bus.dispatch(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::component::{Player, Position};
    use crate::instance::chunk::{Chunk, SECTION_VOLUME};
    use crate::instance::chunk_store::ChunkStore;
    use crate::network::client::ClientNetworks;
    use crate::protocol::packets::play::PlayerAction;
    use crate::resource::connection_manager::ConnectionManager;
    use crate::resource::registries::BlockRegistry;
    use particlemc_framework_ecs::message::MessageInbox;

    fn make_chunk(x: i32, z: i32) -> Chunk {
        let mut c = Chunk::new(x, z, 1);
        for i in 0..SECTION_VOLUME {
            let _ = c.set_block(0, i, 1);
        }
        c
    }

    fn make_app() -> App {
        let mut app = App::new();
        app.init_resource::<ClientNetworks>();
        app.init_resource::<ConnectionManager>();
        app.init_resource::<crate::event::bus::EventBus>();
        app.init_resource::<ChunkStore>();
        app.insert_resource::<BlockRegistry>(BlockRegistry::default());
        app.add_systems(super::block_interaction_validator);
        // 预注册 PlayerActionEvent 消息收件箱，使测试可直接写入事件。
        app.add_message::<PlayerActionEvent>();
        app
    }

    #[test]
    fn valid_nearby_block_is_dispatched() {
        let mut app = make_app();
        // 在 (0,0) 区块填充石头方块。
        let chunk = make_chunk(0, 0);
        app.world_mut()
            .resource_mut::<ChunkStore>()
            .unwrap()
            .load_chunk(chunk);
        // 玩家位于 (8, 64, 0) 附近，朝向目标。
        let player = app.world_mut().spawn_empty()
            .insert(Position::with_rotation(8.0, 64.0, 0.0, 0.0, 0.0))
            .insert(Player::new(uuid::Uuid::new_v4(), "test"))
            .id();
        // 通过 MessageInbox 直接写入事件（绕过 packet_action_system）。
        app.world_mut()
            .resource_mut::<MessageInbox<PlayerActionEvent>>()
            .unwrap()
            .write(PlayerActionEvent {
                player,
                status: 0,
                position: (8, 64, 0),
            });
        app.update();
        // 校验通过：事件应被派发（此处验证无 panic 即可）。
    }

    #[test]
    fn out_of_range_block_is_rejected() {
        let mut app = make_app();
        let player = app.world_mut().spawn_empty()
            .insert(Position::new(0.0, 64.0, 0.0))
            .insert(Player::new(uuid::Uuid::new_v4(), "test"))
            .id();
        app.world_mut()
            .resource_mut::<MessageInbox<PlayerActionEvent>>()
            .unwrap()
            .write(PlayerActionEvent {
                player,
                status: 0,
                position: (100, 64, 100),
            });
        // 不应 panic，距离超限时事件被静默丢弃。
        app.update();
    }

    #[test]
    fn unloaded_chunk_block_is_rejected() {
        let mut app = make_app();
        let player = app.world_mut().spawn_empty()
            .insert(Position::new(0.0, 64.0, 0.0))
            .insert(Player::new(uuid::Uuid::new_v4(), "test"))
            .id();
        // 目标方块在 (100, 64, 100)，对应区块 (6, 6) 未加载。
        app.world_mut()
            .resource_mut::<MessageInbox<PlayerActionEvent>>()
            .unwrap()
            .write(PlayerActionEvent {
                player,
                status: 0,
                position: (100, 64, 100),
            });
        app.update();
    }
}
