//! 坐标值类型：`Vec` / `BlockVec` / `Area` / `ChunkRange`。
//!
//! 语义对齐 Java Minestom 的 `net.minestom.server.coordinate` 包：
//! [`Vec`] 为双精度 3D 向量（长度 / 归一化 / 点积 / 叉积），[`BlockVec`] 为
//! 整数方块坐标（floor 转换 / chunk 坐标），[`Area`] 为含边界的方块区域
//! （contains / 迭代），[`ChunkRange`] 为区块坐标范围迭代。仅承载纯值语义，
//! 不复制 Java 的 `Point` 接口体系与旋转等浮点扩展。
//!
//! 变更标识符：`complete-missing-subsystems`（R15 utils/coordinate/thread 工具层）。
//! 见 `.specs/complete-missing-subsystems/spec.md`。

use crate::component::Position;

/// 3D 双精度向量。
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Vec {
    /// X 分量。
    pub x: f64,
    /// Y 分量。
    pub y: f64,
    /// Z 分量。
    pub z: f64,
}

impl Vec {
    /// 零向量。
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// 以三个分量构造向量。
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// 向量长度（欧几里得范数）。
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    /// 向量长度的平方（避免开方，用于距离比较）。
    pub fn length_squared(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// 归一化：返回方向相同、长度为 1 的单位向量。
    ///
    /// 零向量（长度为零）时返回零向量本身，避免除零产生 NaN。
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len == 0.0 {
            Self::ZERO
        } else {
            Self {
                x: self.x / len,
                y: self.y / len,
                z: self.z / len,
            }
        }
    }

    /// 与另一向量的点积（标量投影）。
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// 与另一向量的叉积（右手定则），结果垂直于二者所在平面。
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - other.y * self.z,
            y: self.z * other.x - other.z * self.x,
            z: self.x * other.y - other.x * self.y,
        }
    }

    /// 各分量乘以标量（缩放），返回新向量。
    pub fn scale(self, factor: f64) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
            z: self.z * factor,
        }
    }
}

/// 向量加法：`add` 以 `+` / `Add::add` 表达（对应任务 API 的 `add`）。
impl std::ops::Add for Vec {
    type Output = Self;

    /// 分量相加，返回新向量。
    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
            z: self.z + other.z,
        }
    }
}

/// 向量减法：`sub` 以 `-` / `Sub::sub` 表达（对应任务 API 的 `sub`）。
impl std::ops::Sub for Vec {
    type Output = Self;

    /// 分量相减，返回新向量。
    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }
}

/// 把 double 坐标向下取整到 `i32` 方块坐标；超出 `i32` 范围时钳制到边界。
///
/// 遵循项目章程：`f64 → i32` 一律经 `floor` + `TryFrom`，溢出分支显式钳制，
/// 不做裸缩窄转换。
fn floor_to_i32(v: f64) -> i32 {
    let floored = v.floor();
    // `i32` 无 `TryFrom<f64>` 实现（Rust std 仅提供截断语义的 `as`），
    // 故先把值钳制到 `i32` 可表示闭区间再 `as` 转换——钳制后该转换必然
    // 无损安全（区间内整数在 f64 中精确可表），不构成裸缩窄截断。
    if floored >= f64::from(i32::MAX) {
        i32::MAX
    } else if floored <= f64::from(i32::MIN) {
        i32::MIN
    } else {
        floored as i32
    }
}

/// 整数方块坐标（x/y/z 均为 `i32`）。
///
/// 从浮点构造时向下取整（对齐 Java `globalToBlock` 语义）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockVec {
    /// 方块 X 坐标。
    pub x: i32,
    /// 方块 Y 坐标。
    pub y: i32,
    /// 方块 Z 坐标。
    pub z: i32,
}

impl BlockVec {
    /// 原点方块。
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };

    /// 以整数坐标构造方块向量。
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// 从双精度向量构造：各分量向下取整。
    ///
    /// `vec` 超出 `i32` 表示范围时钳制到边界（正常世界坐标远小于该范围）。
    pub fn from_vec(vec: &Vec) -> Self {
        Self {
            x: floor_to_i32(vec.x),
            y: floor_to_i32(vec.y),
            z: floor_to_i32(vec.z),
        }
    }

    /// 转换为双精度向量。
    pub fn to_vec(self) -> Vec {
        Vec {
            x: f64::from(self.x),
            y: f64::from(self.y),
            z: f64::from(self.z),
        }
    }

    /// 所在区块 X 坐标（`x >> 4`，对负数亦向下取整）。
    pub fn chunk_x(self) -> i32 {
        self.x >> 4
    }

    /// 所在区块 Z 坐标（`z >> 4`，对负数亦向下取整）。
    pub fn chunk_z(self) -> i32 {
        self.z >> 4
    }

    /// 转换为 [`Position`]（零朝向）。
    pub fn as_position(self) -> Position {
        Position::new(f64::from(self.x), f64::from(self.y), f64::from(self.z))
    }

    /// 各分量偏移后返回新方块向量。
    pub fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            z: self.z + dz,
        }
    }

    /// 三轴坐标是否完全相同。
    pub fn same_point(self, other: Self) -> bool {
        self.x == other.x && self.y == other.y && self.z == other.z
    }
}

