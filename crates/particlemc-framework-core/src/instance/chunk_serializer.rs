//! 区块序列化：将区块/区段编码为 Minecraft 1.21.11 调色板字节流。
//!
//! 移植自 Minestom Java `PaletteImpl` / `Palettes.pack` / `Heightmap.encode`，
//! 实现三种存储模式：
//!
//! - **单值模式**（`bits_per_entry = 0`）：整个区段为同一方块，直接编码其 id；
//! - **间接模式**（`bits_per_entry ∈ [4, 8]`）：调色板索引表 + 按位打包的索引数组；
//! - **直接模式**（`bits_per_entry = 15`）：按位打包的方块 id 数组。
//!
//! 位打包与字节序规则（必须与客户端一致）：
//! - 数据数组按「每 `u64` 从低位连续存放 `floor(64 / bits)` 个条目」打包，
//!   与 Minecraft `BitStorage` / Minestom `Palettes.pack` 相同；每个 `u64`
//!   以**大端**（`to_be_bytes`）写出 8 字节，末尾不足部分补零。
//! - 高度图为 256 个 9-bit 值**连续**打包（客户端按 `i * 9` 位偏移解析），
//!   允许条目跨 `u64` 边界，共 36 个 `u64`。
//!
//! 模式自动选择：1 种方块 → 单值；2..=16 种 → 间接（`bits = 4`）；
//! 多于 16 种 → 直接（15 bits）。

use std::collections::BTreeSet;

use crate::instance::chunk::{Chunk, Section};
use crate::protocol::varint::write_varint;
use crate::resource::registries::BlockRegistry;

/// 区段边长（16）。
pub const BLOCK_DIMENSION: usize = 16;
/// 间接模式允许的最小位数。
pub const BLOCK_PALETTE_MIN_BITS: u32 = 4;
/// 间接模式允许的最大位数。
pub const BLOCK_PALETTE_MAX_BITS: u32 = 8;
/// 直接模式的位数（方块状态 id 位宽上限）。
pub const BLOCK_PALETTE_DIRECT_BITS: u32 = 15;
/// MOTION_BLOCKING 高度图在 `minecraft:heightmap` 注册表中的协议序号。
const HEIGHTMAP_MOTION_BLOCKING_ID: i32 = 4;
/// 高度图每列高度的位宽（9 位可表示 0..=511）。
const HEIGHTMAP_BITS: u32 = 9;
/// 区块每列数（16×16 = 256）。
const HEIGHTMAP_COLUMNS: usize = BLOCK_DIMENSION * BLOCK_DIMENSION;
/// 高度图打包后的 u64 数量（`ceil(256 × 9 / 64) = 36`）。
const HEIGHTMAP_LONGS: usize = 36;

/// 一个区块的序列化结果，字节可直接用于 `MapChunk` 数据包。
pub struct SerializedChunk {
    /// 区块 X 坐标。
    pub x: i32,
    /// 区块 Z 坐标。
    pub z: i32,
    /// 全部区段的调色板编码字节（对应 `MapChunk.chunk_data`）。
    pub data: Vec<u8>,
    /// 高度图协议字节（`MapChunk.heightmaps` 的 wire 编码，含 VarInt 头部）。
    pub heightmaps: Vec<u8>,
    /// 方块实体协议字节（当前恒为 VarInt(0)，表示无方块实体）。
    pub block_entities: Vec<u8>,
}

/// 序列化整个区块：拼接全部区段 + 计算 MOTION_BLOCKING 高度图。
pub fn serialize_chunk(chunk: &Chunk, block_registry: &BlockRegistry) -> SerializedChunk {
    let mut data = Vec::new();
    for section in &chunk.sections {
        data.extend_from_slice(&serialize_section(section, block_registry));
    }
    SerializedChunk {
        x: chunk.x,
        z: chunk.z,
        data,
        heightmaps: encode_motion_blocking_heightmap(chunk, block_registry),
        // 协议中无方块实体时的合法最小编码：VarInt(0)。
        block_entities: vec![0x00],
    }
}

