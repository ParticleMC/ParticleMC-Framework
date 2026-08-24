// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 区块生成器：`ChunkGenerator` trait + 内存中的生成结果表示。
//!
//! 语义对齐 Minestom Java 的 `Generator`（`generate` 产出区块数据，随后才由
//! 世界容器入库），但**不复制 Java 实现**：这里以纯数据 [`GeneratedChunk`]
//! 承载一次生成的输出——`blocks` 为 16×16×16 = 4096 个方块状态 id，
//! `biomes` 为 4×4×4 = 64 个生物群系 id（v1 恒为 0）。
//!
//! 变更标识符：`complete-missing-subsystems`（R8 ChunkGenerator 接口 + 内存快照存档）。
//! 见 `.specs/complete-missing-subsystems/spec.md`。

use super::chunk::{Chunk, SECTION_VOLUME, Section};

// ── Perlin 噪声（内联实现，无外部依赖）───────────────────────────────────────
//
// 采用经典 2D Perlin noise：整数格点上的固定梯度向量 + 平滑衰减函数 + 双线性
// 插值。多个 octave 叠加产生更自然的起伏地形。实现遵循 Ken Perlin 1985 年原始
// 算法，仅做适定点数与位宽的调整以适配区块生成场景。
//
// 块 ID 约定（对应最小注册表）：0 = air，1 = stone。
const PERLIN_GRADIENTS: [(i32, i32); 12] = [
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (-1, 1),
    (1, -1),
    (-1, -1),
];

/// 构造确定性置换表（基于给定种子）。
fn make_perm(seed: u64) -> [u8; 256] {
    let mut perm = [0u8; 256];
    // Fisher-Yates shuffle，混合 64 位种子。
    (0..256).for_each(|i| {
        perm[i] = i as u8;
    });
    let mut state = seed;
    for i in (1..256).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = ((state >> 56) as u8 & 0x7f) as usize;
        let idx = (i + j) % 256;
        perm.swap(i, idx);
    }
    perm
}

/// 平滑衰减函数：-6t^5 + 15t^4 + 10t^3（Perlin 推荐的多项式）。
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// 线性插值。
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + t * (b - a)
}

/// 计算点 (x, y) 在当前置换表下的梯度点积值。
fn grad(perm: &[u8; 256], px: i32, py: i32, x: f64, y: f64) -> f64 {
    let hash = (perm[(px as usize & 0xff) ^ (py as usize & 0xff)]) as usize;
    let (gx, gy) = PERLIN_GRADIENTS[hash % 12];
    gx as f64 * x + gy as f64 * y
}

/// 2D Perlin 噪声（单 octave）。
fn perlin_single(perm: &[u8; 256], x: f64, y: f64) -> f64 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = x - xi as f64;
    let yf = y - yi as f64;
    let u = fade(xf);
    let v = fade(yf);
    let aa = grad(perm, xi, yi, xf, yf);
    let ba = grad(perm, xi + 1, yi, xf - 1.0, yf);
    let ab = grad(perm, xi, yi + 1, xf, yf - 1.0);
    let bb = grad(perm, xi + 1, yi + 1, xf - 1.0, yf - 1.0);
    lerp(lerp(aa, ba, u), lerp(ab, bb, u), v)
}

/// 多 octave Perlin 噪声，返回 [-1, 1] 范围内的值。
fn perlin_noise(perm: &[u8; 256], x: f64, y: f64, octaves: u32, persistence: f64) -> f64 {
    let mut total = 0.0f64;
    let mut amplitude = 1.0f64;
    let mut frequency = 1.0f64;
    let mut max_value = 0.0f64;
    for _ in 0..octaves {
        total += perlin_single(perm, x * frequency, y * frequency) * amplitude;
        max_value += amplitude;
        amplitude *= persistence;
        frequency *= 2.0;
    }
    if max_value == 0.0 {
        return 0.0;
    }
    total / max_value
}

// ── 生成器定义 ────────────────────────────────────────────────────────────────

/// 生物群系格容量（4×4×4 = 64）。
pub const BIOME_VOLUME: usize = 4 * 4 * 4;

/// 区块生成器：根据区块坐标与实例种子产出一份生成数据。
///
/// 实现必须 `Send + Sync`，以便在并发 tick / 预生成场景下被世界容器持有并调用。
pub trait ChunkGenerator: Send + Sync {
    /// 生成指定坐标区块的方块与生物群系数据。
    ///
    /// - `x` / `z`：区块坐标（非方块坐标）；
    /// - `seed`：实例级种子，确定性生成器应保证同一 (x, z, seed) 产出相同结果。
    fn generate(&self, x: i32, z: i32, seed: u64) -> GeneratedChunk;
}

