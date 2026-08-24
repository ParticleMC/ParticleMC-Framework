//! 光照传播引擎（complete-framework-gaps WS1-T3）。
//!
//! 依据方块不透明度（`BlockRegistry::light_opacity`）与发光度
//! （`BlockRegistry::light_emission`）重算 [`Chunk`] 的天空光与方块光两层光照，
//! 写入 `Chunk.light`（`ChunkLightStorage` 冻结接口）。
//!
//! 算法总览：
//! - **方块光**：由发光方块（与邻块边界光源）作为种子，做 6 邻域 BFS 洪泛，
//!   每穿过一个方块衰减 `1 + opacity`，截断到 `0..=15`。
//! - **天空光**：每列自顶向下竖直扫描（顶部以上视为全亮 15，遇不透明方块按
//!   不透明度衰减，透明方块透传），再以邻块边界天空光做种子修正接缝。
//!
//! 边界处理保证「区块边界光照一致性」（见 spec WS1 scenario「区块边界光照
//! 一致性」）：计算区块 B 时，其邻块 A 已先行计算，B 边界格的天空光/方块光
//! 读取 A 对应边界值作为种子，避免出现亮暗突变接缝。
//!
//! 变更标识符：`complete-framework-gaps`（WS1-T3）。见
//! `.specs/complete-framework-gaps/spec.md` 与 `docs/decisions.md`。

use std::collections::VecDeque;

use super::chunk::{Chunk, light_index};
use crate::resource::registries::BlockRegistry;

/// 邻块顺序：`[+x, -x, +z, -z]`（东、西、南、北）。
///
/// [`LightEngine::recompute`] 的 `neighbors` 参数严格按此顺序提供，边界 seed
/// 与跨区块方块光传播均依赖该固定布局。
pub type Neighbors<'a> = [Option<&'a Chunk>; 4];

/// 光照边界方向：东、西、南、北，对应 `Neighbors` 数组下标 `[0, 1, 2, 3]`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightBoundaryDir {
    /// +x 方向（东）。
    East,
    /// -x 方向（西）。
    West,
    /// +z 方向（南）。
    South,
    /// -z 方向（北）。
    North,
}

/// 单个方向的光照边界数据。
///
/// `sky` 和 `block` 的长度均为 `section_count * 256`，按全局 y 与水平坐标
/// 线性化：`index = global_y * 16 + coord`，其中 `coord` 为 `lz`（东/西方向）
/// 或 `lx`（南/北方向）。
pub struct SectionLightBoundary {
    /// 天空光边界值（0..=15）。
    sky: Vec<u8>,
    /// 方块光边界值（0..=15）。
    block: Vec<u8>,
}

impl SectionLightBoundary {
    /// 构造全零边界（`section_count` 个区段，每个区段 256 个边界格）。
    #[must_use]
    pub fn empty(section_count: usize) -> Self {
        let len = section_count * 256;
        Self {
            sky: vec![0; len],
            block: vec![0; len],
        }
    }

    /// 按给定天空光与方块光切片构造边界。
    #[must_use]
    pub fn from_slices(sky: Vec<u8>, block: Vec<u8>) -> Self {
        Self { sky, block }
    }

    /// 返回天空光边界值切片。
    #[must_use]
    pub fn sky(&self) -> &[u8] {
        &self.sky
    }

    /// 返回方块光边界值切片。
    #[must_use]
    pub fn block(&self) -> &[u8] {
        &self.block
    }
}

/// 光照传播引擎：纯算法集合，无状态，仅经 [`Chunk`] 与 [`BlockRegistry`] 读写。
pub struct LightEngine;

