// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 区块与区段结构（世界模型骨架）。
//!
//! [`Section`] 是 16×16×16 的方块容器，内部以「调色板 + 位压缩」存储：
//! 方块 id 先映射到调色板下标，再按每 `u64` 从低位连续打包
//! `floor(64 / bits)` 个条目（与 [`crate::instance::chunk_serializer`] 的
//! 打包规则一致）。空气（id 0）不进入调色板，而以「越界哨兵」下标存储，
//! 使 16 种方块仍可压缩在 4 bit 内；全空区段退化为单值调色板
//! （`bits = 0`），内部存储极小。[`Chunk`] 由若干 `Section` 垂直堆叠而成，
//! 并跟踪每个区段的脏标记，供后续按区段做增量同步。

use std::collections::HashMap;

use super::light_engine::{LightBoundaryDir, SectionLightBoundary};
use crate::prelude::Component;

use crate::component::Block;
use crate::resource::registries::BlockRegistry;

/// 单个区段包含的方块数量（16×16×16）。
pub const SECTION_VOLUME: usize = 16 * 16 * 16;

/// 间接调色板允许的最小位宽。
const PALETTE_MIN_BITS: u32 = 4;
/// 间接调色板允许的最大位宽（超出后跳转到直接位宽）。
const PALETTE_MAX_BITS: u32 = 8;
/// 直接位宽（容量覆盖全部可表达的下标）。
const PALETTE_DIRECT_BITS: u32 = 15;

/// 批量填充时返回的错误类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionFillError {
    /// ids 切片长度不等于 SECTION_VOLUME（4096），或区段数量与区块配置不一致。
    InvalidLength,
    /// 区段索引越界（section >= sections.len()）。
    SectionOutOfBounds,
    /// 目标区块未加载（仅保留为枚举变体，当前规格无新增接口使用；保留用于未来扩展）。
    ChunkNotFound,
}

/// 区段：一段 16×16×16 的方块 ID，内部以调色板 + 位压缩存储。
///
/// 空气区段退化为单值调色板：`palette_ids` 仅含一个元素、`bits = 0`、
/// `data` 为空，内存开销极小。读写均为 O(1) 平均（`HashMap` 定位下标 +
/// 单次位运算），扩容（新方块 id 加入调色板）时 O(4096) 重打包。
///
/// 调色板只登记非空气方块；空气以「越界哨兵下标」（等于调色板条目数）
/// 写入数据，读取时越界即还原为 0。
#[derive(Component, Debug, Clone)]
#[component(storage = "sparse")]
pub struct Section {
    /// 方块 id → 调色板下标（加速 id → 下标查找）。
    palette: HashMap<u32, usize>,
    /// 调色板下标 → 方块 id（读取时还原）。
    palette_ids: Vec<u32>,
    /// 报告位宽：0 表示单值模式；4..=8 为间接模式；15 为直接模式。
    ///
    /// 该值仅反映「非空气方块种类数」所需位宽；实际数据打包还需容纳空气
    /// 越界哨兵（见 [`Self::storage_bits`]）。
    bits: u32,
    /// 打包的条目数组（每 `u64` 从低位连续放 `floor(64 / bits)` 个条目）。
    data: Vec<u64>,
}

impl Default for Section {
    fn default() -> Self {
        Self::new()
    }
}

/// 区段相等判定：比较 4096 个格子的方块 id 内容，而非内部存储布局，
/// 保证「相同方块内容 ↔ 相等」的语义一致。
impl PartialEq for Section {
    fn eq(&self, other: &Self) -> bool {
        (0..SECTION_VOLUME).all(|index| self.get_block_id(index) == other.get_block_id(index))
    }
}

impl Eq for Section {}

impl Section {
    /// 构造一个全为空气（0）的区段，内部退化为单值调色板。
    pub fn new() -> Self {
        Self {
            palette: HashMap::from([(0u32, 0usize)]),
            palette_ids: vec![0u32],
            bits: 0,
            data: Vec::new(),
        }
    }

    /// 读取某索引处的方块 ID；越界或空气均返回 0。
    ///
    /// 索引范围为 `[0, 4096)`，越界返回 0 而不 panic。
    pub fn get_block_id(&self, index: usize) -> u32 {
        if index >= SECTION_VOLUME {
            return 0;
        }
        if self.bits == 0 {
            // 单值模式：整段仅一种方块。
            return self.palette_ids.first().copied().unwrap_or(0);
        }
        let palette_index = self.read_index(index);
        self.palette_ids.get(palette_index).copied().unwrap_or(0)
    }

    /// 写入某索引处的方块 ID。
    ///
    /// 返回是否成功写入（索引越界时返回 `false`）。
    pub fn set_block_id(&mut self, index: usize, id: u32) -> bool {
        if index >= SECTION_VOLUME {
            return false;
        }
        // 空气（id 0）不进调色板，写入「越界哨兵」下标，读取时还原为 0。
        if id == 0 {
            self.write_index(index, self.air_index());
            return true;
        }
        if let Some(&palette_index) = self.palette.get(&id) {
            self.write_index(index, palette_index);
            return true;
        }
        // 首次写入非空气方块：把空气哨兵替换为实际方块，调色板转为
        // 不含空气的紧凑布局，使 16 种方块仍可压缩在 4 bit 内。
        if self.bits == 0
            && self.palette_ids.len() == 1
            && self.palette_ids.first().copied() == Some(0)
        {
            self.palette.remove(&0);
            self.palette.insert(id, 0);
            if let Some(slot) = self.palette_ids.get_mut(0) {
                *slot = id;
            }
            self.bits = PALETTE_MIN_BITS;
            self.repack(true, 0);
            self.write_index(index, 0);
            return true;
        }
        // 新方块：追加到调色板末尾，必要时提升位宽并重打包数据数组。
        let old_storage = self.storage_bits();
        let palette_index = self.palette_ids.len();
        self.palette.insert(id, palette_index);
        self.palette_ids.push(id);
        self.bits = Self::bits_for_palette_len(self.palette_ids.len());
        // 调色板扩容后空气越界哨兵 +1，必须重打包以保持旧空气格一致。
        self.repack(false, old_storage);
        self.write_index(index, palette_index);
        true
    }