/// 一次生成的结果：方块 id 数组 + 生物群系 id 数组。
///
/// `blocks` 长度恒为 4096（16×16×16），索引映射为 `y * 256 + z * 16 + x`，
/// 即 y 每层 256 格、每层内按 (z, x) 行主序排列。`biomes` 长度恒为 64
/// （4×4×4），v1 暂不使用（生成器可填 0）。v1 仅承载一层区段
/// （y ∈ [0, 16)），更高层的多层生成留给后续批次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedChunk {
    /// 方块状态 id，长度 4096，索引 = `y * 256 + z * 16 + x`。
    pub blocks: Vec<u32>,
    /// 生物群系 id，长度 64（4×4×4），v1 恒为 0。
    pub biomes: Vec<u32>,
}

impl GeneratedChunk {
    /// 构造全空气生成结果（`blocks` 全 0、`biomes` 全 0）。
    pub fn empty() -> Self {
        Self {
            blocks: vec![0; SECTION_VOLUME],
            biomes: vec![0; BIOME_VOLUME],
        }
    }

    /// 从既有区段提取生成结果（仅取方块内容，生物群系按 v1 填 0）。
    pub fn from_section(section: &Section) -> Self {
        let blocks = (0..SECTION_VOLUME)
            .map(|index| section.get_block_id(index))
            .collect();
        Self {
            blocks,
            biomes: vec![0; BIOME_VOLUME],
        }
    }

    /// 读取 (x, y, z) 处方块 id；任一坐标越界返回 0。
    pub fn get(&self, x: usize, y: usize, z: usize) -> u32 {
        match self.block_index(x, y, z) {
            Some(index) => self.blocks.get(index).copied().unwrap_or(0),
            None => 0,
        }
    }

    /// 写入 (x, y, z) 处方块 id；任一坐标越界返回 `false` 且不写入。
    pub fn set(&mut self, x: usize, y: usize, z: usize, id: u32) -> bool {
        let index = match self.block_index(x, y, z) {
            Some(i) => i,
            None => return false,
        };
        match self.blocks.get_mut(index) {
            Some(slot) => {
                *slot = id;
                true
            }
            None => false,
        }
    }

    /// 由 (x, y, z) 计算方块数组下标（`y*256 + z*16 + x`）；越界返回 `None`。
    fn block_index(&self, x: usize, y: usize, z: usize) -> Option<usize> {
        if x >= 16 || y >= 16 || z >= 16 {
            return None;
        }
        Some(y * 256 + z * 16 + x)
    }
}

/// 空实现生成器：任何坐标都产出全空气区块。
///
/// # 注意
/// 本类型已被 [`NoiseChunkGenerator`] 取代，保留用于兼容旧代码与回滚路径。
#[deprecated(since = "0.1.0", note = "使用 NoiseChunkGenerator 替代")]
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopChunkGenerator;

#[allow(deprecated)]
impl ChunkGenerator for NoopChunkGenerator {
    fn generate(&self, _x: i32, _z: i32, _seed: u64) -> GeneratedChunk {
        GeneratedChunk::empty()
    }
}

/// 基于 Perlin noise 的地形生成器。
///
/// 参数说明：
/// - `base_height`：地形基准高度（相对于区段底部，y=0）；
/// - `amplitude`：噪声引起的最大高度偏移量；
/// - `noise_scale`：控制地形起伏频率（值越小，地形越平缓）；
/// - `octaves`：叠加的噪声层数（越多越细腻，典型值 2–4）。
///
/// 地形填充规则：
/// - 区块内方块局部坐标 `(x, z)` 对应的世界坐标为 `(x*16 + local_x, z*16 + local_z)`；
/// - 对该世界坐标采样噪声，映射到 `[base_height - amplitude, base_height + amplitude]`；
/// - 取整后得到该列的 terrain_y（限幅到 [0, 16)）；
/// - y < terrain_y 的格子填 stone（id=1），y >= terrain_y 的格子填空气（id=0）。
#[derive(Debug, Clone)]
pub struct NoiseChunkGenerator {
    base_height: u32,
    amplitude: u32,
    noise_scale: f64,
    octaves: u32,
    persistence: f64,
}

impl Default for NoiseChunkGenerator {
    fn default() -> Self {
        Self {
            // v1 单区段 y ∈ [0, 16)，base_height 与 amplitude 均在此范围内，
            // 使地形起伏分布在 [4, 12]，产生合理的石头/空气混合。
            base_height: 8,
            amplitude: 4,
            noise_scale: 0.01,
            octaves: 3,
            persistence: 0.5,
        }
    }
}

impl NoiseChunkGenerator {
    /// 构造默认参数生成器（base_height=8, amplitude=4，适应 v1 单区段）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 以自定义振幅构造生成器（base_height 保持默认值 8）。
    pub fn with_amplitude(amplitude: u32) -> Self {
        Self {
            amplitude,
            ..Self::default()
        }
    }

