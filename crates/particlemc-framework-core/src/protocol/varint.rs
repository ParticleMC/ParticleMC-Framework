// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Minecraft 变长整数（VarInt / VarLong）编解码。
//!
//! 编码规则：每字节低 7 位为数据，最高位（0x80）为「还有后续」标志。
//! VarInt 最多 5 字节、VarLong 最多 10 字节，超出判定为 `VarIntTooLong`。

use crate::protocol::error::ProtocolError;

/// 写入 VarInt（最多 5 字节）。
///
/// 负数按 Minecraft 标准编码为 5 字节全 1 前缀（如 -1 = `FF FF FF FF 0F`）。
/// 必须以**无符号逻辑右移**处理（算术右移会使 -1 永不归零而无限循环）。
pub fn write_varint(buf: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            buf.push(byte);
        } else {
            buf.push(byte);
            break;
        }
    }
}

/// 从 `data` 的 `*pos` 处读取 VarInt 并推进 `*pos`。
pub fn read_varint(data: &[u8], pos: &mut usize) -> Result<i32, ProtocolError> {
    let mut result: i32 = 0;
    for i in 0..5 {
        let byte = *data.get(*pos).ok_or(ProtocolError::UnexpectedEof)?;
        *pos += 1;
        let part = i32::from(byte & 0x7F);
        result |= part << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(ProtocolError::VarIntTooLong)
}

/// 写入 VarLong（最多 10 字节）。
///
/// 与 [`write_varint`] 相同，负数以无符号逻辑右移处理，避免无限循环。
pub fn write_varlong(buf: &mut Vec<u8>, value: i64) {
    let mut value = value as u64;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            buf.push(byte);
        } else {
            buf.push(byte);
            break;
        }
    }
}

/// 从 `data` 的 `*pos` 处读取 VarLong 并推进 `*pos`。
pub fn read_varlong(data: &[u8], pos: &mut usize) -> Result<i64, ProtocolError> {
    let mut result: i64 = 0;
    for i in 0..10 {
        let byte = *data.get(*pos).ok_or(ProtocolError::UnexpectedEof)?;
        *pos += 1;
        let part = i64::from(byte & 0x7F);
        result |= part << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(result);
        }
    }
    Err(ProtocolError::VarIntTooLong)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_small() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 25565);
        assert_eq!(buf.len(), 3, "25565 应编码为 3 字节");
        let mut pos = 0;
        assert_eq!(read_varint(&buf, &mut pos).unwrap(), 25565);
    }

    #[test]
    fn varint_roundtrip_zero() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 0);
        assert_eq!(buf, vec![0]);
        let mut pos = 0;
        assert_eq!(read_varint(&buf, &mut pos).unwrap(), 0);
    }

    #[test]
    fn varint_roundtrip_negative() {
        let mut buf = Vec::new();
        write_varint(&mut buf, -1);
        let mut pos = 0;
        assert_eq!(read_varint(&buf, &mut pos).unwrap(), -1);
    }

    #[test]
    fn varint_too_long() {
        // 6 个带 continuation 位的字节
        let data = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80];
        let mut pos = 0;
        assert_eq!(
            read_varint(&data, &mut pos),
            Err(ProtocolError::VarIntTooLong)
        );
    }

    #[test]
    fn varint_eof() {
        let data = [0x80]; // continuation 但无后续
        let mut pos = 0;
        assert_eq!(
            read_varint(&data, &mut pos),
            Err(ProtocolError::UnexpectedEof)
        );
    }

    #[test]
    fn varlong_roundtrip() {
        let mut buf = Vec::new();
        write_varlong(&mut buf, i64::MAX);
        let mut pos = 0;
        assert_eq!(read_varlong(&buf, &mut pos).unwrap(), i64::MAX);
    }
}

/// 模糊测试入口（仅 `cargo fuzz` 构建启用，由 `#[cfg(fuzzing)]` 控制）。
///
/// 对 VarInt / VarLong 解码喂入任意字节，确认所有畸形 / 截断 / 超长输入均返回
/// `Err`，绝不 panic（读越界一律经 [`ProtocolError::UnexpectedEof`] 拒绝，连续位
/// 超 5/10 字节经 [`ProtocolError::VarIntTooLong`] 拒绝）。
#[cfg(fuzzing)]
pub fn fuzz_target_varint(data: &[u8]) {
    let mut pos = 0usize;
    let _ = read_varint(data, &mut pos);
    let mut pos = 0usize;
    let _ = read_varlong(data, &mut pos);
}
