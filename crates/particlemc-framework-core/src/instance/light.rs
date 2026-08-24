//! 光照系统（简化 API 骨架）。
//!
//! 提供按坐标设置/读取天空光照（`sky_light`）与方块光照（`block_light`）的
//! 基础容器，容量为一个区段（16×16×16 = 4096），索引约定与区块线性布局一致：
//! `(y << 8) | (z << 4) | x`。
//!
//! **注意（v1 限制）**：本模块仅为存储骨架，**不实现真实的光照传播算法**
//! （Java Minestom `LightEngine` / `SkyLight.calculateInternal` 的逐列扫描与
//! 传播）。写入值即存储值，读取原样返回；光照重算/扩散由后续批次按
//! LightEngine 语义补齐。
//!
//! 变更标识符：`complete-missing-subsystems`（T9/R9）。

/// 区段边长（16）。
const BLOCK_DIMENSION: usize = 16;
/// 单区段方块数（16×16×16 = 4096）。
const SECTION_VOLUME: usize = BLOCK_DIMENSION * BLOCK_DIMENSION * BLOCK_DIMENSION;
/// Minecraft 光照等级上限（0..=15）。
const MAX_LIGHT_LEVEL: u8 = 15;

/// 光照系统：天空光与方块光两个独立存储层。
///
/// 两数组各含 4096 项，初始全 0（未填充满）。写入越界坐标被忽略（返回
/// `false`），读取越界返回 0，保证调用方无需前置边界检查。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightSystem {
    /// 天空光照等级（0..=15），索引 `(y << 8) | (z << 4) | x`。
    pub sky_light: Vec<u8>,
    /// 方块光照等级（0..=15），索引 `(y << 8) | (z << 4) | x`。
    pub block_light: Vec<u8>,
}

impl Default for LightSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl LightSystem {
    /// 构造全 0 光照系统（两存储层均未填充）。
    pub fn new() -> Self {
        Self {
            sky_light: vec![0; SECTION_VOLUME],
            block_light: vec![0; SECTION_VOLUME],
        }
    }

    /// 设置某坐标的天空光照等级。
    ///
    /// `level` 会被截断到 `0..=15`（超出部分取 15）；`x` / `y` / `z` 任一
    /// 越界 `[0, 16)` 时忽略写入并返回 `false`。
    pub fn set_sky_light(&mut self, x: u8, y: u8, z: u8, level: u8) -> bool {
        Self::write(&mut self.sky_light, x, y, z, level)
    }

    /// 读取某坐标的天空光照等级；越界返回 0。
    pub fn get_sky_light(&self, x: u8, y: u8, z: u8) -> u8 {
        Self::read(&self.sky_light, x, y, z)
    }

    /// 设置某坐标的方块光照等级（截断到 `0..=15`，越界忽略）。
    pub fn set_block_light(&mut self, x: u8, y: u8, z: u8, level: u8) -> bool {
        Self::write(&mut self.block_light, x, y, z, level)
    }

    /// 读取某坐标的方块光照等级；越界返回 0。
    pub fn get_block_light(&self, x: u8, y: u8, z: u8) -> u8 {
        Self::read(&self.block_light, x, y, z)
    }

    /// 按索引写入（共用实现：越界返回 `false`，等级截断到 15）。
    fn write(layer: &mut [u8], x: u8, y: u8, z: u8, level: u8) -> bool {
        let index = Self::index_of(x, y, z);
        let Some(slot) = layer.get_mut(index) else {
            return false;
        };
        *slot = level.min(MAX_LIGHT_LEVEL);
        true
    }

    /// 按索引读取（共用实现：越界返回 0）。
    fn read(layer: &[u8], x: u8, y: u8, z: u8) -> u8 {
        let index = Self::index_of(x, y, z);
        layer.get(index).copied().unwrap_or(0)
    }

    /// 坐标 → 线性索引；任一座标越界返回越界索引（供读写共同判定）。
    fn index_of(x: u8, y: u8, z: u8) -> usize {
        if x >= BLOCK_DIMENSION as u8 || y >= BLOCK_DIMENSION as u8 || z >= BLOCK_DIMENSION as u8 {
            return usize::MAX;
        }
        (usize::from(y) << 8) | (usize::from(z) << 4) | usize::from(x)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_initializes_both_layers_to_zero() {
        let system = LightSystem::new();
        assert_eq!(system.sky_light.len(), SECTION_VOLUME);
        assert_eq!(system.block_light.len(), SECTION_VOLUME);
        assert!(system.sky_light.iter().all(|&v| v == 0));
        assert!(system.block_light.iter().all(|&v| v == 0));
    }

    #[test]
    fn sky_light_roundtrip() {
        let mut system = LightSystem::new();
        assert!(system.set_sky_light(3, 4, 5, 12));
        assert_eq!(system.get_sky_light(3, 4, 5), 12);
        // 其余位置不受影响。
        assert_eq!(system.get_sky_light(3, 4, 6), 0);
    }

    #[test]
    fn block_light_roundtrip_and_independent_layers() {
        let mut system = LightSystem::new();
        assert!(system.set_block_light(1, 2, 3, 9));
        assert_eq!(system.get_block_light(1, 2, 3), 9);
        // 天空光层独立，不被方块光写入污染。
        assert_eq!(system.get_sky_light(1, 2, 3), 0);
    }

    #[test]
    fn level_is_clamped_to_fifteen() {
        let mut system = LightSystem::new();
        assert!(system.set_sky_light(0, 0, 0, 255));
        assert_eq!(system.get_sky_light(0, 0, 0), MAX_LIGHT_LEVEL);
        assert!(system.set_block_light(0, 0, 0, 7));
        assert_eq!(system.get_block_light(0, 0, 0), 7);
    }

    #[test]
    fn out_of_bounds_write_is_rejected_and_read_returns_zero() {
        let mut system = LightSystem::new();
        // 各轴越界均拒绝写入。
        assert!(!system.set_sky_light(16, 0, 0, 5));
        assert!(!system.set_sky_light(0, 16, 0, 5));
        assert!(!system.set_sky_light(0, 0, 16, 5));
        // 越界读取返回 0，不 panic。
        assert_eq!(system.get_sky_light(16, 0, 0), 0);
        assert_eq!(system.get_block_light(0, 255, 0), 0);
    }

    #[test]
    fn max_corner_coordinate_roundtrips() {
        let mut system = LightSystem::new();
        assert!(system.set_sky_light(15, 15, 15, 14));
        assert_eq!(system.get_sky_light(15, 15, 15), 14);
    }
}