    /// 区段容量（恒为 4096）。
    pub fn len(&self) -> usize {
        SECTION_VOLUME
    }

    /// 区段是否未初始化（正常构造后恒为 `false`）。
    pub fn is_empty(&self) -> bool {
        false
    }

    /// 整区段批量写入：接受长度为 `SECTION_VOLUME` 的方块 id 切片，
    /// 以单次调色板构建 + 单次数据写入完成填充，不触发逐格 repack。
    ///
    /// 先验校验长度，长度不符立即返回 `Err(InvalidLength)` 且不修改区段状态。
    /// 位宽超 8 bits（>256 种非空气方块）时自动切换 direct 模式（bits=15）。
    ///
    /// # 不变性
    /// 失败时区段内部状态与调用前完全一致（先验校验，无中间可变操作）。
    pub fn fill_blocks(&mut self, ids: &[u32]) -> Result<(), SectionFillError> {
        if ids.len() != SECTION_VOLUME {
            return Err(SectionFillError::InvalidLength);
        }
        // 单次遍历收集所有唯一非空气 id，构建完整调色板。
        // 使用 Vec 而非固定数组以支持 >256 种方块的 direct 模式。
        let mut unique_ids: Vec<u32> = Vec::new();
        for &id in ids {
            if id == 0 {
                continue;
            }
            if !unique_ids.contains(&id) {
                unique_ids.push(id);
            }
        }
        // 全空气特例：保持单值模式。
        if unique_ids.is_empty() {
            self.bits = 0;
            self.palette.clear();
            self.palette_ids.clear();
            self.palette_ids.push(0);
            self.data.clear();
            return Ok(());
        }
        // 重新构建调色板（id → 下标）。
        self.palette.clear();
        self.palette_ids.clear();
        for &id in &unique_ids {
            let idx = self.palette_ids.len();
            self.palette.insert(id, idx);
            self.palette_ids.push(id);
        }
        // 计算存储位宽：若条目数 > 255 则使用 direct 模式（bits=15）。
        let new_air = self.palette_ids.len();
        let storage_bits = if new_air > 255 {
            PALETTE_DIRECT_BITS
        } else {
            Self::bits_for_palette_len(new_air)
        };
        let storage_bits_usize = usize::try_from(storage_bits).unwrap_or(usize::MAX);
        let entries_per_long = 64 / storage_bits_usize.max(1);
        let long_count = SECTION_VOLUME.div_ceil(entries_per_long);
        // 预分配 data，单次线性遍历 ids 写入。
        self.data = vec![0u64; long_count];
        for (i, &id) in ids.iter().enumerate() {
            // direct 模式下空气哨兵为 0（index 0 → id 0），非 direct 用 air_index。
            let palette_index = if id == 0 {
                if storage_bits == PALETTE_DIRECT_BITS { 0 } else { new_air }
            } else {
                *self.palette.get(&id).unwrap()
            };
            let long_index = i / entries_per_long;
            let bit_index = (i % entries_per_long) * storage_bits_usize;
            if let Some(long) = self.data.get_mut(long_index) {
                let mask = if storage_bits >= 64 {
                    u64::MAX
                } else {
                    (1u64 << storage_bits) - 1
                };
                let value = u64::try_from(palette_index).unwrap_or(0) & mask;
                *long = (*long & !(mask << bit_index)) | (value << bit_index);
            }
        }
        self.bits = storage_bits;
        Ok(())
    }

    /// 给定非空气方块种类数，返回报告位宽。
    ///
    /// 位宽策略：1..=16 种 → 4；随后按需 5→6→…→8；超过 256 种
    /// （9 位以上）→ 直接跳至 15 位。与 [`crate::instance::chunk_serializer`]
    /// 的「2..=16 种方块 → 4 bit」策略一致：调色板条目（含空气越界哨兵）
    /// 达 16 时仍保持 4 bit，第 17 条才升位。
    ///
    /// `len` 为非空气方块种数（= 调色板条目数）；空气以「越界哨兵下标
    /// `len`」写入数据，故报告位宽须能容纳哨兵值 `len` 本身。
    fn bits_for_palette_len(len: usize) -> u32 {
        let needed = Self::bits_to_represent(len);
        let clamped = needed.max(PALETTE_MIN_BITS);
        if clamped > PALETTE_MAX_BITS {
            PALETTE_DIRECT_BITS
        } else {
            clamped
        }
    }

    /// 表示 `n` 所需位数（`n = 0` 时返回 0），等价于 `ceil(log2(n + 1))`。
    fn bits_to_represent(n: usize) -> u32 {
        if n == 0 {
            return 0;
        }
        usize::BITS - n.leading_zeros()
    }

    /// 空气在调色板之外的越界哨兵下标（恒等于调色板条目数）。
    ///
    /// 空气（id 0）不进调色板，而是以「越界下标」写入数据数组；读取时
    /// 越界即还原为 0，从而让 16 种方块仍可压缩在 4 bit 内。
    fn air_index(&self) -> usize {
        self.palette_ids.len()
    }

    /// 实际数据打包位宽：报告位宽与空气越界哨兵所需位宽取较大者。
    fn storage_bits(&self) -> u32 {
        let air_bits = Self::bits_to_represent(self.air_index());
        self.bits.max(air_bits)
    }

    /// 读取某索引处的调色板下标（仅在 `bits > 0` 时调用）。
    fn read_index(&self, index: usize) -> usize {
        if self.bits == 0 {
            return 0;
        }
        self.read_index_at(self.storage_bits(), index)
    }

    /// 按指定位宽读取某索引处的条目值（不依赖当前 `bits` 字段，
    /// 供重打包时读取旧位宽下的数据）。
    fn read_index_at(&self, bits: u32, index: usize) -> usize {
        let bits_usize = usize::try_from(bits).unwrap_or(usize::MAX);
        let entries_per_long = 64 / bits_usize.max(1);
        let long_index = index / entries_per_long;
        let bit_index = (index % entries_per_long) * bits_usize;
        let long = self.data.get(long_index).copied().unwrap_or(0);
        let mask = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        usize::try_from((long >> bit_index) & mask).unwrap_or(0)
    }