    /// 平坦地形生成器： terrain_y 固定为 `height`，无噪声起伏。
    ///
    /// 等价于 `NoiseChunkGenerator` 的 `amplitude = 0` 变体。
    pub fn flat(height: u32) -> Self {
        Self {
            base_height: height,
            amplitude: 0,
            noise_scale: 0.0,
            octaves: 0,
            persistence: 0.0,
        }
    }
}

impl ChunkGenerator for NoiseChunkGenerator {
    fn generate(&self, x: i32, z: i32, seed: u64) -> GeneratedChunk {
        let perm = make_perm(seed);
        let scale = self.noise_scale;
        let amp = self.amplitude as f64;
        let base = self.base_height as f64;
        let oct = self.octaves;
        let pers = self.persistence;
        let chunk_block_x = x * 16;
        let chunk_block_z = z * 16;
        let mut blocks = vec![0u32; SECTION_VOLUME];

        for local_z in 0..16 {
            for local_x in 0..16 {
                let world_x = chunk_block_x + local_x as i32;
                let world_z = chunk_block_z + local_z as i32;
                let noise_val = if oct == 0 {
                    0.0
                } else {
                    perlin_noise(
                        &perm,
                        world_x as f64 * scale,
                        world_z as f64 * scale,
                        oct,
                        pers,
                    )
                };
                let terrain_y = (base + noise_val * amp).clamp(0.0, 16.0) as u32;

                // 垂直列填充：y < terrain_y 为 stone，否则为 air。
                for local_y in 0..terrain_y {
                    let idx = local_y as usize * 256 + local_z * 16 + local_x;
                    blocks[idx] = 1;
                }
            }
        }

        GeneratedChunk {
            blocks,
            biomes: vec![0; BIOME_VOLUME],
        }
    }
}