/// 序列化单个区段：`u16` 非空气计数 + 调色板编码。
pub fn serialize_section(section: &Section, block_registry: &BlockRegistry) -> Vec<u8> {
    let air = air_block_id(block_registry);

    // 一次遍历收集原始 id，同时统计非空气数与不同方块种类数。
    let mut ids = Vec::with_capacity(section.len());
    let mut unique = BTreeSet::new();
    let mut non_air: u16 = 0;
    for index in 0..section.len() {
        let id = section.get_block_id(index);
        ids.push(id);
        if id != air {
            non_air = non_air.saturating_add(1);
        }
        unique.insert(id);
    }

    let mut buf = Vec::new();
    // 线格式首字段：非空气方块数（u16，大端）。
    buf.extend_from_slice(&non_air.to_be_bytes());

    match unique.len() {
        // 0 种理论上不可达（区段恒含方块），与 1 种一样回退单值 0。
        0 | 1 => encode_single_value(&mut buf, unique.first().copied().unwrap_or(0)),
        2..=16 => encode_indirect(&mut buf, &ids, &unique),
        _ => encode_direct(&mut buf, &ids),
    }
    buf
}

/// 单值模式：`bits = 0` + VarInt 该方块 id。
fn encode_single_value(buf: &mut Vec<u8>, value: u32) {
    buf.push(0);
    write_varint(buf, i32::try_from(value).unwrap_or(0));
}

/// 间接模式：调色板索引表 + 按位打包的索引数组。
///
/// 位宽取 `max(4, 表示最大下标的位数)`；条目数 ∈ [2, 16] 时恒为 4。
fn encode_indirect(buf: &mut Vec<u8>, ids: &[u32], unique: &BTreeSet<u32>) {
    let bits = indirect_bits(unique.len());
    // bits ∈ {4}，恒小于 256，此处窄化理论不可达。
    buf.push(u8::try_from(bits).unwrap_or(0));

    // palette：有序 id 列表，数组下标即该方块在数据数组中的索引。
    let palette: Vec<i32> = unique
        .iter()
        .map(|&id| i32::try_from(id).unwrap_or(0))
        .collect();
    write_varint(buf, i32::try_from(palette.len()).unwrap_or(0));
    for &id in &palette {
        write_varint(buf, id);
    }

    // 数据数组存 palette 下标：方块 id → palette 位置。
    let indices: Vec<u32> = ids
        .iter()
        .map(|&id| {
            let id_i32 = i32::try_from(id).unwrap_or(0);
            palette
                .iter()
                .position(|&p| p == id_i32)
                .and_then(|idx| u32::try_from(idx).ok())
                .unwrap_or(0)
        })
        .collect();
    let longs = pack_entries(&indices, bits);
    for long in &longs {
        buf.extend_from_slice(&long.to_be_bytes());
    }
}

/// 直接模式：`bits = 15` + 按位打包的方块 id 数组。
fn encode_direct(buf: &mut Vec<u8>, ids: &[u32]) {
    buf.push(u8::try_from(BLOCK_PALETTE_DIRECT_BITS).unwrap_or(0));
    let longs = pack_entries(ids, BLOCK_PALETTE_DIRECT_BITS);
    for long in &longs {
        buf.extend_from_slice(&long.to_be_bytes());
    }
}

/// 间接模式的位数：`max(4, 表示 palette 最大下标的位数)`。
fn indirect_bits(unique_count: usize) -> u32 {
    let needed = bits_to_represent(unique_count.saturating_sub(1));
    needed.max(BLOCK_PALETTE_MIN_BITS)
}

/// 表示 `n` 所需的位数（对应 Minestom `MathUtils.bitsToRepresent`）。
///
/// 例如 15 → 4、16 → 5；`n = 0`（空集合）返回 0。
fn bits_to_represent(n: usize) -> u32 {
    if n == 0 {
        return 0;
    }
    // 设计意图：无符号整数位宽减去前导零数量即为表示该值所需位数，
    // 等价于 `ceil(log2(n + 1))`，避免浮点运算。
    usize::BITS - n.leading_zeros()
}