    /// 写入某索引处的调色板下标（仅在 `bits > 0` 时调用；单值模式无需写）。
    fn write_index(&mut self, index: usize, palette_index: usize) {
        if self.bits == 0 {
            return;
        }
        let bits = self.storage_bits();
        let bits_usize = usize::try_from(bits).unwrap_or(usize::MAX);
        let entries_per_long = 64 / bits_usize.max(1);
        let long_index = index / entries_per_long;
        let bit_index = (index % entries_per_long) * bits_usize;
        if let Some(long) = self.data.get_mut(long_index) {
            let mask = if bits >= 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            let value = u64::try_from(palette_index).unwrap_or(0) & mask;
            // 先清空目标位区间，再置入新下标，保证不破坏同字内相邻条目。
            *long = (*long & !(mask << bit_index)) | (value << bit_index);
        }
    }

    /// 按「报告位宽 + 空气哨兵」确定的数据位宽重打包数据数组。
    ///
    /// 调色板每追加一个方块，空气越界哨兵随之 +1；本函数把旧数据中等于
    /// `old_air` 的格子（旧空气）迁移为新哨兵值，其余下标保持不变，随后
    /// 按新的存储位宽整体重打包。单值模式（`was_single`）整段为空气，
    /// 直接全量填入新哨兵。
    fn repack(&mut self, was_single: bool, old_storage: u32) {
        let new_air = self.air_index();
        let old_air = new_air.saturating_sub(1);
        let indices: Vec<usize> = if was_single {
            vec![new_air; SECTION_VOLUME]
        } else {
            (0..SECTION_VOLUME)
                .map(|i| {
                    let idx = self.read_index_at(old_storage, i);
                    if idx == old_air { new_air } else { idx }
                })
                .collect()
        };
        let new_storage = self.storage_bits();
        let new_bits_usize = usize::try_from(new_storage).unwrap_or(usize::MAX);
        let entries_per_long = 64 / new_bits_usize.max(1);
        let long_count = SECTION_VOLUME.div_ceil(entries_per_long);
        let mut new_data = vec![0u64; long_count];
        for (i, &palette_index) in indices.iter().enumerate() {
            let long_index = i / entries_per_long;
            let bit_index = (i % entries_per_long) * new_bits_usize;
            if let Some(long) = new_data.get_mut(long_index) {
                let mask = if new_storage >= 64 {
                    u64::MAX
                } else {
                    (1u64 << new_storage) - 1
                };
                *long |= (u64::try_from(palette_index).unwrap_or(0) & mask) << bit_index;
            }
        }
        self.data = new_data;
    }
}

/// 单区段光照：天空光与方块光各 4096 字节。
///
/// 索引采用与协议 / Anvil 一致的局部坐标线性化 `(y << 8) | (z << 4) | x`
/// （见 [`light_index`]），取值范围 `[0, 4096)`。光照等级恒为 `0..=15`，
/// 越界或空气按 [`crate::instance::light`] 语义以 `0` 处理。
///
/// 该结构是 `ChunkLightStorage` 接口契约（`complete-framework-gaps` WS1）
/// 的冻结布局，WS2 的 Anvil 序列化直接读写其两个数组，禁止改字段名 / 类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightSection {
    /// 天空光等级（0..=15），逐方块 1 字节。
    pub sky_light: [u8; SECTION_VOLUME],
    /// 方块光等级（0..=15），逐方块 1 字节。
    pub block_light: [u8; SECTION_VOLUME],
}

impl Default for LightSection {
    fn default() -> Self {
        Self {
            sky_light: [0u8; SECTION_VOLUME],
            block_light: [0u8; SECTION_VOLUME],
        }
    }
}