/// 把生成结果写入一个以 `(x, z)` 为坐标、含 1 个区段的 [`Chunk`]。
///
/// v1 中 [`GeneratedChunk`] 只承载一层区段（y ∈ [0, 16)），故目标区块固定
/// 为单区段；写入使用区段级 `set_block`，无需裸索引。
pub fn generated_to_chunk(r#gen: &GeneratedChunk, x: i32, z: i32) -> Chunk {
    let mut chunk = Chunk::new(x, z, 1);
    for (index, &id) in r#gen.blocks.iter().enumerate() {
        // index 恒在 [0, 4096) 内，落在区段 0；失败仅可能是内部状态异常。
        let _ = chunk.set_block(0, index, id);
    }
    chunk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(deprecated)]
    #[test]
    fn noop_generator_produces_all_air() {
        let generated = NoopChunkGenerator.generate(0, 0, 42);
        assert_eq!(generated.blocks.len(), SECTION_VOLUME);
        assert_eq!(generated.biomes.len(), BIOME_VOLUME);
        assert!(generated.blocks.iter().all(|&id| id == 0));
        assert!(generated.biomes.iter().all(|&id| id == 0));
    }

    #[test]
    fn empty_chunk_has_full_air_volume() {
        let generated = GeneratedChunk::empty();
        assert_eq!(generated.blocks.len(), 4096);
        assert_eq!(generated.biomes.len(), 64);
    }

    #[test]
    fn generated_to_chunk_writes_and_reads_back() {
        let mut generated = GeneratedChunk::empty();
        // (x=1, y=2, z=3) → 下标 2*256 + 3*16 + 1 = 561。
        assert!(generated.set(1, 2, 3, 42));
        let chunk = generated_to_chunk(&generated, 7, -3);
        assert_eq!(chunk.x(), 7);
        assert_eq!(chunk.z(), -3);
        assert_eq!(chunk.get_block(0, 561), 42);
        // 其余格仍为空气。
        assert_eq!(chunk.get_block(0, 0), 0);
        assert_eq!(chunk.get_block(0, 4095), 0);
    }

    #[test]
    fn index_mapping_follows_yz_then_x_order() {
        // 写入多个 (x, y, z)，校验下标公式 y*256 + z*16 + x 与读回一致。
        let mut generated = GeneratedChunk::empty();
        let cases = [(0usize, 0usize, 0usize), (15, 0, 0), (0, 15, 15), (7, 8, 9)];
        for (x, y, z) in cases {
            let id = u32::try_from(x * 16 + y * 8 + z + 1).unwrap_or(0);
            assert!(generated.set(x, y, z, id));
        }
        let chunk = generated_to_chunk(&generated, 0, 0);
        for (x, y, z) in cases {
            let id = u32::try_from(x * 16 + y * 8 + z + 1).unwrap_or(0);
            let index = y * 256 + z * 16 + x;
            assert_eq!(chunk.get_block(0, index), id);
        }
    }

    #[test]
    fn generated_chunk_bounds_are_safe() {
        let mut generated = GeneratedChunk::empty();
        assert!(!generated.set(16, 0, 0, 1));
        assert!(!generated.set(0, 16, 0, 1));
        assert!(!generated.set(0, 0, 16, 1));
        assert_eq!(generated.get(16, 0, 0), 0);
        assert_eq!(generated.get(0, 16, 0), 0);
        assert_eq!(generated.get(0, 0, 16), 0);
    }

    #[test]
    fn from_section_extracts_blocks() {
        let mut section = Section::new();
        assert!(section.set_block_id(100, 7));
        assert!(section.set_block_id(400, 3));
        let generated = GeneratedChunk::from_section(&section);
        // 下标 100 = 0*256 + 6*16 + 4 → (x=4, y=0, z=6)；下标 400 = 1*256 + 9*16 + 0。
        assert_eq!(generated.get(4, 0, 6), 7);
        assert_eq!(generated.get(0, 1, 9), 3);
        assert_eq!(generated.get(0, 0, 0), 0);
        assert_eq!(generated.biomes.len(), BIOME_VOLUME);
    }

    // ── NoiseChunkGenerator 测试 ──────────────────────────────────────────────

    #[test]
    fn flat_generator_produces_stone_below_height() {
        // FlatNoiseGenerator（base_height=64, amplitude=0）：terrain_y 恒为 0，
        // 在 v1 单区段（y ∈ [0,16)）内无石头，全为空气。
        let r#gen = NoiseChunkGenerator::flat(64);
        let generated = r#gen.generate(0, 0, 0);
        assert_eq!(generated.blocks.len(), SECTION_VOLUME);
        // height=64 超出区段范围，clamp 到 16；但 amplitude=0 时 noise=0，
        // terrain_y = 64 clamp 到 16 → 全石。验证这一点：
        let stone_count = generated.blocks.iter().filter(|&&id| id == 1).count();
        assert_eq!(stone_count, SECTION_VOLUME);
        assert_eq!(generated.biomes.len(), BIOME_VOLUME);
        assert!(generated.biomes.iter().all(|&id| id == 0));
    }

    #[test]
    fn noise_generator_deterministic() {
        // 同一 (x, z, seed) 必须产出完全相同的 GeneratedChunk。
        let r#gen = NoiseChunkGenerator::new();
        let first = r#gen.generate(3, -5, 12345);
        let second = r#gen.generate(3, -5, 12345);
        assert_eq!(first.blocks, second.blocks);
        assert_eq!(first.biomes, second.biomes);
    }

    #[test]
    fn noise_generator_has_both_stone_and_air() {
        // 默认参数（base_height=8, amplitude=4）使 terrain_y 分布在 [4, 12]，
        // 区内至少有一个石头格和一个空气格。
        let r#gen = NoiseChunkGenerator::new();
        let generated = r#gen.generate(0, 0, 0);
        let stone_count = generated.blocks.iter().filter(|&&id| id == 1).count();
        let air_count = generated.blocks.iter().filter(|&&id| id == 0).count();
        assert!(stone_count > 0, "应至少有一个石头方块");
        assert!(air_count > 0, "应至少有一个空气方块");
        assert_eq!(stone_count + air_count, SECTION_VOLUME);
    }

    #[test]
    fn different_seeds_produce_different_chunks() {
        // 不同种子应产生不同的地形（极大概率）。
        let r#gen = NoiseChunkGenerator::new();
        let a = r#gen.generate(0, 0, 1);
        let b = r#gen.generate(0, 0, 2);
        assert_ne!(a.blocks, b.blocks, "不同种子应产生不同地形");
    }

    #[test]
    fn different_chunks_are_independent() {
        // 相邻区块的生成结果应各自独立。
        let r#gen = NoiseChunkGenerator::new();
        let c1 = r#gen.generate(0, 0, 0);
        let c2 = r#gen.generate(1, 0, 0);
        // 两个区块内容不完全相同（概率极低相同）。
        assert_ne!(c1.blocks, c2.blocks);
    }

    #[test]
    fn flat_generator_with_low_height() {
        // base_height=2, amplitude=0：terrain_y = 2，前 2 层全石，其余空气。
        let r#gen = NoiseChunkGenerator::flat(2);
        let generated = r#gen.generate(0, 0, 0);
        let stone_count = generated.blocks.iter().filter(|&&id| id == 1).count();
        assert_eq!(stone_count, 2 * 16 * 16);
        let air_count = generated.blocks.iter().filter(|&&id| id == 0).count();
        assert_eq!(air_count, 14 * 16 * 16);
    }

    #[test]
    fn noise_generator_send_and_sync() {
        // NoiseChunkGenerator 必须满足 Send + Sync。
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoiseChunkGenerator>();
    }
}