/// 将条目按固定位数打包为 `u64` 数组。
///
/// 位运算设计意图：与 Minecraft `BitStorage` / Minestom `Palettes.pack` 一致，
/// 每 `u64` 从低位（LSB）开始连续存放 `floor(64 / bits)` 个条目，剩余高位
/// 补零；条目不跨 `u64` 边界。
fn pack_entries(entries: &[u32], bits: u32) -> Vec<u64> {
    // bits ∈ {4, 15}，此处扩宽不缩窄。
    let bits_usize = bits as usize;
    let entries_per_long = 64 / bits_usize;
    let long_count = entries.len().div_ceil(entries_per_long);
    let mut out = vec![0u64; long_count];
    for (i, &entry) in entries.iter().enumerate() {
        let long_index = i / entries_per_long;
        let bit_index = (i % entries_per_long) * bits_usize;
        if let Some(long) = out.get_mut(long_index) {
            // 仅取低 bits 位后左移入其槽位；`bits >= 64` 分支仅防御
            // `(1u64 << 64)` 溢出，实际不可达（bits ≤ 15）。
            let mask = if bits >= 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            *long |= (u64::from(entry) & mask) << bit_index;
        }
    }
    out
}

/// MOTION_BLOCKING 高度图协议字节。
///
/// 线格式：`VarInt(高度图数量=1)` + `VarInt(类型=4)` + `VarInt(long 数)`
/// + 逐 `u64` 大端数据。
fn encode_motion_blocking_heightmap(chunk: &Chunk, block_registry: &BlockRegistry) -> Vec<u8> {
    let air = air_block_id(block_registry);
    let mut heights = [0u16; HEIGHTMAP_COLUMNS];
    for z in 0..BLOCK_DIMENSION {
        for x in 0..BLOCK_DIMENSION {
            if let Some(slot) = heights.get_mut(z * BLOCK_DIMENSION + x) {
                *slot = column_height(chunk, x, z, air);
            }
        }
    }

    let longs = pack_9bit(&heights);
    let mut buf = Vec::new();
    write_varint(&mut buf, 1); // 仅发送 MOTION_BLOCKING 一种高度图
    write_varint(&mut buf, HEIGHTMAP_MOTION_BLOCKING_ID);
    write_varint(&mut buf, i32::try_from(longs.len()).unwrap_or(0));
    for long in &longs {
        buf.extend_from_slice(&long.to_be_bytes());
    }
    buf
}

/// 将 256 个高度值按 9-bit 连续打包为 `u64` 数组。
///
/// 位运算设计意图：客户端按 `i * 9` 位偏移逐位解析高度图，条目可跨越
/// `u64` 字边界，因此借助 [`write_bits`] 做跨字写入，而非逐字独立打包。
fn pack_9bit(heights: &[u16]) -> Vec<u64> {
    let mut out = vec![0u64; HEIGHTMAP_LONGS];
    let mut bit_pos = 0usize;
    for &height in heights {
        // 高度以 9 位存储；超出 0x1FF 时截断（常规世界 384 层远低于上限）。
        write_bits(
            &mut out,
            &mut bit_pos,
            u64::from(height) & 0x1FF,
            HEIGHTMAP_BITS,
        );
    }
    out
}

/// 从 `bit_pos` 位起向 `out` 写入 `value` 的低 `bits` 位，必要时跨 `u64` 边界。
///
/// 位运算设计意图：与 Java `Palettes.write` 相同的「先清空目标位区间、再置入
/// 新值」两步，保证不破坏同字内相邻条目；跨字时按当前字可用位数分片写入。
fn write_bits(out: &mut [u64], bit_pos: &mut usize, value: u64, bits: u32) {
    let mut remaining = bits;
    let mut value = value;
    let mut pos = *bit_pos;
    while remaining > 0 {
        let long_index = pos / 64;
        let bit_in_long = pos % 64;
        // 当前字内从 bit_in_long 到结尾的可用位数。
        let available = 64 - bit_in_long;
        let take = remaining.min(available as u32);
        // 防御 `(1u64 << 64)` 溢出：本调用 bits ≤ 9，`take == 64` 实际不可达。
        let mask = if take >= 64 {
            u64::MAX
        } else {
            (1u64 << take) - 1
        };
        if let Some(long) = out.get_mut(long_index) {
            // 先清空 [bit_in_long, bit_in_long + take) 位，再写入数据低位。
            *long = (*long & !(mask << bit_in_long)) | ((value & mask) << bit_in_long);
        }
        value >>= take;
        pos += take as usize;
        remaining -= take;
    }
    *bit_pos = pos;
}