#[cfg(test)]
mod fill_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// 全空气填充：bits=0，data 为空。
    #[test]
    fn fill_blocks_all_air() {
        let mut section = Section::new();
        let ids = vec![0u32; SECTION_VOLUME];
        assert!(section.fill_blocks(&ids).is_ok());
        assert_eq!(section.bits, 0);
        assert!(section.data.is_empty());
        assert_eq!(section.get_block_id(0), 0);
    }

    /// 单一非空气方块填充后读回一致。
    #[test]
    fn fill_blocks_single_id() {
        let mut section = Section::new();
        let ids = vec![42u32; SECTION_VOLUME];
        assert!(section.fill_blocks(&ids).is_ok());
        for i in 0..SECTION_VOLUME {
            assert_eq!(section.get_block_id(i), 42);
        }
    }

    /// 多类型方块填充后读回一致。
    #[test]
    fn fill_blocks_multiple_ids_roundtrip() {
        let mut section = Section::new();
        let ids: Vec<u32> = (0..SECTION_VOLUME).map(|i| (i as u32 % 50) + 1).collect();
        assert!(section.fill_blocks(&ids).is_ok());
        for i in 0..SECTION_VOLUME {
            assert_eq!(section.get_block_id(i), ids[i]);
        }
    }

    /// 17 种方块触发位宽升级。
    #[test]
    fn fill_blocks_bit_width_upgrade() {
        let mut section = Section::new();
        let ids: Vec<u32> = (0..SECTION_VOLUME).map(|i| (i as u32 % 17) + 1).collect();
        assert!(section.fill_blocks(&ids).is_ok());
        assert!(section.bits > 4);
        for i in 0..SECTION_VOLUME {
            assert_eq!(section.get_block_id(i), ids[i]);
        }
    }

    /// 200 种方块填充（匹配 REPORT 中的实际场景）。
    #[test]
    fn fill_blocks_200_types() {
        let mut section = Section::new();
        let ids: Vec<u32> = (0..SECTION_VOLUME)
            .map(|i| (((i as u64).wrapping_mul(73) ^ (i as u64).wrapping_mul(131)).rotate_left(17) % 200) as u32 + 1)
            .collect();
        assert!(section.fill_blocks(&ids).is_ok());
        for i in 0..SECTION_VOLUME {
            assert_eq!(section.get_block_id(i), ids[i]);
        }
    }

    /// 257 种方块切换到 direct 模式。
    #[test]
    fn fill_blocks_direct_mode() {
        let mut section = Section::new();
        let ids: Vec<u32> = (0..SECTION_VOLUME)
            .map(|i| ((i as u32).wrapping_mul(31) % 300) + 1)
            .collect();
        assert!(section.fill_blocks(&ids).is_ok());
        assert_eq!(section.bits, 15);
        for i in 0..SECTION_VOLUME {
            assert_eq!(section.get_block_id(i), ids[i]);
        }
    }

    /// 长度不符拒绝写入，且区段状态不变。
    #[test]
    fn fill_blocks_invalid_length_no_side_effect() {
        let mut section = Section::new();
        for i in 0..SECTION_VOLUME {
            let _ = section.set_block_id(i, 1);
        }
        let before: Vec<u32> = (0..SECTION_VOLUME).map(|i| section.get_block_id(i)).collect();
        let short_ids = vec![1u32; SECTION_VOLUME - 1];
        assert_eq!(section.fill_blocks(&short_ids), Err(SectionFillError::InvalidLength));
        let after: Vec<u32> = (0..SECTION_VOLUME).map(|i| section.get_block_id(i)).collect();
        assert_eq!(before, after);
    }

    /// 原有 set_block_id 单格写入回归测试。
    #[test]
    fn fill_blocks_set_block_id_still_works() {
        let mut section = Section::new();
        assert!(section.set_block_id(0, 42));
        assert_eq!(section.get_block_id(0), 42);
        assert_eq!(section.get_block_id(1), 0);
    }

    /// Chunk::fill_section_blocks 正常填充并标记脏。
    #[test]
    fn chunk_fill_section_blocks_marks_dirty() {
        let mut chunk = Chunk::new(0, 0, 1);
        let ids: Vec<u32> = (0..SECTION_VOLUME).map(|i| (i as u32 % 10) + 1).collect();
        assert!(chunk.fill_section_blocks(0, &ids).is_ok());
        assert!(chunk.dirty_sections[0]);
        for i in 0..SECTION_VOLUME {
            assert_eq!(chunk.get_block(0, i), ids[i]);
        }
    }

    /// Chunk::fill_section_blocks 越界返回 SectionOutOfBounds。
    #[test]
    fn chunk_fill_section_blocks_out_of_bounds() {
        let mut chunk = Chunk::new(0, 0, 1);
        let ids = vec![1u32; SECTION_VOLUME];
        assert_eq!(
            chunk.fill_section_blocks(5, &ids),
            Err(SectionFillError::SectionOutOfBounds)
        );
    }

    /// Chunk::fill_section_blocks 越界时不返回 InvalidLength。
    #[test]
    fn chunk_fill_section_blocks_error_is_not_invalid_length() {
        let mut chunk = Chunk::new(0, 0, 1);
        let ids = vec![1u32; SECTION_VOLUME];
        let result = chunk.fill_section_blocks(99, &ids);
        assert!(matches!(result, Err(SectionFillError::SectionOutOfBounds)));
    }
}

impl LightSection {
    /// 构造一个全零（无光照）的区段光照。
    pub fn new() -> Self {
        Self::default()
    }

    /// 读取某区段的索引处天空光（越界返回 0）。
    pub fn sky(&self, index: usize) -> u8 {
        self.sky_light.get(index).copied().unwrap_or(0)
    }

    /// 读取某区段的索引处方块光（越界返回 0）。
    pub fn block(&self, index: usize) -> u8 {
        self.block_light.get(index).copied().unwrap_or(0)
    }

    /// 写入某区段的索引处天空光（越界忽略）。
    pub fn set_sky(&mut self, index: usize, value: u8) {
        if let Some(slot) = self.sky_light.get_mut(index) {
            *slot = value.min(15);
        }
    }

    /// 写入某区段的索引处方块光（越界忽略）。
    pub fn set_block(&mut self, index: usize, value: u8) {
        if let Some(slot) = self.block_light.get_mut(index) {
            *slot = value.min(15);
        }
    }
}

/// 由区段内局部坐标 `(x, y, z)` 计算线性光照索引。
///
/// 与 Minecraft 区块内 `x/y/z ∈ [0, 16)` 的打包约定一致：`(y << 8) | (z << 4) | x`。
/// 越界坐标返回的最大值不超过 `4095`（`SECTION_VOLUME - 1`）。
#[must_use]
pub fn light_index(x: usize, y: usize, z: usize) -> usize {
    let x = x & 0x0f;
    let y = y & 0x0f;
    let z = z & 0x0f;
    (y << 8) | (z << 4) | x
}

/// 区块：坐标 (x, z) 与垂直堆叠的若干区段。
#[derive(Default, Component, Debug, Clone, PartialEq, Eq)]
#[component(storage = "sparse")]
pub struct Chunk {
    /// 区块 X 坐标。
    pub x: i32,
    /// 区块 Z 坐标。
    pub z: i32,
    /// 自底向上的区段列表。
    pub sections: Vec<Section>,
    /// 区段脏标记（长度 = 区段数，初始全为 `false`）。
    pub dirty_sections: Vec<bool>,
    /// 每区段光照存储（长度恒等于 `sections.len()`）。
    ///
    /// 区段数变化时由 [`Chunk::ensure_light_synced`] 同步 resize（新增区段补零）。
    /// 冻结接口 `ChunkLightStorage`（`complete-framework-gaps` WS1）。
    pub light: Vec<LightSection>,
}

impl Chunk {
    /// 以坐标与区段数量构造区块（至少 1 个区段）。
    pub fn new(x: i32, z: i32, section_count: usize) -> Self {
        let count = section_count.max(1);
        Self {
            x,
            z,
            sections: vec![Section::new(); count],
            dirty_sections: vec![false; count],
            light: vec![LightSection::new(); count],
        }
    }

