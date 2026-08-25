// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 世界快照（R7）：区块 / 实体 / 实例的序列化载体。
//!
//! 语义对齐框架的 `snapshot/` 包：`Snapshotable` 对象在安全点（如
//! tick 结束）被保存为不可变快照，供服务器存档、传输与「一次性读取」场景
//! 使用。本模块提供三组承载结构：
//!
//! - [`ChunkSnapshot`]：区块的方块 id 平铺快照（`to_chunk` 可完整重建）；
//! - [`EntitySnapshot`]：实体身份 + 位置 + 组件数据（NBT 承载）；
//! - [`InstanceSnapshot`]：区块集合 + 实体集合的整体快照。
//!
//! **v1 限制**：[`EntitySnapshot`] 仅记录 position，`components` 字段为预留
//! 槽位（恒为空）；真实组件 NBT 序列化（生命/物品等）由后续批次补齐。
//! [`InstanceSnapshot::from_instance`] 只捕获区块数据，不捕获生成器/持久化器
//! 装配信息（重建后的容器为裸容器，无 generator/loader）。
//!
//! 变更标识符：`complete-missing-subsystems`（T9/R7）。

use crate::prelude::{Entity, World};

use crate::component::Position;
use crate::instance::chunk::{Chunk, SECTION_VOLUME};
use crate::instance::chunk_store::ChunkStore;
use crate::protocol::nbt::NbtTag;

/// 区块快照：坐标 + 全部区段的方块 id 平铺。
///
/// `blocks` 按「区段顺序 × 区段内线性索引」平铺（区段内索引 `(y<<8)|(z<<4)|x`），
/// 长度恒为 `区段数 × 4096`。由此 `to_chunk` 可无损重建区块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSnapshot {
    /// 区块 X 坐标。
    pub x: i32,
    /// 区块 Z 坐标。
    pub z: i32,
    /// 全部区段的方块状态 id（平铺）。
    pub blocks: Vec<u32>,
}

impl ChunkSnapshot {
    /// 从区块捕获快照（克隆全部方块 id）。
    pub fn from_chunk(chunk: &Chunk) -> ChunkSnapshot {
        let mut blocks = Vec::with_capacity(chunk.sections.len() * SECTION_VOLUME);
        for section in &chunk.sections {
            for index in 0..SECTION_VOLUME {
                blocks.push(section.get_block_id(index));
            }
        }
        ChunkSnapshot {
            x: chunk.x,
            z: chunk.z,
            blocks,
        }
    }

    /// 重建区块（区段数由 `blocks` 长度推导，至少 1 个区段）。
    ///
    /// 若 `blocks` 长度不是 4096 的整数倍，多出的不足一区段部分被忽略
    /// （该部分方块的原始写入按越界区段失败处理）。
    pub fn to_chunk(&self) -> Chunk {
        let section_count = self.blocks.len().div_ceil(SECTION_VOLUME).max(1);
        let mut chunk = Chunk::new(self.x, self.z, section_count);
        for (index, &id) in self.blocks.iter().enumerate() {
            let section = index / SECTION_VOLUME;
            let local = index % SECTION_VOLUME;
            let _ = chunk.set_block(section, local, id);
        }
        chunk
    }

    /// 读取某坐标的方块 id（区块局部坐标，越界返回 0）。
    pub fn get_block(&self, x: u8, y: u8, z: u8) -> u32 {
        let section = usize::from(y) / 16;
        let local_y = usize::from(y) % 16;
        let index =
            (section * SECTION_VOLUME) + (local_y << 8) + (usize::from(z) << 4) + usize::from(x);
        self.blocks.get(index).copied().unwrap_or(0)
    }
}

/// 实体快照：实体身份 + 位置 + 组件数据。
///
/// `entity` 保存 旧 ECS 方案 实体句柄；`position` 为 `[x, y, z]`；`components` 以
/// `(名称, NBT)` 承载组件数据。
///
/// **v1 限制**：仅捕获 position（读 `Position` 组件），`components` 恒为空；
/// 其余组件（生命/物品等）的 NBT 序列化由后续批次补齐。
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySnapshot {
    /// 被快照的实体句柄。
    pub entity: Entity,
    /// 位置（`[x, y, z]`，取自 `Position` 组件）。
    pub position: [f64; 3],
    /// 组件数据（`(组件名, NBT)`），v1 恒为空。
    pub components: Vec<(String, NbtTag)>,
}

