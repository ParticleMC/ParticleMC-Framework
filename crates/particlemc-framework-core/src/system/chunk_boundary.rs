// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 区块边界检测系统（G3-T2）。
//!
//! 每 tick 遍历所有 Play 状态玩家，收集其视距边界内的区块坐标集合，
//! 与 [`ChunkStore`](crate::instance::ChunkStore) 中已加载区块对比，
//! 将未加载但应在边界内的区块投递至 [`ChunkLoadQueue`]。

use std::collections::HashSet;

use crate::prelude::{MessageReader, Res, ResMut};

use crate::component::{Player, Position};
use crate::event::EnterPlayEvent;
use crate::instance::ChunkStore;
use crate::resource::chunk_load_request::ChunkLoadQueue;
use particlemc_framework_ecs::scheduler::InstanceScheduler;

/// 默认视距（区块数）。
const DEFAULT_VIEW_DISTANCE: i32 = 8;

/// 区块边界检测系统。
///
/// 遍历各实例 World，对每个挂载 `Player` + `Position` 的实体，
/// 以其区块坐标为中心、视距为半径收集边界区块；
/// 未加载的区块投递至 `ChunkLoadQueue`。
pub fn chunk_boundary(
    _enter_events: MessageReader<EnterPlayEvent>,
    scheduler: Res<InstanceScheduler>,
    chunk_load_queue: ResMut<ChunkLoadQueue>,
) {
    for wid in scheduler.world_ids() {
        let Some(mut guard) = scheduler.lock_world(wid) else {
            continue;
        };
        let world = guard.world();
        let Some(chunk_store) = world.resource::<ChunkStore>() else {
            continue;
        };

        // 收集该实例中所有玩家的区块坐标。
        let mut player_chunks: HashSet<(i32, i32)> = HashSet::new();
        for (pos, _player) in world
            .query::<(&Position, &Player), ()>()
            .iter()
        {
            let cx = pos.x.floor() as i32 / 16;
            let cz = pos.z.floor() as i32 / 16;
            for dy in -DEFAULT_VIEW_DISTANCE..=DEFAULT_VIEW_DISTANCE {
                for dz in -DEFAULT_VIEW_DISTANCE..=DEFAULT_VIEW_DISTANCE {
                    player_chunks.insert((cx + dy, cz + dz));
                }
            }
        }

        // 将未加载但应在边界内的区块投递到队列。
        for &(cx, cz) in &player_chunks {
            if chunk_store.get_chunk(cx, cz).is_none() {
                chunk_load_queue.push(crate::resource::chunk_load_request::ChunkLoadRequest {
                    world_id: wid.0,
                    chunk_x: cx,
                    chunk_z: cz,
                });
            }
        }
    }
}