impl LightEngine {
    /// 从四个方向的邻居区块提取边界光照数据。
    ///
    /// 对于每个方向，遍历本块对应边界格（`lx/lz ∈ {0, 15}`），读取邻块对应
    /// 格的光值并写入 [`SectionLightBoundary`]。邻块缺失或越界时该格为 `0`。
    pub fn extract_boundary(chunk: &Chunk, neighbors: &Neighbors<'_>) -> [SectionLightBoundary; 4] {
        let mut boundary = [
            SectionLightBoundary::empty(chunk.section_count()),
            SectionLightBoundary::empty(chunk.section_count()),
            SectionLightBoundary::empty(chunk.section_count()),
            SectionLightBoundary::empty(chunk.section_count()),
        ];
        let height = chunk.section_count() * 16;

        for (dir, boundary_item) in boundary.iter_mut().enumerate() {
            for ly in 0..height {
                for coord in 0..16 {
                    let lx = match dir {
                        0 => 15,
                        1 => 0,
                        2 => coord,
                        3 => coord,
                        _ => continue,
                    };
                    let lz = match dir {
                        0 => coord,
                        1 => coord,
                        2 => 15,
                        3 => 0,
                        _ => continue,
                    };
                    let boundary_idx = ly * 16 + coord;
                    let n_lx = match dir {
                        0 => 0,
                        1 => 15,
                        2 => lx,
                        3 => lx,
                        _ => continue,
                    };
                    let n_lz = match dir {
                        0 => lz,
                        1 => lz,
                        2 => 0,
                        3 => 15,
                        _ => continue,
                    };
                    let n_chunk = match neighbors.get(dir) {
                        Some(Some(c)) => c,
                        _ => continue,
                    };
                    let n_section = ly / 16;
                    let n_y_local = ly % 16;
                    let n_index = light_index(n_lx, n_y_local, n_lz);
                    let section_light = match n_chunk.light_sections().get(n_section) {
                        Some(s) => s,
                        _ => continue,
                    };
                    boundary_item.sky[boundary_idx] = section_light.sky(n_index);
                    boundary_item.block[boundary_idx] = section_light.block(n_index);
                }
            }
        }
        boundary
    }

    /// 重算单个区块的天空光与方块光两层光照，接收预计算的边界数据。
    ///
    /// - `chunk`：待刷新光照的区块（其 `light` 长度经 `ensure_light_synced`
    ///   与 `sections` 对齐）。
    /// - `boundary`：`[东, 西, 南, 北]` 四个方向的边界光照数据（由
    ///   [`Self::extract_boundary`] 生成，或由调用方直接构造）。
    /// - `registry`：方块光属性来源（不透明度 / 发光度）。
    ///
    /// 调用方需保证 `boundary` 中的数据已正确填充。
    pub fn recompute_with_boundary(
        chunk: &mut Chunk,
        boundary: &[SectionLightBoundary; 4],
        registry: &BlockRegistry,
    ) {
        chunk.ensure_light_synced();
        let height = chunk.section_count() * 16;
        // 把区块方块 id 读入本地副本，避免后续读写 `Chunk.light` 时的借用冲突。
        // 线性布局 `ly * 256 + lz * 16 + lx` 与 `Section` 内部索引对齐：
        // `get_block(ly/16, light_index(lx, ly%16, lz))`。
        let mut blocks = Vec::with_capacity(height * 256);
        for ly in 0..height {
            let section = ly / 16;
            let y_local = ly % 16;
            for lz in 0..16 {
                for lx in 0..16 {
                    let index = light_index(lx, y_local, lz);
                    blocks.push(chunk.get_block(section, index));
                }
            }
        }

        Self::compute_block_light_boundary(chunk, boundary, &blocks, height, registry);
        Self::compute_sky_light_boundary(chunk, boundary, &blocks, height, registry);
    }

    /// 重算单个区块的天空光与方块光两层光照。
    ///
    /// - `chunk`：待刷新光照的区块（其 `light` 长度经 `ensure_light_synced`
    ///   与 `sections` 对齐）。
    /// - `neighbors`：`[+x, -x, +z, -z]` 四个方向上的邻块（缺块为 `None`）。
    /// - `registry`：方块光属性来源（不透明度 / 发光度）。
    ///
    /// 调用方需保证 `neighbors` 中的区块已先行完成光照计算，以满足边界一致性。
    pub fn recompute(chunk: &mut Chunk, neighbors: &Neighbors<'_>, registry: &BlockRegistry) {
        let boundary = Self::extract_boundary(chunk, neighbors);
        Self::recompute_with_boundary(chunk, &boundary, registry);
    }

