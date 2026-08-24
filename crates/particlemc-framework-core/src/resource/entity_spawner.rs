// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 实体生成器：批量创建 / 销毁世界实体的统一入口。
//!
//! [`EntitySpawner`] 以 旧 ECS 方案 `Resource` 形式提供 `spawn_entity` / `despawn_entity`，
//! 负责为生成实体装配基础组件（坐标 / 速度 / 实体元数据 / 实例引用 / 生命值）。
//! 玩家实体不走此路径（玩家登录另有专用逻辑），生成结果不携带 `Player` 组件。

use crate::prelude::{Commands, Entity, World};

use crate::component::{EntityMeta, Health, InstanceRef, Position, Velocity};
use crate::resource::EntityType;
use particlemc_framework_ecs::scheduler::WorldId;

/// 实体生成器（`Resource`）。
///
/// 本身无内部状态，作为系统间共享的实体创建 / 销毁入口存在。
#[derive(Default)]
pub struct EntitySpawner;

impl EntitySpawner {
    /// 在指定实例 World 中生成一个非玩家实体，返回其实体句柄。
    ///
    /// 调用方应经 `scheduler.lock_world(wid).world()` 取得目标实例 World 的
    /// `&mut World` 传入，使实体落于该实例 World。生成的实体携带以下组件：
    /// `Position`（拷贝）、`Velocity::zero()`、`EntityMeta::new(entity_type)`、
    /// `InstanceRef(instance)` 与 `Health::new(20.0, 20.0)`。**不添加 `Player`
    /// 组件**——玩家实体由登录路径另行创建（亦跨 World 落入实例 World）。
    pub fn spawn_entity(
        &self,
        world: &mut World,
        entity_type: EntityType,
        position: Position,
        instance: WorldId,
    ) -> Entity {
        world
            .spawn_bundle((
                position,
                Velocity::zero(),
                EntityMeta::new(entity_type),
                InstanceRef(instance),
                Health::new(20.0, 20.0),
            ))
            .id()
    }

    /// 销毁一个实体（经主 World 命令缓冲；跨 World 销毁请直接 `World::despawn`）。
    pub fn despawn_entity(&self, commands: &mut Commands, entity: Entity) {
        commands.despawn(entity);
    }
}