    /// 返回区块 X 坐标。
    pub fn x(&self) -> i32 {
        self.x
    }

    /// 返回区块 Z 坐标。
    pub fn z(&self) -> i32 {
        self.z
    }

    /// 区段数量。
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// 光照区段数量（恒等于 `section_count`）。
    pub fn light_section_count(&self) -> usize {
        self.light.len()
    }

    /// 读取全部区段光照（只读）。
    pub fn light_sections(&self) -> &[LightSection] {
        &self.light
    }

    /// 读取全部区段光照（可写），供 LightEngine 刷新。
    pub fn light_sections_mut(&mut self) -> &mut [LightSection] {
        &mut self.light
    }

    /// 提取指定方向边界格的光照数据，按区段顺序展平为连续 `Vec<u8>`。
    ///
    /// - `East`：取 `lx = 15` 的 `(y, z)` 列（`lz ∈ [0,16)`）
    /// - `West`：取 `lx = 0` 的 `(y, z)` 列
    /// - `South`：取 `lz = 15` 的 `(y, x)` 列（`lx ∈ [0,16)`）
    /// - `North`：取 `lz = 0` 的 `(y, x)` 列
    ///
    /// 输出长度恒为 `section_count * 256`，即每区段 256 字节按顺序拼接。
    pub fn extract_light_boundary(&self, dir: LightBoundaryDir) -> SectionLightBoundary {
        let section_count = self.section_count();
        let len = section_count * 256;
        let mut sky = vec![0u8; len];
        let mut block = vec![0u8; len];

        for (section_idx, section_light) in self.light.iter().enumerate() {
            let base = section_idx * 256;
            for lz in 0..16 {
                for lx in 0..16 {
                    let flat = lz * 16 + lx;
                    match dir {
                        LightBoundaryDir::East => {
                            sky[base + flat] = section_light.sky(light_index(15, lz, lx));
                            block[base + flat] = section_light.block(light_index(15, lz, lx));
                        }
                        LightBoundaryDir::West => {
                            sky[base + flat] = section_light.sky(light_index(0, lz, lx));
                            block[base + flat] = section_light.block(light_index(0, lz, lx));
                        }
                        LightBoundaryDir::South => {
                            sky[base + flat] = section_light.sky(light_index(lx, lz, 15));
                            block[base + flat] = section_light.block(light_index(lx, lz, 15));
                        }
                        LightBoundaryDir::North => {
                            sky[base + flat] = section_light.sky(light_index(lx, lz, 0));
                            block[base + flat] = section_light.block(light_index(lx, lz, 0));
                        }
                    }
                }
            }
        }
        SectionLightBoundary::from_slices(sky, block)
    }

    /// 保证 `light` 长度与 `sections` 一致：不足补零、过多截断。
    ///
    /// 区段数变化（如 Anvil 加载或重新分配区段）后调用，避免 `light`
    /// 与 `sections` 失配导致越界读取；新增区段以零光照填充。
    pub fn ensure_light_synced(&mut self) {
        let target = self.sections.len();
        if self.light.len() < target {
            self.light.resize(target, LightSection::new());
        } else if self.light.len() > target {
            self.light.truncate(target);
        }
    }

    /// 读取某区段某索引处的方块 ID（区段越界返回 0）。
    pub fn get_block(&self, section: usize, index: usize) -> u32 {
        self.sections
            .get(section)
            .map(|section| section.get_block_id(index))
            .unwrap_or(0)
    }

    /// 写入某区段某索引处的方块 ID（区段越界返回 `false`）。
    ///
    /// 写入成功后将该区段标记为脏。
    pub fn set_block(&mut self, section: usize, index: usize, id: u32) -> bool {
        let written = match self.sections.get_mut(section) {
            Some(section) => section.set_block_id(index, id),
            None => false,
        };
        if written {
            self.mark_dirty(section);
        }
        written
    }

    /// 整区段批量填充：接受长度为 `SECTION_VOLUME` 的方块 id 切片，
    /// 委托给 [`Section::fill_blocks`]，成功后标记该区段为脏。
    ///
    /// 不触发任何光照重算或 LRU 更新。
    pub fn fill_section_blocks(&mut self, section: usize, ids: &[u32]) -> Result<(), SectionFillError> {
        if section >= self.sections.len() {
            return Err(SectionFillError::SectionOutOfBounds);
        }
        self.sections[section].fill_blocks(ids)?;
        self.mark_dirty(section);
        Ok(())
    }

    /// 将某区段标记为脏（区段越界时忽略）。
    pub fn mark_dirty(&mut self, section: usize) {
        if let Some(flag) = self.dirty_sections.get_mut(section) {
            *flag = true;
        }
    }

    /// 取走全部脏区段索引并清空脏标记。
    pub fn take_dirty_sections(&mut self) -> Vec<usize> {
        let mut dirty = Vec::new();
        for (index, &flag) in self.dirty_sections.iter().enumerate() {
            if flag {
                dirty.push(index);
            }
        }
        for flag in &mut self.dirty_sections {
            *flag = false;
        }
        dirty
    }

    /// 以注册表语义读取某位置的方块（内部经 `Block::from_state_id`）。
    pub fn get_block_state(&self, section: usize, index: usize, registry: &BlockRegistry) -> Block {
        // registry 为后续语义扩展（属性解析）预留，当前实现直接取状态 id。
        let _ = registry;
        Block::from_state_id(self.get_block(section, index))
    }

    /// 按世界坐标写入方块 id（坐标顺序 `(x, y, z)`）。
    ///
    /// y < 0 时返回 `false`；否则计算所属区段和局部索引，委托给 [`Self::set_block`]。
    /// 写入成功后将对应区段标记为脏。
    pub fn set_block_world(&mut self, x: i32, y: i32, z: i32, id: u32) -> bool {
        if y < 0 {
            return false;
        }
        let section = usize::try_from(y / 16).unwrap_or(usize::MAX);
        let y_local = usize::try_from(y.rem_euclid(16)).unwrap_or(0);
        let x_local = usize::try_from(x.rem_euclid(16)).unwrap_or(0);
        let z_local = usize::try_from(z.rem_euclid(16)).unwrap_or(0);
        let index = y_local * 256 + z_local * 16 + x_local;
        self.set_block(section, index, id)
    }

