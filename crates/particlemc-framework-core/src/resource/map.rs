//! 地图画布（T13，对应 spec R13）。
//!
//! [`MapData`] 承载一张 128×128 像素的简化地图画布（简化 framebuffer），
//! `columns` / `rows` 为协议「更新区域」尺寸（v1 全量更新，恒 128），
//! 每像素 1 字节地图颜色索引。经 [`to_packet`](MapData::to_packet) 转换为
//! 协议 `MapData`(0x31) 包。
//!
//! 变更标识符：`complete-missing-subsystems`（T13）。

use crate::protocol::packets::play;

/// 地图边长（Minecraft 地图恒为 128×128 像素）。
pub const MAP_SIZE: u8 = 128;
/// 像素总数（128×128）。
const PIXEL_COUNT: usize = MAP_SIZE as usize * MAP_SIZE as usize;

/// 地图画布（简化 framebuffer）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapData {
    /// 地图 id。
    pub id: u32,
    /// 缩放级别（0..=3）。
    pub scale: u8,
    /// 更新区域列数（v1 全量，恒 128）。
    pub columns: u8,
    /// 更新区域行数（v1 全量，恒 128）。
    pub rows: u8,
    /// 像素数据（按行主序，每像素 1 字节地图颜色索引）。
    pub data: Vec<u8>,
}

impl MapData {
    /// 构造一张空白地图（128×128，全量更新区域）。
    pub fn new(id: u32, scale: u8) -> Self {
        Self {
            id,
            scale,
            columns: MAP_SIZE,
            rows: MAP_SIZE,
            data: vec![0; PIXEL_COUNT],
        }
    }

    /// 设置 (x, y) 像素颜色；越界时返回 `false` 且不修改画布。
    pub fn set_pixel(&mut self, x: u8, y: u8, color: u8) -> bool {
        let Some(index) = self.index(x, y) else {
            return false;
        };
        let Some(slot) = self.data.get_mut(index) else {
            return false;
        };
        *slot = color;
        true
    }

    /// 查询 (x, y) 像素颜色；越界返回 `None`。
    pub fn get_pixel(&self, x: u8, y: u8) -> Option<u8> {
        let index = self.index(x, y)?;
        self.data.get(index).copied()
    }

    /// 行主序像素下标（越界返回 `None`，避免裸索引）。
    fn index(&self, x: u8, y: u8) -> Option<usize> {
        if x >= MAP_SIZE || y >= MAP_SIZE {
            return None;
        }
        usize::from(y)
            .checked_mul(usize::from(MAP_SIZE))?
            .checked_add(usize::from(x))
    }

    /// 转换为协议 `MapData`(0x31) 包。
    ///
    /// 无符号 → 有符号字节按位宽转换（`0x80` → `-128`），客户端按无符号字节
    /// 还原 128 列/行；这是 Minecraft 线格式对 byte 的既有语义。
    pub fn to_packet(&self) -> play::MapData {
        play::MapData {
            map_id: i32::try_from(self.id).unwrap_or(i32::MAX),
            scale: i8::from_ne_bytes([self.scale]),
            locked: false,
            columns: i8::from_ne_bytes([self.columns]),
            rows: i8::from_ne_bytes([self.rows]),
            data: self.data.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::packet::Packet;

    #[test]
    fn new_builds_blank_128x128_canvas() {
        let map = MapData::new(7, 2);
        assert_eq!(map.id, 7);
        assert_eq!(map.scale, 2);
        assert_eq!(map.columns, MAP_SIZE);
        assert_eq!(map.rows, MAP_SIZE);
        assert_eq!(map.data.len(), PIXEL_COUNT);
        assert!(map.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn set_and_get_pixel_roundtrip() {
        let mut map = MapData::new(1, 0);
        assert!(map.set_pixel(0, 0, 42));
        assert!(map.set_pixel(127, 127, 255));
        assert_eq!(map.get_pixel(0, 0), Some(42));
        assert_eq!(map.get_pixel(127, 127), Some(255));
        // 行主序：data[128*1 + 0] 对应 (0, 1)。
        assert_eq!(map.data[128], 0);
        assert_eq!(map.data[PIXEL_COUNT - 1], 255);
    }

    #[test]
    fn out_of_bounds_pixel_is_rejected() {
        let mut map = MapData::new(1, 0);
        assert!(!map.set_pixel(128, 0, 1));
        assert!(!map.set_pixel(0, 128, 1));
        assert!(!map.set_pixel(255, 255, 1));
        assert_eq!(map.get_pixel(128, 0), None);
        assert_eq!(map.get_pixel(200, 200), None);
        // 画布保持原样（全 0）。
        assert!(map.data.iter().all(|&b| b == 0));
    }

    #[test]
    fn to_packet_encodes_full_region() {
        let mut map = MapData::new(5, 1);
        map.set_pixel(3, 4, 77);
        let packet = map.to_packet();
        assert_eq!(packet.map_id, 5);
        assert_eq!(packet.scale, 1);
        assert!(!packet.locked);
        // 128 列/行经位宽转换后线上字节为 0x80。
        assert_eq!(packet.columns, i8::from_ne_bytes([128]));
        assert_eq!(packet.rows, i8::from_ne_bytes([128]));
        assert_eq!(packet.data, map.data);
        assert_eq!(packet.data[4 * 128 + 3], 77);

        // 完整编解码 roundtrip：map_id 与数据可还原。
        let mut buf = ByteBuffer::with_capacity(64);
        packet.encode(&mut buf).unwrap();
        let decoded = play::MapData::decode(&mut buf).unwrap();
        assert_eq!(decoded.map_id, 5);
        assert_eq!(decoded.scale, 1);
        assert_eq!(decoded.data, map.data);
    }

    #[test]
    fn id_out_of_i32_range_clamps() {
        let map = MapData::new(u32::MAX, 0);
        let packet = map.to_packet();
        assert_eq!(packet.map_id, i32::MAX);
    }
}