/// 含边界的方块区域：`[min, max]` 之间的全部方块坐标（含端点）。
///
/// `new` 自动归一化：`min` / `max` 逐轴取极小 / 极大值，调用方无需保证传入顺序。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Area {
    /// 最小角点（含）。
    pub min: BlockVec,
    /// 最大角点（含）。
    pub max: BlockVec,
}

impl Area {
    /// 以两个角点构造区域，自动归一化 `min` / `max`。
    pub fn new(a: BlockVec, b: BlockVec) -> Self {
        Self {
            min: BlockVec::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)),
            max: BlockVec::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)),
        }
    }

    /// 指定方块坐标是否在区域内（含边界）。
    pub fn contains(&self, pos: BlockVec) -> bool {
        pos.x >= self.min.x
            && pos.x <= self.max.x
            && pos.y >= self.min.y
            && pos.y <= self.max.y
            && pos.z >= self.min.z
            && pos.z <= self.max.z
    }

    /// 迭代区域内全部方块坐标（含边界），顺序为 x → y → z（x 最快）。
    ///
    /// 对应 Java `Area.Cuboid` 迭代语义。经 [`Area::new`] 构造时 `min <= max`
    /// 恒成立；若直接字段构造出 `min.z > max.z` 的空区域，则不产出任何项。
    pub fn iter(&self) -> AreaIter {
        AreaIter {
            min: self.min,
            max: self.max,
            x: self.min.x,
            y: self.min.y,
            z: self.min.z,
        }
    }
}

/// [`Area`] 的迭代器：按 x → y → z 顺序产出区域内全部方块坐标。
#[derive(Copy, Clone, Debug)]
pub struct AreaIter {
    min: BlockVec,
    max: BlockVec,
    x: i32,
    y: i32,
    z: i32,
}

impl Iterator for AreaIter {
    type Item = BlockVec;

    fn next(&mut self) -> Option<Self::Item> {
        // `min` 恒有 `min <= max`（`Area::new` 归一化），因此只要 z 越界即结束。
        if self.z > self.max.z {
            return None;
        }
        let item = BlockVec::new(self.x, self.y, self.z);
        // 推进游标：先 x，再 y，最后 z。
        if self.x < self.max.x {
            self.x += 1;
        } else if self.y < self.max.y {
            self.x = self.min.x;
            self.y += 1;
        } else if self.z < self.max.z {
            self.x = self.min.x;
            self.y = self.min.y;
            self.z += 1;
        } else {
            // 已产出最后一个点（max）：置 z 越界使迭代终止。
            self.z += 1;
        }
        Some(item)
    }
}

/// 区块坐标范围：`[min_chunk_x, max_chunk_x] × [min_chunk_z, max_chunk_z]`（含边界）。
///
/// 仅承载 x / z 两维区块坐标迭代，对应 Java `ChunkRange` 的「按范围遍历区块」
/// 语义；Java 的螺旋遍历与「差异区块」回调不在此范围（真实区块分发由实例层负责）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ChunkRange {
    /// 最小区块 X。
    pub min_chunk_x: i32,
    /// 最小区块 Z。
    pub min_chunk_z: i32,
    /// 最大区块 X。
    pub max_chunk_x: i32,
    /// 最大区块 Z。
    pub max_chunk_z: i32,
}

impl ChunkRange {
    /// 以两个区块角点构造范围，自动归一化各轴。
    pub fn new(min_chunk_x: i32, min_chunk_z: i32, max_chunk_x: i32, max_chunk_z: i32) -> Self {
        Self {
            min_chunk_x: min_chunk_x.min(max_chunk_x),
            min_chunk_z: min_chunk_z.min(max_chunk_z),
            max_chunk_x: min_chunk_x.max(max_chunk_x),
            max_chunk_z: min_chunk_z.max(max_chunk_z),
        }
    }

    /// 区块坐标 `(chunk_x, chunk_z)` 是否在范围内（含边界）。
    pub fn contains(&self, chunk_x: i32, chunk_z: i32) -> bool {
        chunk_x >= self.min_chunk_x
            && chunk_x <= self.max_chunk_x
            && chunk_z >= self.min_chunk_z
            && chunk_z <= self.max_chunk_z
    }