    /// 按世界坐标读取方块 id（坐标顺序 `(x, y, z)`）。
    ///
    /// y < 0 时返回 0；否则计算所属区段和局部索引，委托给 [`Self::get_block`]。
    pub fn get_block_world(&self, x: i32, y: i32, z: i32) -> u32 {
        if y < 0 {
            return 0;
        }
        let section = usize::try_from(y / 16).unwrap_or(usize::MAX);
        let y_local = usize::try_from(y.rem_euclid(16)).unwrap_or(0);
        let x_local = usize::try_from(x.rem_euclid(16)).unwrap_or(0);
        let z_local = usize::try_from(z.rem_euclid(16)).unwrap_or(0);
        let index = y_local * 256 + z_local * 16 + x_local;
        self.get_block(section, index)
    }

    /// 按世界坐标读取方块（坐标顺序 `(x, y, z)`，语义经注册表解析）。
    pub fn get_block_state_world(&self, x: i32, y: i32, z: i32, _registry: &BlockRegistry) -> Block {
        Block::from_state_id(self.get_block_world(x, y, z))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn section_is_initialized_to_air() {
        let section = Section::new();
        assert_eq!(section.len(), SECTION_VOLUME);
        assert!(!section.is_empty());
        assert_eq!(section.get_block_id(0), 0);
        assert_eq!(section.get_block_id(4095), 0);
    }

    #[test]
    fn section_set_and_get_roundtrip() {
        let mut section = Section::new();
        assert!(section.set_block_id(0, 42));
        assert_eq!(section.get_block_id(0), 42);
        // 未写入区域仍为空气
        assert_eq!(section.get_block_id(1), 0);
    }

    #[test]
    fn section_out_of_bounds_is_safe() {
        let mut section = Section::new();
        assert!(!section.set_block_id(4096, 1));
        assert_eq!(section.get_block_id(4096), 0);
        assert_eq!(section.get_block_id(99999), 0);
    }

    #[test]
    fn air_section_degrades_to_tiny_single_value_palette() {
        let section = Section::new();
        // 单值模式：palette 仅 1 个条目、bits = 0、data 为空。
        assert_eq!(section.palette.len(), 1);
        assert_eq!(section.palette_ids.len(), 1);
        assert_eq!(section.bits, 0);
        assert!(section.data.is_empty());
    }

    #[test]
    fn palette_roundtrip_with_multiple_ids() {
        let mut section = Section::new();
        // 写入多种方块 id，读回必须与写入一致。
        for (index, id) in [7u32, 3, 42, 100, 1].iter().enumerate() {
            assert!(section.set_block_id(index * 100, *id));
        }
        for (index, id) in [7u32, 3, 42, 100, 1].iter().enumerate() {
            assert_eq!(section.get_block_id(index * 100), *id);
        }
        assert_eq!(section.get_block_id(0), 7);
        assert_eq!(section.get_block_id(400), 1);
        // 其余位置仍为空气
        assert_eq!(section.get_block_id(1), 0);
    }

    #[test]
    fn section_full_write_roundtrip() {
        let mut section = Section::new();
        for index in 0..SECTION_VOLUME {
            let id = u32::try_from(index % 7).unwrap_or(0);
            assert!(section.set_block_id(index, id));
        }
        for index in 0..SECTION_VOLUME {
            let expected = u32::try_from(index % 7).unwrap_or(0);
            assert_eq!(section.get_block_id(index), expected);
        }
        // 内容相等（自定义 PartialEq 按内容比较）
        let mut clone = section.clone();
        for index in 0..SECTION_VOLUME {
            let expected = u32::try_from(index % 7).unwrap_or(0);
            assert!(clone.set_block_id(index, expected));
        }
        assert_eq!(section, clone);
    }

    #[test]
    fn palette_expands_bits_with_more_than_sixteen_ids() {
        let mut section = Section::new();
        // 15 种不同 id + 空气（palette 初始含 air）= 16 个条目：bits 应为 4
        // （4 位可表示下标 0..=15，与 chunk_serializer 的 2..=16 种规则一致）。
        for id in 1..=15u32 {
            assert!(section.set_block_id(usize::try_from(id).unwrap_or(0), id));
        }
        assert_eq!(section.bits, 4);
        // 第 16 种新 id 使 palette 达 17 个条目：位宽提升（5 位）。
        assert!(section.set_block_id(16, 16));
        assert!(section.bits > 4);
        // 全部读回一致。
        for id in 1..=16u32 {
            let index = usize::try_from(id).unwrap_or(0);
            assert_eq!(section.get_block_id(index), id);
        }
    }

    #[test]
    fn random_read_write_consistency() {
        use std::collections::HashMap;

        let mut rng_state = 0x1234_5678u64;
        // 简单的 xorshift 伪随机数生成器，避免引入外部依赖。
        let mut next = || {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            rng_state
        };
        let mut expected = HashMap::new();
        let mut section = Section::new();
        for _ in 0..3000 {
            let index =
                usize::try_from(next() % u64::try_from(SECTION_VOLUME).unwrap_or(0)).unwrap_or(0);
            let id = u32::try_from(next() % 64).unwrap_or(0);
            assert!(section.set_block_id(index, id));
            expected.insert(index, id);
        }
        for index in 0..SECTION_VOLUME {
            let want = expected.get(&index).copied().unwrap_or(0);
            assert_eq!(section.get_block_id(index), want);
        }
    }

    #[test]
    fn chunk_block_routing() {
        let mut chunk = Chunk::new(3, -2, 2);
        assert_eq!(chunk.x(), 3);
        assert_eq!(chunk.z(), -2);
        assert_eq!(chunk.section_count(), 2);
        assert!(chunk.set_block(1, 100, 7));
        assert_eq!(chunk.get_block(1, 100), 7);
        // 越界区段写入失败
        assert!(!chunk.set_block(5, 0, 1));
        assert_eq!(chunk.get_block(5, 0), 0);
    }

    #[test]
    fn dirty_sections_track_and_take() {
        let mut chunk = Chunk::new(0, 0, 3);
        assert_eq!(chunk.dirty_sections, vec![false, false, false]);
        // set_block 成功后自动标记脏。
        assert!(chunk.set_block(0, 0, 1));
        assert!(chunk.set_block(2, 5, 2));
        assert_eq!(chunk.take_dirty_sections(), vec![0, 2]);
        // 取走后清空。
        assert!(chunk.take_dirty_sections().is_empty());
        // mark_dirty 越界忽略。
        chunk.mark_dirty(99);
        assert!(chunk.take_dirty_sections().is_empty());
    }

    #[test]
    fn get_block_state_routes_through_registry() {
        // 最小注册表：仅 air（id 0）。
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"
        "#;
        let registry =
            BlockRegistry(crate::resource::registries::Registry::from_toml_str(toml).unwrap());
        let mut chunk = Chunk::new(0, 0, 1);
        assert!(chunk.set_block(0, 10, 0));
        let block = chunk.get_block_state(0, 10, &registry);
        assert_eq!(block.state_id(), 0);
        assert!(block.is_air(&registry));
    }

    #[test]
    fn east_boundary_flattens_correctly() {
        let mut chunk = Chunk::new(0, 0, 2);
        // 在 section 0 的 lx=15 列写入不同值以区分方向。
        for lz in 0..16 {
            for lx in 0..16 {
                chunk.light_sections_mut()[0].set_sky(light_index(15, lz, lx), lz as u8);
                chunk.light_sections_mut()[0].set_block(light_index(15, lz, lx), lx as u8);
            }
        }
        // section 1 全零（默认）。
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::East);
        assert_eq!(boundary.sky().len(), 2 * 256);
        assert_eq!(boundary.block().len(), 2 * 256);
        // 第一个区段：sky[base + lz*16 + lx] = lz，block[base + lz*16 + lx] = lx。
        let base = 0;
        for lz in 0..16 {
            for lx in 0..16 {
                let flat = lz * 16 + lx;
                assert_eq!(boundary.sky()[base + flat], lz as u8);
                assert_eq!(boundary.block()[base + flat], lx as u8);
            }
        }
        // 第二个区段全零。
        let base1 = 256;
        for i in 0..256 {
            assert_eq!(boundary.sky()[base1 + i], 0);
            assert_eq!(boundary.block()[base1 + i], 0);
        }
    }