impl EntitySnapshot {
    /// 从 World 中捕获指定实体的快照。
    ///
    /// 实体不存在或缺少 [`Position`] 组件时返回 `None`。
    pub fn from_entity(world: &World, entity: Entity) -> Option<EntitySnapshot> {
        let position = world.get::<Position>(entity)?;
        Some(EntitySnapshot {
            entity,
            position: [position.x(), position.y(), position.z()],
            components: Vec::new(),
        })
    }
}

/// 实例快照：区块集合 + 实体集合。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstanceSnapshot {
    /// 全部区块快照。
    pub chunks: Vec<ChunkSnapshot>,
    /// 全部实体快照。
    pub entities: Vec<EntitySnapshot>,
}

impl InstanceSnapshot {
    /// 构造空快照。
    pub fn new() -> Self {
        Self::default()
    }

    /// 捕获实例的全部区块（克隆方块数据）。
    ///
    /// 实体部分不参与捕获（[`InstanceContainer`] 不持有实体），`entities` 为空，
    /// 需结合外部 World 经 [`EntitySnapshot::from_entity`] 填充。
    pub fn from_chunk_store(store: &ChunkStore) -> InstanceSnapshot {
        let chunks = store.iter_chunks().map(ChunkSnapshot::from_chunk).collect();
        InstanceSnapshot {
            chunks,
            entities: Vec::new(),
        }
    }

    /// 由快照重建实例容器（裸容器：无生成器/持久化器装配）。
    pub fn to_chunk_store(&self, store: &mut ChunkStore) {
        for chunk in &self.chunks {
            store.load_chunk(chunk.to_chunk());
        }
    }

    /// 区块快照数量。
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// 实体快照数量。
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }
}

/// 快照能力 trait：可被保存为不可变快照、并可从快照加载还原。
pub trait Snapshotable {
    /// 创建当前状态的不变快照。
    fn create_snapshot(&self) -> InstanceSnapshot;
    /// 以快照整体还原（替换当前状态）。
    fn load_snapshot(&mut self, snapshot: &InstanceSnapshot);
}

impl Snapshotable for ChunkStore {
    fn create_snapshot(&self) -> InstanceSnapshot {
        InstanceSnapshot::from_chunk_store(self)
    }