    /// 方块光 BFS 洪泛：发光方块（含邻块边界光源）为种子，6 邻域衰减传播。
    fn compute_block_light_boundary(
        chunk: &mut Chunk,
        boundary: &[SectionLightBoundary; 4],
        blocks: &[u32],
        height: usize,
        registry: &BlockRegistry,
    ) {
        // 清零方块光层。
        for section in chunk.light_sections_mut() {
            for value in section.block_light.iter_mut() {
                *value = 0;
            }
        }

        // 队列元素：(局部 x, 局部 y, 局部 z, 当前光等级)。
        let mut queue: VecDeque<(usize, usize, usize, u8)> = VecDeque::new();

        // 种子 1：本区块内的发光方块。
        for ly in 0..height {
            let section = ly / 16;
            let y_local = ly % 16;
            for lz in 0..16 {
                for lx in 0..16 {
                    let flat = ly * 256 + lz * 16 + lx;
                    let id = blocks.get(flat).copied().unwrap_or(0);
                    let emission = registry.light_emission(id);
                    if emission > 0 {
                        let index = light_index(lx, y_local, lz);
                        if let Some(section_light) = chunk.light_sections_mut().get_mut(section) {
                            section_light.set_block(index, emission);
                        }
                        queue.push_back((lx, ly, lz, emission));
                    }
                }
            }
        }

        // 种子 2：邻块边界方块光（满足「边界光照一致性」）。
        for (dir, boundary_item) in boundary.iter().enumerate() {
            for ly in 0..height {
                let section = ly / 16;
                let y_local = ly % 16;
                for coord in 0..16 {
                    let (lx, lz) = match dir {
                        0 => (15, coord),
                        1 => (0, coord),
                        2 => (coord, 15),
                        3 => (coord, 0),
                        _ => continue,
                    };
                    let boundary_idx = ly * 16 + coord;
                    let incoming = boundary_item.block[boundary_idx];
                    if incoming == 0 {
                        continue;
                    }
                    let index = light_index(lx, y_local, lz);
                    if let Some(section_light) = chunk.light_sections_mut().get_mut(section)
                        && section_light.block(index) < incoming
                    {
                        section_light.set_block(index, incoming);
                        queue.push_back((lx, ly, lz, incoming));
                    }
                }
            }
        }

        // BFS 洪泛：从队列取出亮格，向 6 邻域传播。
        while let Some((x, y, z, level)) = queue.pop_front() {
            if level == 0 {
                continue;
            }
            for (dx, dy, dz) in [
                (1i32, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                let nz = z as i32 + dz;
                // 越出本区块水平边界的，由邻块边界 seed 处理；垂直越界忽略。
                if !(0..16).contains(&nx) || !(0..16).contains(&nz) || ny < 0 || ny >= height as i32
                {
                    continue;
                }
                let nx = nx as usize;
                let ny = ny as usize;
                let nz = nz as usize;
                let n_section = ny / 16;
                let n_y_local = ny % 16;
                let n_flat = ny * 256 + nz * 16 + nx;
                let n_id = blocks.get(n_flat).copied().unwrap_or(0);
                let opacity = registry.light_opacity(n_id);
                // 穿过目标方块：等级衰减 `1 + opacity`，截断到 0。
                let new_level = level.saturating_sub(1).saturating_sub(opacity);
                if new_level == 0 {
                    continue;
                }
                let n_index = light_index(nx, n_y_local, nz);
                if let Some(section_light) = chunk.light_sections_mut().get_mut(n_section)
                    && section_light.block(n_index) < new_level
                {
                    section_light.set_block(n_index, new_level);
                    queue.push_back((nx, ny, nz, new_level));
                }
            }
        }
    }

    /// 天空光：每列自顶向下竖直扫描（顶部以上视为全亮 15），再以邻块边界
    /// 天空光修正接缝。
    fn compute_sky_light_boundary(
        chunk: &mut Chunk,
        boundary: &[SectionLightBoundary; 4],
        blocks: &[u32],
        height: usize,
        registry: &BlockRegistry,
    ) {
        // 竖直列扫描：每列独立，从顶部向下。
        for lx in 0..16 {
            for lz in 0..16 {
                // 区块顶以上视为全亮。
                let mut current: u8 = 15;
                for ly in (0..height).rev() {
                    let section = ly / 16;
                    let y_local = ly % 16;
                    let flat = ly * 256 + lz * 16 + lx;
                    let id = blocks.get(flat).copied().unwrap_or(0);
                    let opacity = registry.light_opacity(id);
                    let index = light_index(lx, y_local, lz);
                    let value = if opacity == 0 {
                        // 透明方块（含空气）：天空光透传。
                        current
                    } else {
                        // 不透明方块：按不透明度衰减，并从此处继续向下衰减。
                        let reduced = current.saturating_sub(opacity);
                        current = reduced;
                        reduced
                    };
                    if let Some(section_light) = chunk.light_sections_mut().get_mut(section) {
                        section_light.set_sky(index, value);
                    }
                }
            }
        }

        // 边界 seed：四周边界格读取邻块对应天空光，取较大值修正接缝。
        for (dir, boundary_item) in boundary.iter().enumerate() {
            for ly in 0..height {
                let section = ly / 16;
                let y_local = ly % 16;
                for coord in 0..16 {
                    let (lx, lz) = match dir {
                        0 => (15, coord),
                        1 => (0, coord),
                        2 => (coord, 15),
                        3 => (coord, 0),
                        _ => continue,
                    };
                    let boundary_idx = ly * 16 + coord;
                    let incoming = boundary_item.sky[boundary_idx];
                    if incoming == 0 {
                        continue;
                    }
                    let index = light_index(lx, y_local, lz);
                    if let Some(section_light) = chunk.light_sections_mut().get_mut(section)
                        && section_light.sky(index) < incoming
                    {
                        section_light.set_sky(index, incoming);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::resource::registries::Registry;

    /// 构造测试用方块注册表：air(0)、stone(1, opacity 默认 15)、
    /// glowstone(2, emission 15)、glass(3, opacity 0)。
    fn test_registry() -> BlockRegistry {
        let toml = r#"
            [[entry]]
            id = 0
            name = "minecraft:air"

            [[entry]]
            id = 1
            name = "minecraft:stone"

            [[entry]]
            id = 2
            name = "minecraft:glowstone"
            light_emission = 15

            [[entry]]
            id = 3
            name = "minecraft:glass"
            light_opacity = 0
        "#;
        BlockRegistry(
            Registry::<crate::resource::registries::BlockDefinition>::from_toml_str(toml).unwrap(),
        )
    }

    /// 取区块内某格天空光（局部坐标）。
    fn sky_at(chunk: &Chunk, lx: usize, ly: usize, lz: usize) -> u8 {
        let section = ly / 16;
        let y_local = ly % 16;
        chunk
            .light_sections()
            .get(section)
            .map(|s| s.sky(light_index(lx, y_local, lz)))
            .unwrap_or(0)
    }

    /// 取区块内某格方块光（局部坐标）。
    fn block_at(chunk: &Chunk, lx: usize, ly: usize, lz: usize) -> u8 {
        let section = ly / 16;
        let y_local = ly % 16;
        chunk
            .light_sections()
            .get(section)
            .map(|s| s.block(light_index(lx, y_local, lz)))
            .unwrap_or(0)
    }

    #[test]
    fn all_air_column_transmits_sky_light() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        let none: Neighbors<'_> = [None, None, None, None];
        LightEngine::recompute(&mut chunk, &none, &registry);
        // 全空气：每格天空光均为 15（顶部全亮，透明透传）。
        assert_eq!(sky_at(&chunk, 5, 0, 5), 15);
        assert_eq!(sky_at(&chunk, 5, 15, 5), 15);
    }

    #[test]
    fn opaque_block_blocks_sky_light_below() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 y=8（局部）放置不透明 stone（id=1）。
        assert!(chunk.set_block(0, light_index(5, 8, 5), 1));
        let none: Neighbors<'_> = [None, None, None, None];
        LightEngine::recompute(&mut chunk, &none, &registry);
        // 不透明方块处天空光被衰减为 0。
        assert_eq!(sky_at(&chunk, 5, 8, 5), 0);
        // 正下方（y=7）空气的天空光被阻断为 0。
        assert_eq!(sky_at(&chunk, 5, 7, 5), 0);
        // 上方（y=9）仍为 15。
        assert_eq!(sky_at(&chunk, 5, 9, 5), 15);
    }

    #[test]
    fn emissive_block_produces_block_light_decay() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 (5,8,5) 放置发光度 15 的 glowstone（id=2）。
        assert!(chunk.set_block(0, light_index(5, 8, 5), 2));
        let none: Neighbors<'_> = [None, None, None, None];
        LightEngine::recompute(&mut chunk, &none, &registry);
        // 发光方块本身亮度 15。
        assert_eq!(block_at(&chunk, 5, 8, 5), 15);
        // 相邻一格衰减为 14（曼哈顿距离 1）。
        assert_eq!(block_at(&chunk, 6, 8, 5), 14);
        assert_eq!(block_at(&chunk, 5, 9, 5), 14);
        assert_eq!(block_at(&chunk, 5, 8, 6), 14);
        // 远离两格衰减为 13。
        assert_eq!(block_at(&chunk, 7, 8, 5), 13);
    }

    #[test]
    fn transparent_glass_passes_sky_light() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 y=8 放置透明 glass（opacity 0）。
        assert!(chunk.set_block(0, light_index(5, 8, 5), 3));
        let none: Neighbors<'_> = [None, None, None, None];
        LightEngine::recompute(&mut chunk, &none, &registry);
        // 透明方块不阻断：上下均为 15。
        assert_eq!(sky_at(&chunk, 5, 8, 5), 15);
        assert_eq!(sky_at(&chunk, 5, 7, 5), 15);
        assert_eq!(sky_at(&chunk, 5, 9, 5), 15);
    }

    #[test]
    fn boundary_light_matches_neighbor() {
        let registry = test_registry();
        // 邻块 A（+x 方向与 B 相邻）全空气，先算（顶部全亮 15）。
        let mut neighbor_a = Chunk::new(1, 0, 1);
        let none: Neighbors<'_> = [None, None, None, None];
        LightEngine::recompute(&mut neighbor_a, &none, &registry);

        // 区块 B 在 (lx=0) 边界处放一个不透明 stone，其东侧（lx=1）为空气。
        let mut chunk_b = Chunk::new(0, 0, 1);
        assert!(chunk_b.set_block(0, light_index(0, 8, 5), 1));
        // B 的东邻是 A。
        let neighbors: Neighbors<'_> = [Some(&neighbor_a), None, None, None];
        LightEngine::recompute(&mut chunk_b, &neighbors, &registry);

        // B 边界（lx=0）不透明方块处 sky=0（被自身方块阻断）。
        assert_eq!(sky_at(&chunk_b, 0, 8, 5), 0);
        // B 的 lx=1（东侧空气格）应接收到来自 A 的边界天空光（A 同位置为 15），
        // 体现「边界光照一致性」接缝修正。
        assert_eq!(sky_at(&chunk_b, 1, 8, 5), 15);
    }

    /// 验证 recompute_with_boundary 与 recompute 产生相同结果。
    #[test]
    fn recompute_with_boundary_matches_recompute() {
        let registry = test_registry();
        let mut chunk_orig = Chunk::new(0, 0, 1);
        assert!(chunk_orig.set_block(0, light_index(5, 8, 5), 2));
        let mut chunk_copy = chunk_orig.clone();

        let none: Neighbors<'_> = [None, None, None, None];
        LightEngine::recompute(&mut chunk_orig, &none, &registry);

        let boundary = LightEngine::extract_boundary(&chunk_copy, &none);
        LightEngine::recompute_with_boundary(&mut chunk_copy, &boundary, &registry);

        assert_eq!(chunk_orig.light_sections(), chunk_copy.light_sections());
    }

    /// 验证 SectionLightBoundary::empty 创建正确长度的边界。
    #[test]
    fn section_light_boundary_empty_length() {
        let boundary = SectionLightBoundary::empty(4);
        assert_eq!(boundary.sky.len(), 4 * 256);
        assert_eq!(boundary.block.len(), 4 * 256);
        assert!(boundary.sky.iter().all(|&v| v == 0));
        assert!(boundary.block.iter().all(|&v| v == 0));
    }

    /// 验证边界数据正确透传：手动构造边界，通过 recompute_with_boundary
    /// 使边界格获得预期光值。
    #[test]
    fn boundary_value_propagates_correctly() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 (lx=1, ly=8, lz=5) 放置空气（透明）。
        assert!(chunk.set_block(0, light_index(1, 8, 5), 0));

        // 手动构造东方向边界：在 ly=8*16+8=136, coord=5 处设置 sky=15。
        let mut boundary = [
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
        ];
        // 东方向（dir=0）：bound_lx=15，coord=lz。
        // 我们要让 chunk 的 lx=15 边界获得 sky=15，但 chunk 只有 1 个 section，
        // lx=15 边界对应 boundary[0] 的 sky[ly*16 + lz]。
        // 测试逻辑：设置 boundary[0].sky[8*16 + 5] = 15，即 ly=8, lz=5 的东边界。
        boundary[0].sky[8 * 16 + 5] = 15;

        LightEngine::recompute_with_boundary(&mut chunk, &boundary, &registry);

        // chunk 的 lx=15, ly=8, lz=5 处应获得 sky=15（来自东边界 seed）。
        assert_eq!(sky_at(&chunk, 15, 8, 5), 15);
    }

    /// 验证西方向边界透传。
    #[test]
    fn west_boundary_value_propagates() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 (lx=0, ly=10, lz=3) 放置不透明 stone，使竖扫天空光为 0。
        assert!(chunk.set_block(0, light_index(0, 10, 3), 1));

        let mut boundary = [
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
        ];
        // 西方向（dir=1）：bound_lx=0，coord=lz。
        // 设置 boundary[1].sky[10*16 + 3] = 12，高于 stone 处的 0。
        boundary[1].sky[10 * 16 + 3] = 12;

        LightEngine::recompute_with_boundary(&mut chunk, &boundary, &registry);

        // chunk 的 lx=0, ly=10, lz=3 处应获得 sky=12（来自西边界 seed）。
        assert_eq!(sky_at(&chunk, 0, 10, 3), 12);
    }