    #[test]
    fn west_boundary_flattens_correctly() {
        let mut chunk = Chunk::new(0, 0, 1);
        // West 方向只读 lx=0 列，因此只在该列写入数据。
        for lz in 0..16 {
            chunk.light_sections_mut()[0].set_sky(light_index(0, lz, 0), lz as u8);
            chunk.light_sections_mut()[0].set_block(light_index(0, lz, 0), lz as u8);
        }
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::West);
        assert_eq!(boundary.sky().len(), 256);
        for lz in 0..16 {
            for lx in 0..16 {
                let flat = lz * 16 + lx;
                // West 边界只含 lx=0 列的数据，其余列（lx>0）为默认零值。
                if lx == 0 {
                    assert_eq!(boundary.sky()[flat], lz as u8);
                    assert_eq!(boundary.block()[flat], lz as u8);
                } else {
                    assert_eq!(boundary.sky()[flat], 0);
                    assert_eq!(boundary.block()[flat], 0);
                }
            }
        }
    }

    #[test]
    fn south_boundary_flattens_correctly() {
        let mut chunk = Chunk::new(0, 0, 1);
        // South 方向只读 lz=15 列，因此只在该列写入数据。
        for lz in 0..16 {
            for lx in 0..16 {
                chunk.light_sections_mut()[0].set_sky(light_index(lx, lz, 15), lx as u8);
                chunk.light_sections_mut()[0].set_block(light_index(lx, lz, 15), lz as u8);
            }
        }
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::South);
        assert_eq!(boundary.sky().len(), 256);
        for lz in 0..16 {
            for lx in 0..16 {
                let flat = lz * 16 + lx;
                assert_eq!(boundary.sky()[flat], lx as u8);
                assert_eq!(boundary.block()[flat], lz as u8);
            }
        }
    }

    #[test]
    fn north_boundary_flattens_correctly() {
        let mut chunk = Chunk::new(0, 0, 1);
        // North 方向：extract_light_boundary(North) 遍历所有 lx/lz，读取 light_index(lx, lz, 0)。
        // 测试需向相同索引写入，使每个 (lx,lz) 位置的 sky/block 值与断言一致。
        for lz in 0..16 {
            for lx in 0..16 {
                chunk.light_sections_mut()[0].set_sky(light_index(lx, lz, 0), (lx + lz) as u8);
                chunk.light_sections_mut()[0].set_block(light_index(lx, lz, 0), ((lx + lz) & 0x0f) as u8);
            }
        }
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::North);
        assert_eq!(boundary.sky().len(), 256);
        for lz in 0..16 {
            for lx in 0..16 {
                let flat = lz * 16 + lx;
                assert_eq!(boundary.sky()[flat], (lx + lz) as u8);
                assert_eq!(boundary.block()[flat], ((lx + lz) & 0x0f) as u8);
            }
        }
    }

    #[test]
    fn multi_section_boundary_concatenates_sequentially() {
        let mut chunk = Chunk::new(0, 0, 3);
        // 每个区段遍历全部 lx/lz，向 light_index(lx, lz, 15) 写入不同值以区分方向。
        // South 方向：extract_light_boundary 读 light_index(lx, lz, 15)。
        for section_idx in 0..3 {
            for lz in 0..16 {
                for lx in 0..16 {
                    let val = (section_idx + lz) as u8;
                    chunk.light_sections_mut()[section_idx]
                        .set_sky(light_index(lx, lz, 15), val);
                    chunk.light_sections_mut()[section_idx]
                        .set_block(light_index(lx, lz, 15), val.wrapping_sub(1));
                }
            }
        }
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::South);
        assert_eq!(boundary.sky().len(), 3 * 256);
        assert_eq!(boundary.block().len(), 3 * 256);
        for section_idx in 0..3 {
            let base = section_idx * 256;
            for lz in 0..16 {
                for lx in 0..16 {
                    let flat = lz * 16 + lx;
                    let expected_sky = (section_idx + lz) as u8;
                    let expected_block = (section_idx as u8 + lz as u8).wrapping_sub(1);
                    assert_eq!(boundary.sky()[base + flat], expected_sky);
                    assert_eq!(boundary.block()[base + flat], expected_block);
                }
            }
        }
    }

    #[test]
    fn zero_light_chunk_returns_all_zeros() {
        let chunk = Chunk::new(0, 0, 2);
        for dir in [
            LightBoundaryDir::East,
            LightBoundaryDir::West,
            LightBoundaryDir::South,
            LightBoundaryDir::North,
        ] {
            let boundary = chunk.extract_light_boundary(dir);
            assert_eq!(boundary.sky().len(), 2 * 256);
            assert!(boundary.sky().iter().all(|&v| v == 0));
            assert!(boundary.block().iter().all(|&v| v == 0));
        }
    }

    #[test]
    fn boundary_matches_section_layout() {
        // 验证单区段内边界格的取值与 extract_boundary 的逻辑等价。
        let mut chunk = Chunk::new(0, 0, 1);
        // 仅在 lx=15 边界写入 sky=7，其余保持默认 0。
        for ly in 0..16 {
            for lz in 0..16 {
                chunk.light_sections_mut()[0]
                    .set_sky(light_index(15, ly, lz), 7);
            }
        }
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::East);
        for ly in 0..16 {
            for lz in 0..16 {
                let idx = lz * 16 + ly;
                assert_eq!(boundary.sky()[idx], 7, "ly={}, lz={}", ly, lz);
                assert_eq!(boundary.block()[idx], 0);
            }
        }
    }

    #[test]
    fn multi_section_boundary_with_nonzero_sections() {
        let mut chunk = Chunk::new(0, 0, 4);
        // 奇数区段写入非零值。
        for section_idx in [1, 3] {
            for lz in 0..16 {
                for lx in 0..16 {
                    chunk.light_sections_mut()[section_idx]
                        .set_sky(light_index(lx, 0, lz), 10 + section_idx as u8);
                    chunk.light_sections_mut()[section_idx]
                        .set_block(light_index(lx, 0, lz), 5 + section_idx as u8);
                }
            }
        }
        let boundary = chunk.extract_light_boundary(LightBoundaryDir::North);
        // 奇数区段 sky/block 非零，偶数区段为零。
        for section_idx in 0..4 {
            let base = section_idx * 256;
            if section_idx % 2 == 1 {
                for i in 0..256 {
                    assert_eq!(boundary.sky()[base + i], 10 + section_idx as u8);
                    assert_eq!(boundary.block()[base + i], 5 + section_idx as u8);
                }
            } else {
                for i in 0..256 {
                    assert_eq!(boundary.sky()[base + i], 0);
                    assert_eq!(boundary.block()[base + i], 0);
                }
            }
        }
    }

    // ── Chunk::set_block_world / get_block_world 测试 ────────────────────────────

    #[test]
    fn chunk_set_block_world_roundtrip_single_chunk() {
        let mut chunk = Chunk::new(1, 2, 1);
        assert!(chunk.set_block_world(16, 5, 32, 7));
        assert_eq!(chunk.get_block_world(16, 5, 32), 7);
    }

    #[test]
    fn chunk_set_block_world_negative_y_rejected() {
        let mut chunk = Chunk::new(0, 0, 1);
        assert!(!chunk.set_block_world(0, -1, 0, 1));
        assert_eq!(chunk.get_block_world(0, -1, 0), 0);
    }

    #[test]
    fn chunk_set_block_world_cross_section_boundary() {
        let mut chunk = Chunk::new(0, 0, 2);
        // y=15 在 section 0（y ∈ [0, 15]），y=16 在 section 1（y ∈ [16, 31]）。
        assert!(chunk.set_block_world(0, 15, 0, 1));
        assert!(chunk.set_block_world(0, 16, 0, 2));
        assert_eq!(chunk.get_block_world(0, 15, 0), 1);
        assert_eq!(chunk.get_block_world(0, 16, 0), 2);
        // 相邻位置仍为空气
        assert_eq!(chunk.get_block_world(0, 14, 0), 0);
        assert_eq!(chunk.get_block_world(0, 17, 0), 0);
    }

    #[test]
    fn chunk_set_block_world_negative_coords() {
        let mut chunk = Chunk::new(-1, -1, 1);
        // (-16, 5, -16) 对应 chunk(-1,-1) 内 x=0,y=5,z=0
        assert!(chunk.set_block_world(-16, 5, -16, 3));
        assert_eq!(chunk.get_block_world(-16, 5, -16), 3);
    }

    #[test]
    fn chunk_set_block_world_out_of_sections_returns_false() {
        let mut chunk = Chunk::new(0, 0, 1);
        // section 越界：y=32 超出单区段（y ∈ [0,15]）
        assert!(!chunk.set_block_world(0, 32, 0, 1));
        assert_eq!(chunk.get_block_world(0, 32, 0), 0);
    }

    #[test]
    fn chunk_set_block_world_dirty_flag_set() {
        let mut chunk = Chunk::new(0, 0, 1);
        assert!(chunk.set_block_world(5, 5, 5, 42));
        assert!(chunk.dirty_sections[0]);
    }

    #[test]
    fn chunk_get_block_state_world_routes_through_registry() {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"
        "#;
        let registry =
            BlockRegistry(crate::resource::registries::Registry::from_toml_str(toml).unwrap());
        let mut chunk = Chunk::new(0, 0, 1);
        assert!(chunk.set_block_world(0, 0, 0, 0));
        let block = chunk.get_block_state_world(0, 0, 0, &registry);
        assert_eq!(block.state_id(), 0);
        assert!(block.is_air(&registry));
    }
}
