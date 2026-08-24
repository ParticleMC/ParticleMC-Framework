//! 高度图：每个区块列的「最高实心方块」高度表。
//!
//! 语义对齐 Minestom Java `Heightmap` / `MotionBlockingHeightmap`：对每列
//! `(x, z)` 自顶向下扫描，返回首个满足判定谓词的方块所在全局 y。当前提供
//! MOTION_BLOCKING 一种类型（实心判定由调用方闭包注入），供区块序列化
//! （[`crate::instance::chunk_serializer`]）与后续地形查询复用。
//!
//! 内部以 256 项 `u16` 数组存储；索引约定与区块线性布局一致：`z * 16 + x`
//! （z 占高 4 位、x 占低 4 位）。高度值为区块内自底向上的全局 y，
//! 整列无实心方块时返回 0（区块底部之下）。
//!
//! 变更标识符：`complete-missing-subsystems`（T9/R9）。

use crate::instance::chunk::Chunk;

/// 区块每列数（16×16 = 256）。
pub const HEIGHTMAP_COLUMNS: usize = 16 * 16;
/// 区块边长（16）。
const BLOCK_DIMENSION: usize = 16;

/// 区块高度图：256 项列高度，索引 `z * 16 + x`。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Heightmap {
    /// 各列最高实心方块的全局 y（区块内自底向上），恒含 256 项。
    pub heights: Vec<u16>,
}

impl Heightmap {
    /// 以 256 项全 0 高度构造空高度图。
    pub fn new() -> Self {
        Self {
            heights: vec![0; HEIGHTMAP_COLUMNS],
        }
    }

    /// 读取某列高度（区块局部坐标）。
    ///
    /// `x` / `z` 超出 `[0, 16)` 时 clamp 到最近边界列（而非返回 0），
    /// 保证边界读请求仍返回该方向最外层列的真实高度。
    pub fn get_height(&self, x: u8, z: u8) -> u16 {
        let cx = usize::from(x.min(15));
        let cz = usize::from(z.min(15));
        self.heights
            .get(cz * BLOCK_DIMENSION + cx)
            .copied()
            .unwrap_or(0)
    }
}

/// 构建 MOTION_BLOCKING 高度图：对每列从顶向下找首个实心方块，记录其全局 y。
///
/// `is_solid` 为实心判定谓词（如经注册表判定「非空气」），调用方按语义注入。
/// 整列无实心方块时该列高度为 0，与 Minestom 高度图「未找到落回底部」一致。
pub fn build_motion_blocking(chunk: &Chunk, is_solid: impl Fn(u32) -> bool) -> Heightmap {
    let mut heights = vec![0u16; HEIGHTMAP_COLUMNS];
    for z in 0..BLOCK_DIMENSION {
        for x in 0..BLOCK_DIMENSION {
            let column = column_height(chunk, x, z, &is_solid);
            if let Some(slot) = heights.get_mut(z * BLOCK_DIMENSION + x) {
                *slot = column;
            }
        }
    }
    Heightmap { heights }
}

/// 计算某一列 `(x, z)` 的最高实心方块高度（区块内自底向上的全局 y）。
///
/// 自顶向下扫描区段与局部 y，首个满足 `is_solid` 的方块即该列最高点；
/// 无实心方块时返回 0。
fn column_height(chunk: &Chunk, x: usize, z: usize, is_solid: &impl Fn(u32) -> bool) -> u16 {
    for section_index in (0..chunk.sections.len()).rev() {
        let Some(section) = chunk.sections.get(section_index) else {
            break; // 理论不可达：Chunk 保证至少一个区段
        };
        for local_y in (0..BLOCK_DIMENSION).rev() {
            // 区段内线性索引：y 占高 8 位、z 占中间 4 位、x 占低 4 位。
            let index = (local_y << 8) | (z << 4) | x;
            if is_solid(section.get_block_id(index)) {
                return u16::try_from(section_index * BLOCK_DIMENSION + local_y).unwrap_or(0);
            }
        }
    }
    0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 实心判定：非空气（id != 0）即实心。
    fn is_solid(id: u32) -> bool {
        id != 0
    }

    /// 构造含若干实心方块的测试区块：3 个区段覆盖 y ∈ [0, 47]。
    fn test_chunk() -> Chunk {
        let mut chunk = Chunk::new(1, -2, 3);
        // 列 (1, 2)：section 0 内 y=5、y=10 均为实心，取最高 10。
        assert!(chunk.set_block(0, (5 << 8) | (2 << 4) | 1, 1));
        assert!(chunk.set_block(0, (10 << 8) | (2 << 4) | 1, 2));
        // 列 (5, 4)：section 1 内 y=3，全局高度 16 + 3 = 19。
        assert!(chunk.set_block(1, (3 << 8) | (4 << 4) | 5, 1));
        // 列 (0, 0)：section 0 内 y=0 实心，全局高度 0。
        assert!(chunk.set_block(0, 0, 1));
        chunk
    }

    #[test]
    fn build_motion_blocking_finds_highest_solid_per_column() {
        let chunk = test_chunk();
        let map = build_motion_blocking(&chunk, is_solid);
        assert_eq!(map.heights.len(), HEIGHTMAP_COLUMNS);
        assert_eq!(map.get_height(1, 2), 10);
        assert_eq!(map.get_height(5, 4), 19);
        assert_eq!(map.get_height(0, 0), 0);
    }

    #[test]
    fn air_column_degrades_to_zero() {
        // 全空区块：所有列高度均为 0。
        let chunk = Chunk::new(0, 0, 2);
        let map = build_motion_blocking(&chunk, is_solid);
        assert!(map.heights.iter().all(|&h| h == 0));
    }

    #[test]
    fn get_height_clamps_out_of_bounds_to_edge() {
        let chunk = test_chunk();
        let map = build_motion_blocking(&chunk, is_solid);
        // 越界坐标 clamp 到最近边界：x=200→15、z=200→15 取列 (15,15)。
        assert_eq!(map.get_height(200, 200), map.get_height(15, 15));
        // 边界内正常读取不受影响。
        assert_eq!(map.get_height(15, 15), 0);
        // 索引约定：heights[z*16+x] 与 get_height 一致。
        assert_eq!(map.heights.get(2 * 16 + 1).copied().unwrap_or(0), 10);
    }

    #[test]
    fn solid_column_spans_sections() {
        // 实心方块跨区段：section 1 顶部，验证全局 y 累计。
        let mut chunk = Chunk::new(0, 0, 3);
        assert!(chunk.set_block(2, (15 << 8) | (7 << 4) | 9, 1)); // 全局 y = 32 + 15
        let map = build_motion_blocking(&chunk, is_solid);
        assert_eq!(map.get_height(9, 7), 47);
    }
}