    /// 验证南方向边界透传（方块光）。
    #[test]
    fn south_boundary_block_light_propagates() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 (lx=5, ly=5, lz=15) 放置空气。
        assert!(chunk.set_block(0, light_index(5, 5, 15), 0));

        let mut boundary = [
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
        ];
        // 南方向（dir=2）：bound_lz=15，coord=lx。
        // 设置 boundary[2].block[5*16 + 5] = 10。
        boundary[2].block[5 * 16 + 5] = 10;

        LightEngine::recompute_with_boundary(&mut chunk, &boundary, &registry);

        // chunk 的 lx=5, ly=5, lz=15 处应获得 block=10（来自南边界 seed）。
        assert_eq!(block_at(&chunk, 5, 5, 15), 10);
    }

    /// 验证北方向边界透传。
    #[test]
    fn north_boundary_block_light_propagates() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 (lx=3, ly=12, lz=0) 放置空气。
        assert!(chunk.set_block(0, light_index(3, 12, 0), 0));

        let mut boundary = [
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
        ];
        // 北方向（dir=3）：bound_lz=0，coord=lx。
        // 设置 boundary[3].block[12*16 + 3] = 8。
        boundary[3].block[12 * 16 + 3] = 8;

        LightEngine::recompute_with_boundary(&mut chunk, &boundary, &registry);

        // chunk 的 lx=3, ly=12, lz=0 处应获得 block=8（来自北边界 seed）。
        assert_eq!(block_at(&chunk, 3, 12, 0), 8);
    }

    /// 验证多区段边界长度正确。
    #[test]
    fn multi_section_boundary_length() {
        let boundary = SectionLightBoundary::empty(8);
        assert_eq!(boundary.sky.len(), 8 * 256);
        assert_eq!(boundary.block.len(), 8 * 256);
    }

    /// 验证边界零值不会覆盖已有光照。
    #[test]
    fn zero_boundary_does_not_override_existing() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 1);
        // 在 (lx=15, ly=8, lz=5) 放置空气。
        assert!(chunk.set_block(0, light_index(15, 8, 5), 0));

        // 全零边界。
        let boundary: [SectionLightBoundary; 4] = [
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
            SectionLightBoundary::empty(1),
        ];

        LightEngine::recompute_with_boundary(&mut chunk, &boundary, &registry);

        // 全空气列，天空光应为 15。
        assert_eq!(sky_at(&chunk, 15, 8, 5), 15);
    }
}