/// 计算某一列 `(x, z)` 的最高实心方块高度（区块内自底向上的全局 y）。
///
/// 实心定义为「非空气方块」。整列无实心方块时返回 0（区块底部之下），
/// 与 Minestom 高度图「未找到时落回底部」的语义一致。
fn column_height(chunk: &Chunk, x: usize, z: usize, air: u32) -> u16 {
    // 自顶向下扫描区段，首个非空气方块即该列最高点。
    for section_index in (0..chunk.sections.len()).rev() {
        let Some(section) = chunk.sections.get(section_index) else {
            break; // 理论不可达：Chunk 保证至少一个区段
        };
        for local_y in (0..BLOCK_DIMENSION).rev() {
            // 区段内线性索引：y 占高 8 位、z 占中间 4 位、x 占低 4 位。
            let index = (local_y << 8) | (z << 4) | x;
            if section.get_block_id(index) != air {
                return u16::try_from(section_index * BLOCK_DIMENSION + local_y).unwrap_or(0);
            }
        }
    }
    0
}

/// 空气方块 id：优先取注册表登记的 `minecraft:air`，缺省约定为 0。
fn air_block_id(registry: &BlockRegistry) -> u32 {
    registry.0.get_id("minecraft:air").unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::chunk::{SECTION_VOLUME, Section};
    use crate::protocol::varint::read_varint;
    use crate::resource::registries::{BlockDefinition, Registry};

    /// 构造测试注册表：air=0、stone=1、dirt=2。
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
            name = "minecraft:dirt"
        "#;
        let inner = Registry::<BlockDefinition>::from_toml_str(toml).unwrap();
        BlockRegistry(inner)
    }

    /// 从 `offset` 处读取一个 u64（大端）。
    fn read_be_u64(buf: &[u8], offset: usize) -> u64 {
        let mut arr = [0u8; 8];
        arr.copy_from_slice(buf.get(offset..offset + 8).unwrap());
        u64::from_be_bytes(arr)
    }

    /// 解码单个区段，返回 (非空气数, 4096 个方块 id)。
    fn decode_section(buf: &[u8]) -> (u16, Vec<u32>) {
        let block_count = u16::from_be_bytes([*buf.first().unwrap(), *buf.get(1).unwrap()]);
        let bits = *buf.get(2).unwrap();
        let mut pos = 3usize;
        let values: Vec<u32> = if bits == 0 {
            let value = read_varint(buf, &mut pos).unwrap();
            vec![u32::try_from(value).unwrap(); SECTION_VOLUME]
        } else if bits <= u8::try_from(BLOCK_PALETTE_MAX_BITS).unwrap() {
            let palette_len = usize::try_from(read_varint(buf, &mut pos).unwrap()).unwrap();
            let mut palette = Vec::with_capacity(palette_len);
            for _ in 0..palette_len {
                palette.push(u32::try_from(read_varint(buf, &mut pos).unwrap()).unwrap());
            }
            unpack_indices(buf, &mut pos, usize::from(bits), &palette)
        } else {
            unpack_direct(buf, &mut pos, usize::from(bits))
        };
        (block_count, values)
    }

    /// 间接模式：数据数组存 palette 下标，解码还原为方块 id。
    fn unpack_indices(buf: &[u8], pos: &mut usize, bits: usize, palette: &[u32]) -> Vec<u32> {
        let entries_per_long = 64 / bits;
        let mask = (1u64 << bits) - 1;
        let mut values = Vec::with_capacity(SECTION_VOLUME);
        for i in 0..SECTION_VOLUME {
            let long_index = i / entries_per_long;
            let bit_index = (i % entries_per_long) * bits;
            let long = read_be_u64(buf, *pos + long_index * 8);
            let idx = usize::try_from((long >> bit_index) & mask).unwrap();
            values.push(*palette.get(idx).unwrap());
        }
        values
    }

    /// 直接模式：数据数组直接存方块 id。
    fn unpack_direct(buf: &[u8], pos: &mut usize, bits: usize) -> Vec<u32> {
        let entries_per_long = 64 / bits;
        let mask = (1u64 << bits) - 1;
        let mut values = Vec::with_capacity(SECTION_VOLUME);
        for i in 0..SECTION_VOLUME {
            let long_index = i / entries_per_long;
            let bit_index = (i % entries_per_long) * bits;
            let long = read_be_u64(buf, *pos + long_index * 8);
            values.push(u32::try_from((long >> bit_index) & mask).unwrap());
        }
        values
    }

    /// 解码高度图协议字节，返回 256 个列高度。
    fn decode_heightmap(buf: &[u8]) -> Vec<u16> {
        let mut pos = 0usize;
        let _count = read_varint(buf, &mut pos).unwrap();
        let _map_type = read_varint(buf, &mut pos).unwrap();
        let long_count = usize::try_from(read_varint(buf, &mut pos).unwrap()).unwrap();
        let mut longs = Vec::with_capacity(long_count);
        for _ in 0..long_count {
            longs.push(read_be_u64(buf, pos));
            pos += 8;
        }
        // 与编码对称：按 `i * 9` 位偏移逐位解析，跨字读取。
        let mut heights = Vec::with_capacity(HEIGHTMAP_COLUMNS);
        for i in 0..HEIGHTMAP_COLUMNS {
            let mut value = 0u16;
            for k in 0..HEIGHTMAP_BITS {
                let bit =
                    i * usize::try_from(HEIGHTMAP_BITS).unwrap() + usize::try_from(k).unwrap();
                let long_index = bit / 64;
                let bit_in_long = bit % 64;
                let v = (longs.get(long_index).unwrap() >> bit_in_long) & 1;
                value |= u16::try_from(v).unwrap() << usize::try_from(k).unwrap();
            }
            heights.push(value);
        }
        heights
    }

    #[test]
    fn empty_section_uses_single_value_mode() {
        let registry = test_registry();
        let section = Section::new();
        let bytes = serialize_section(&section, &registry);
        // block_count=0, bits=0, VarInt(0) = 0x00
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00]);
        let (count, values) = decode_section(&bytes);
        assert_eq!(count, 0);
        assert!(values.iter().all(|&v| v == 0));
    }

    #[test]
    fn uniform_section_encodes_single_value() {
        let registry = test_registry();
        let mut section = Section::new();
        for i in 0..SECTION_VOLUME {
            section.set_block_id(i, 42);
        }
        let bytes = serialize_section(&section, &registry);
        // 4096 = 0x1000，bits=0，VarInt(42)=0x2A
        assert_eq!(bytes, vec![0x10, 0x00, 0x00, 42]);
        let (count, values) = decode_section(&bytes);
        assert_eq!(count, u16::try_from(SECTION_VOLUME).unwrap());
        assert!(values.iter().all(|&v| v == 42));
    }

    #[test]
    fn two_block_types_use_indirect_mode() {
        let registry = test_registry();
        let mut section = Section::new();
        for i in 0..16 {
            section.set_block_id(i, 1); // 石头
        }
        let bytes = serialize_section(&section, &registry);
        assert_eq!(*bytes.get(2).unwrap(), 4); // bits = 4
        let (count, values) = decode_section(&bytes);
        assert_eq!(count, 16);
        for i in 0..16 {
            assert_eq!(*values.get(i).unwrap(), 1);
        }
        for i in 16..SECTION_VOLUME {
            assert_eq!(*values.get(i).unwrap(), 0);
        }
    }

    #[test]
    fn seventeen_block_types_use_direct_mode() {
        let registry = test_registry();
        let mut section = Section::new();
        // id 1..=16 各占一格，加上空气共 17 种 → 直接模式。
        for id in 1..=16u32 {
            section.set_block_id(usize::try_from(id).unwrap(), id);
        }
        let bytes = serialize_section(&section, &registry);
        assert_eq!(*bytes.get(2).unwrap(), 15); // direct bits
        // 长度：2 + 1 + 1024×8
        assert_eq!(bytes.len(), 2 + 1 + 1024 * 8);
        let (count, values) = decode_section(&bytes);
        assert_eq!(count, 16);
        for id in 1..=16u32 {
            assert_eq!(*values.get(usize::try_from(id).unwrap()).unwrap(), id);
        }
    }

    #[test]
    fn mixed_section_roundtrips() {
        let registry = test_registry();
        let mut section = Section::new();
        for i in 0..SECTION_VOLUME {
            let id = match i % 5 {
                0 => 0u32,
                1 => 1,
                2 => 2,
                _ => 7,
            };
            section.set_block_id(i, id);
        }
        let bytes = serialize_section(&section, &registry);
        // 4 种方块 → 间接模式
        assert_eq!(*bytes.get(2).unwrap(), 4);
        let (count, values) = decode_section(&bytes);
        let expected_non_air =
            u16::try_from((0..SECTION_VOLUME).filter(|&i| i % 5 != 0).count()).unwrap();
        assert_eq!(count, expected_non_air);
        for i in 0..SECTION_VOLUME {
            let expected = match i % 5 {
                0 => 0u32,
                1 => 1,
                2 => 2,
                _ => 7,
            };
            assert_eq!(*values.get(i).unwrap(), expected);
        }
    }

    #[test]
    fn pack_entries_pads_high_bits_of_each_long() {
        // 4-bit：第一个 u64 低 8 位为 0b0001_0000 = 0x10。
        let entries = vec![0u32, 1];
        assert_eq!(pack_entries(&entries, 4), vec![0x10u64]);
        // 15-bit：每 u64 放 4 个条目，第 5 个进入下一个 u64。
        let entries: Vec<u32> = (0..5u32).collect();
        let longs = pack_entries(&entries, 15);
        assert_eq!(longs.len(), 2);
        // bits 0..15=0, 15..30=1, 30..45=2, 45..60=3
        assert_eq!(*longs.first().unwrap(), 0x0000_6000_8000_8000u64);
        assert_eq!(*longs.get(1).unwrap(), 4);
    }

    #[test]
    fn heightmap_matches_known_terrain() {
        let registry = test_registry();
        let mut chunk = Chunk::new(3, -4, 2);
        // 列 (x=1, z=2)：y=5 石头、y=10 泥土，取最高 10。
        assert!(chunk.set_block(0, (5 << 8) | (2 << 4) | 1, 1));
        assert!(chunk.set_block(0, (10 << 8) | (2 << 4) | 1, 2));
        // 列 (x=5, z=4)：section 1 的 y=3，全局高度 16 + 3 = 19。
        assert!(chunk.set_block(1, (3 << 8) | (4 << 4) | 5, 1));

        let serialized = serialize_chunk(&chunk, &registry);
        let heights = decode_heightmap(&serialized.heightmaps);
        assert_eq!(heights.len(), HEIGHTMAP_COLUMNS);
        assert_eq!(*heights.get(2 * 16 + 1).unwrap(), 10);
        assert_eq!(*heights.get(4 * 16 + 5).unwrap(), 19);
        assert_eq!(*heights.first().unwrap(), 0); // 无方块列 → 底部
    }

    #[test]
    fn chunk_serializes_multiple_sections_in_order() {
        let registry = test_registry();
        let mut chunk = Chunk::new(0, 0, 3);
        assert!(chunk.set_block(1, 0, 1)); // 中间区段放一块石头

        let serialized = serialize_chunk(&chunk, &registry);
        let s0 = serialize_section(chunk.sections.first().unwrap(), &registry);
        let s1 = serialize_section(chunk.sections.get(1).unwrap(), &registry);
        let s2 = serialize_section(chunk.sections.get(2).unwrap(), &registry);
        let expect_len = s0.len() + s1.len() + s2.len();
        assert_eq!(serialized.data.len(), expect_len);
        assert_eq!(serialized.data.get(..s0.len()).unwrap(), s0.as_slice());
        assert_eq!(serialized.x, 0);
        assert_eq!(serialized.z, 0);
        assert!(!serialized.heightmaps.is_empty());
        assert_eq!(serialized.block_entities, vec![0x00]);
    }
}