    fn load_snapshot(&mut self, snapshot: &InstanceSnapshot) {
        snapshot.to_chunk_store(self);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::instance::chunk::SECTION_VOLUME;

    /// 构造含一个测试区块的 `ChunkStore`（替代旧 `InstanceContainer` 范式）。
    fn test_container() -> ChunkStore {
        let mut store = ChunkStore::new();
        let mut chunk = Chunk::new(3, -2, 2);
        // 区段 0：石头 + 泥土；区段 1：石头。
        assert!(chunk.set_block(0, 5, 1));
        assert!(chunk.set_block(0, 100, 2));
        assert!(chunk.set_block(1, 500, 1));
        store.load_chunk(chunk);
        store
    }

    #[test]
    fn chunk_snapshot_roundtrip_preserves_all_blocks() {
        let container = test_container();
        let chunk = container.get_chunk(3, -2).unwrap();
        let snapshot = ChunkSnapshot::from_chunk(chunk);
        assert_eq!(snapshot.x, 3);
        assert_eq!(snapshot.z, -2);
        assert_eq!(snapshot.blocks.len(), 2 * SECTION_VOLUME);

        let restored = snapshot.to_chunk();
        assert_eq!(restored.section_count(), 2);
        assert_eq!(restored.get_block(0, 5), 1);
        assert_eq!(restored.get_block(0, 100), 2);
        assert_eq!(restored.get_block(1, 500), 1);
        // 空气位置保持空气。
        assert_eq!(restored.get_block(1, 499), 0);
    }

    #[test]
    fn chunk_snapshot_get_block_reads_by_local_coord() {
        let container = test_container();
        let snapshot = ChunkSnapshot::from_chunk(container.get_chunk(3, -2).unwrap());
        // 区段 0 局部 y=5、线性索引 5 → (x=5,y=0,z=0)。
        assert_eq!(snapshot.get_block(5, 0, 0), 1);
        assert_eq!(snapshot.get_block(4, 0, 0), 0);
        // 越界返回 0。
        assert_eq!(snapshot.get_block(16, 0, 0), 0);
    }

    #[test]
    fn instance_snapshot_copies_all_chunks() {
        let store = test_container();
        let snapshot = InstanceSnapshot::from_chunk_store(&store);
        assert_eq!(snapshot.chunk_count(), 1);
        // 方块数据一致。
        let chunk_snapshot = snapshot.chunks.first().unwrap();
        assert_eq!(chunk_snapshot.blocks.get(5).copied().unwrap_or(0), 1);
        assert_eq!(chunk_snapshot.blocks.get(100).copied().unwrap_or(0), 2);
        assert_eq!(
            chunk_snapshot
                .blocks
                .get(SECTION_VOLUME + 500)
                .copied()
                .unwrap_or(0),
            1
        );
    }

    #[test]
    fn to_instance_rebuilds_container_with_same_blocks() {
        let store = test_container();
        let snapshot = InstanceSnapshot::from_chunk_store(&store);
        let mut rebuilt = ChunkStore::new();
        snapshot.to_chunk_store(&mut rebuilt);
        assert_eq!(rebuilt.chunk_count(), 1);
        let chunk = rebuilt.get_chunk(3, -2).unwrap();
        assert_eq!(chunk.get_block(0, 5), 1);
        assert_eq!(chunk.get_block(0, 100), 2);
        assert_eq!(chunk.get_block(1, 500), 1);
    }

    #[test]
    fn entity_snapshot_captures_position() {
        let mut world = World::new();
        let entity = world.spawn_bundle(Position::new(1.5, 64.0, -3.25)).id();
        let snapshot = EntitySnapshot::from_entity(&world, entity).unwrap();
        assert_eq!(snapshot.entity, entity);
        assert_eq!(snapshot.position, [1.5, 64.0, -3.25]);
        // 组件槽位 v1 恒为空。
        assert!(snapshot.components.is_empty());
        // 缺少 Position 的实体返回 None。
        let bare = world.spawn_empty().id();
        assert!(EntitySnapshot::from_entity(&world, bare).is_none());
    }

    #[test]
    fn snapshotable_trait_roundtrip() {
        let mut store = test_container();
        // trait 方法经 trait 对象调用（语义与固有方法一致）。
        let snapshot = Snapshotable::create_snapshot(&store);
        assert_eq!(snapshot.chunk_count(), 1);

        // 修改原存储后加载快照还原。
        assert!(store.set_block(3, -2, 0, 5, 42));
        Snapshotable::load_snapshot(&mut store, &snapshot);
        assert_eq!(store.get_chunk(3, -2).unwrap().get_block(0, 5), 1);
        assert_eq!(store.get_chunk(3, -2).unwrap().get_block(1, 500), 1);
        assert_eq!(store.chunk_count(), 1);
    }

    #[test]
    fn empty_snapshot_to_instance_is_empty_container() {
        let snapshot = InstanceSnapshot::new();
        let mut store = ChunkStore::new();
        snapshot.to_chunk_store(&mut store);
        assert_eq!(store.chunk_count(), 0);
    }

    /// 编译期断言：`Section` 快照重建路径不依赖调色板内部布局。
    #[test]
    fn section_content_equality_after_rebuild() {
        let container = test_container();
        let original = container.get_chunk(3, -2).unwrap();
        let restored = ChunkSnapshot::from_chunk(original).to_chunk();
        // 逐区段内容相等（Section 的 PartialEq 按内容比较）。
        for (a, b) in original.sections.iter().zip(&restored.sections) {
            assert_eq!(a, b);
        }
    }
}