    /// 迭代范围内全部区块坐标 `(chunk_x, chunk_z)`（x 最快，含边界）。
    pub fn iter(&self) -> ChunkRangeIter {
        ChunkRangeIter {
            min_x: self.min_chunk_x,
            max_x: self.max_chunk_x,
            max_z: self.max_chunk_z,
            x: self.min_chunk_x,
            z: self.min_chunk_z,
        }
    }
}

/// [`ChunkRange`] 的迭代器：按 x → z 顺序产出全部区块坐标。
///
/// `z` 在迭代中只会单调递增（`x` 触顶后换行），无需记录 `min_z`。
#[derive(Copy, Clone, Debug)]
pub struct ChunkRangeIter {
    min_x: i32,
    max_x: i32,
    max_z: i32,
    x: i32,
    z: i32,
}

impl Iterator for ChunkRangeIter {
    type Item = (i32, i32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.z > self.max_z {
            return None;
        }
        let item = (self.x, self.z);
        if self.x < self.max_x {
            self.x += 1;
        } else if self.z < self.max_z {
            self.x = self.min_x;
            self.z += 1;
        } else {
            self.z += 1;
        }
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    // 显式导入并把 3D 向量改名为 `V3`，避免与测试中常用的 std `Vec` 集合类型
    // 发生名称冲突（glob 导入的优先级高于 prelude）。
    use super::{Area, BlockVec, ChunkRange, Vec as V3};
    // `add`/`sub` 经 `std::ops` trait 暴露，测试中需将其导入作用域。
    use std::ops::{Add, Sub};

    const EPS: f64 = 1e-9;

    fn assert_vec_close(a: V3, b: V3) {
        assert!((a.x - b.x).abs() < EPS);
        assert!((a.y - b.y).abs() < EPS);
        assert!((a.z - b.z).abs() < EPS);
    }

    #[test]
    fn vec_length_and_length_squared() {
        let v = V3::new(3.0, 4.0, 0.0);
        assert!((v.length_squared() - 25.0).abs() < EPS);
        assert!((v.length() - 5.0).abs() < EPS);
        assert!(V3::ZERO.length() < EPS);
    }

    #[test]
    fn vec_normalized_unit_length() {
        let v = V3::new(3.0, 0.0, 4.0);
        assert_vec_close(v.normalized(), V3::new(0.6, 0.0, 0.8));
        assert!((v.normalized().length() - 1.0).abs() < EPS);
    }

    #[test]
    fn vec_normalized_zero_vector_returns_zero() {
        // 零向量归一化返回零向量本身（文档契约），避免 NaN。
        assert_vec_close(V3::ZERO.normalized(), V3::ZERO);
        assert!(V3::ZERO.normalized().length() < EPS);
    }

    #[test]
    fn vec_dot_product() {
        let a = V3::new(1.0, 2.0, 3.0);
        let b = V3::new(-4.0, 5.0, 6.0);
        // 1*(-4) + 2*5 + 3*6 = 24
        assert!((a.dot(b) - 24.0).abs() < EPS);
        // 正交向量点积为零。
        assert!((V3::new(1.0, 0.0, 0.0).dot(V3::new(0.0, 1.0, 0.0))).abs() < EPS);
    }

    #[test]
    fn vec_cross_product() {
        let a = V3::new(1.0, 0.0, 0.0);
        let b = V3::new(0.0, 1.0, 0.0);
        assert_vec_close(a.cross(b), V3::new(0.0, 0.0, 1.0));
        // 叉积结果垂直于两输入向量。
        let r = V3::new(2.0, 3.0, 4.0).cross(V3::new(5.0, 6.0, 7.0));
        assert!((V3::new(2.0, 3.0, 4.0).dot(r)).abs() < EPS);
        assert!((V3::new(5.0, 6.0, 7.0).dot(r)).abs() < EPS);
        // 自叉积为零。
        assert_vec_close(a.cross(a), V3::ZERO);
    }

    #[test]
    fn vec_add_sub_scale() {
        let a = V3::new(1.0, 2.0, 3.0);
        let b = V3::new(4.0, 5.0, 6.0);
        assert_vec_close(a.add(b), V3::new(5.0, 7.0, 9.0));
        assert_vec_close(a.sub(b), V3::new(-3.0, -3.0, -3.0));
        assert_vec_close(a.scale(2.0), V3::new(2.0, 4.0, 6.0));
        // 原始向量不被修改。
        assert_vec_close(a, V3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn block_vec_from_vec_floors() {
        assert_eq!(
            BlockVec::from_vec(&V3::new(2.9, -1.7, 0.1)),
            BlockVec::new(2, -2, 0)
        );
        // 正负边界取整一致性。
        assert_eq!(
            BlockVec::from_vec(&V3::new(-0.0, 5.5, -4.0)),
            BlockVec::new(0, 5, -4)
        );
    }

    #[test]
    fn block_vec_to_vec_and_roundtrip() {
        let bv = BlockVec::new(10, -20, 30);
        assert_eq!(bv.to_vec(), V3::new(10.0, -20.0, 30.0));
        // 整数向量 roundtrip 无损。
        assert_eq!(BlockVec::from_vec(&bv.to_vec()), bv);
    }

    #[test]
    fn block_vec_chunk_coordinates() {
        let pos = BlockVec::new(15, 0, 16);
        assert_eq!(pos.chunk_x(), 0);
        assert_eq!(pos.chunk_z(), 1);
        // 负坐标：-1 >> 4 = -1（向下取整），与 Java `>>` 一致。
        let neg = BlockVec::new(-1, 0, -16);
        assert_eq!(neg.chunk_x(), -1);
        assert_eq!(neg.chunk_z(), -1);
        assert_eq!(BlockVec::new(-17, 0, 0).chunk_x(), -2);
    }

    #[test]
    fn block_vec_as_position() {
        let p = BlockVec::new(7, 64, -9).as_position();
        assert_eq!(p.x, 7.0);
        assert_eq!(p.y, 64.0);
        assert_eq!(p.z, -9.0);
        assert_eq!(p.yaw, 0.0);
        assert_eq!(p.pitch, 0.0);
    }

    #[test]
    fn block_vec_offset_and_same_point() {
        assert_eq!(
            BlockVec::new(1, 2, 3).offset(1, -1, 0),
            BlockVec::new(2, 1, 3)
        );
        assert!(BlockVec::new(1, 2, 3).same_point(BlockVec::new(1, 2, 3)));
        assert!(!BlockVec::new(1, 2, 3).same_point(BlockVec::new(1, 2, 4)));
    }

    #[test]
    fn area_new_normalizes_min_max() {
        let a = Area::new(BlockVec::new(2, 3, 4), BlockVec::new(0, 1, 6));
        assert_eq!(a.min, BlockVec::new(0, 1, 4));
        assert_eq!(a.max, BlockVec::new(2, 3, 6));
    }

    #[test]
    fn area_contains_boundary_and_outside() {
        let area = Area::new(BlockVec::new(0, 0, 0), BlockVec::new(2, 2, 2));
        assert!(area.contains(BlockVec::new(0, 0, 0)));
        assert!(area.contains(BlockVec::new(2, 2, 2)));
        assert!(area.contains(BlockVec::new(1, 1, 1)));
        assert!(!area.contains(BlockVec::new(3, 1, 1)));
        assert!(!area.contains(BlockVec::new(1, 1, -1)));
    }

    #[test]
    fn area_iter_yields_9_points_inclusive() {
        // 2×2×2 平面（z 固定）区域迭代 9 点，含边界。
        let area = Area::new(BlockVec::new(0, 0, 0), BlockVec::new(2, 2, 0));
        let points: Vec<BlockVec> = area.iter().collect();
        assert_eq!(points.len(), 9);
        // x 最快、随后 y：期望序列首尾与唯一性。
        assert_eq!(points[0], BlockVec::new(0, 0, 0));
        assert_eq!(points[8], BlockVec::new(2, 2, 0));
        let mut unique = points.clone();
        unique.sort_by_key(|p| (p.z, p.y, p.x));
        unique.dedup();
        assert_eq!(unique.len(), 9);
        // 全部点在区域内。
        assert!(points.iter().all(|p| area.contains(*p)));
    }

    #[test]
    fn area_iter_cuboid_3d_volume() {
        // 2×3×4 区域体积 = 2*3*4 = 24。
        let area = Area::new(BlockVec::new(0, 0, 0), BlockVec::new(1, 2, 3));
        assert_eq!(area.iter().count(), 24);
        // 负坐标区域：2×3×3 = 18。
        let neg = Area::new(BlockVec::new(-2, -1, -1), BlockVec::new(-1, 1, 1));
        assert_eq!(neg.iter().count(), 18);
    }

    #[test]
    fn chunk_range_contains_and_iter() {
        let range = ChunkRange::new(0, 0, 2, 1);
        assert!(range.contains(0, 0));
        assert!(range.contains(2, 1));
        assert!(!range.contains(3, 0));
        assert!(!range.contains(0, 2));
        // 3×2 = 6 个区块坐标，含边界，x 最快。
        let coords: Vec<(i32, i32)> = range.iter().collect();
        assert_eq!(coords.len(), 6);
        assert_eq!(coords[0], (0, 0));
        assert_eq!(coords[3], (0, 1));
        assert_eq!(coords[5], (2, 1));
        // 归一化：逆序传入角点仍得到相同集合。
        let flipped = ChunkRange::new(2, 1, 0, 0);
        let mut a: Vec<_> = range.iter().collect();
        let mut b: Vec<_> = flipped.iter().collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }
}
