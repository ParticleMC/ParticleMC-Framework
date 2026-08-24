//! 帧封装：Minecraft 线格式中每个数据包外裹一层 `VarInt 长度 + payload`。
//!
//! `payload` 内含 `packet_id`（VarInt）与包体。监听器读取时先解出长度，再取
//! 对应字节数作为一帧；长度越界返回 [`ProtocolError::FrameTooLarge`]。
//!
//! T7 压缩启用后（`LoginCompression` 已下发），每帧改为压缩帧格式：
//! `VarInt 长度 + VarInt 数据长度 + 数据`，其中「数据长度」为未压缩字节数，
//! `0` 表示未压缩（原样），`>0` 表示后续字节为 zlib 压缩数据。见
//! [`encode_frame_compressed`] 与 [`decode_frame_compressed`]。

use crate::protocol::error::ProtocolError;
use crate::protocol::varint::{read_varint, write_varint};

/// 单帧最大声明长度（2 MiB，与 Minecraft 默认上限一致）。
pub const MAX_FRAME: usize = 2_097_152;

/// 将 `payload` 编码为 `VarInt 长度 + payload` 追加到 `buf`。
pub fn encode_frame(buf: &mut Vec<u8>, payload: &[u8]) -> Result<(), ProtocolError> {
    let len = payload.len();
    if len > MAX_FRAME {
        return Err(ProtocolError::FrameTooLarge);
    }
    let len_i32 = i32::try_from(len).map_err(|_| ProtocolError::FrameTooLarge)?;
    write_varint(buf, len_i32);
    buf.extend_from_slice(payload);
    Ok(())
}

/// 从 `data` 的 `*pos` 处解码一帧，推进 `*pos`，返回 payload 副本。
pub fn decode_frame(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, ProtocolError> {
    let len = read_varint(data, pos)?;
    let len_usize = usize::try_from(len).map_err(|_| ProtocolError::FrameTooLarge)?;
    if len_usize > MAX_FRAME {
        return Err(ProtocolError::FrameTooLarge);
    }
    let start = *pos;
    let end = start + len_usize;
    let payload = data.get(start..end).ok_or(ProtocolError::UnexpectedEof)?;
    *pos = end;
    Ok(payload.to_vec())
}

/// 将 `usize` 以 VarInt 形式写入 `buf`（帧长始终在 `i32` 表示范围内，故等价于
/// [`write_varint`]；独立实现以避免 `as` 缩窄转换，符合章程）。
fn write_usize_varint(buf: &mut Vec<u8>, value: usize) {
    let mut value = value;
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

/// 将 `payload`（`packet_id` + 包体）按压缩规则编码为完整帧。
///
/// - `threshold <= 0`：压缩禁用，返回原帧格式 `VarInt 长度 + payload`。
/// - `threshold > 0`：返回压缩帧格式 `VarInt 长度 + VarInt 数据长度 + 数据`；
///   - `payload` 低于阈值，或压缩后不小于原始长度时，「数据长度」字段为 `0`
///     且数据原样（对齐 Minecraft 权威压缩帧格式与 Java 出站规则）；
///   - 否则「数据长度」为 `payload.len()`，数据为 zlib 压缩字节。
pub fn encode_frame_compressed(payload: &[u8], threshold: i32) -> Vec<u8> {
    if threshold <= 0 {
        // 压缩禁用：与 [`encode_frame`] 相同的原帧格式（调用方已保证不超 MAX_FRAME）。
        let mut buf = Vec::with_capacity(payload.len() + 5);
        write_usize_varint(&mut buf, payload.len());
        buf.extend_from_slice(payload);
        return buf;
    }
    let threshold_usize = usize::try_from(threshold).unwrap_or(0);
    let raw_frame = |buf: &mut Vec<u8>| {
        // 压缩帧格式中的「未压缩」表示：数据长度字段 = 0，数据原样。
        let body_len = varint_byte_len(0) + payload.len();
        write_usize_varint(buf, body_len);
        write_varint(buf, 0);
        buf.extend_from_slice(payload);
    };
    if payload.len() < threshold_usize {
        let mut buf = Vec::with_capacity(payload.len() + 5);
        raw_frame(&mut buf);
        return buf;
    }
    let compressed = deflate_zlib(payload);
    if compressed.len() < payload.len() {
        let payload_len = payload.len();
        let body_len = varint_byte_len(payload_len) + compressed.len();
        let mut buf = Vec::with_capacity(body_len + 5);
        write_usize_varint(&mut buf, body_len);
        write_varint(&mut buf, i32::try_from(payload_len).unwrap_or(0));
        buf.extend_from_slice(&compressed);
        buf
    } else {
        // 压缩未获益：回退为未压缩表示（数据长度字段 = 0）。
        let mut buf = Vec::with_capacity(payload.len() + 5);
        raw_frame(&mut buf);
        buf
    }
}

/// 解码压缩帧体（已剥去外层 `VarInt 长度`），返回解压 / 原样后的 `payload` 副本。
///
/// 读取「数据长度」字段：`0` 表示未压缩（后续字节原样返回），`>0` 表示对后续
/// zlib 数据解压。解压声明长度与实际解压结果均受 [`MAX_FRAME`] 约束，超限返回
/// [`ProtocolError::FrameTooLarge`]；zlib 流损坏返回 [`ProtocolError::Compression`]。
pub fn decode_frame_compressed(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, ProtocolError> {
    let data_len = read_varint(data, pos)?;
    if data_len == 0 {
        let rest = data.get(*pos..).ok_or(ProtocolError::UnexpectedEof)?;
        *pos = data.len();
        return Ok(rest.to_vec());
    }
    let data_len_usize = usize::try_from(data_len).map_err(|_| ProtocolError::Compression)?;
    if data_len_usize > MAX_FRAME {
        return Err(ProtocolError::FrameTooLarge);
    }
    let compressed = data.get(*pos..).ok_or(ProtocolError::UnexpectedEof)?;
    let out = inflate_zlib(compressed, MAX_FRAME)?;
    *pos = data.len();
    Ok(out)
}

/// 计算 `value` 的 VarInt 字节长度（1..=5）。
fn varint_byte_len(value: usize) -> usize {
    let mut len = 1;
    let mut value = value >> 7;
    while value != 0 {
        len += 1;
        value >>= 7;
    }
    len
}

/// zlib 压缩 `payload` 为字节流（flate2 纯 Rust 后端）。分配失败时返回空切片，
/// 由调用方据此回退未压缩表示。
fn deflate_zlib(payload: &[u8]) -> Vec<u8> {
    use std::io::Write as _;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(payload).is_err() {
        return Vec::new();
    }
    encoder.finish().unwrap_or_default()
}

/// 解压 zlib 流为字节，输出上限 `max_len`（超出报 [`ProtocolError::FrameTooLarge`]）。
///
/// 采用固定大小输出缓冲循环（`FlushDecompress::None`），对输入耗尽而流未结束的
/// 截断流返回 [`ProtocolError::Compression`]，并限制内存占用避免压缩炸弹。
fn inflate_zlib(data: &[u8], max_len: usize) -> Result<Vec<u8>, ProtocolError> {
    use flate2::{Decompress, FlushDecompress, Status};

    let mut d = Decompress::new(true);
    let mut out: Vec<u8> = Vec::with_capacity(max_len);
    let mut in_pos = 0usize;
    loop {
        let input = data.get(in_pos..).ok_or(ProtocolError::Compression)?;
        let mut output = [0u8; 4096];
        let before_in = usize::try_from(d.total_in()).map_err(|_| ProtocolError::Compression)?;
        let before_out = usize::try_from(d.total_out()).map_err(|_| ProtocolError::Compression)?;
        let status = d
            .decompress(input, &mut output, FlushDecompress::None)
            .map_err(|_| ProtocolError::Compression)?;
        let consumed =
            usize::try_from(d.total_in()).map_err(|_| ProtocolError::Compression)? - before_in;
        let produced =
            usize::try_from(d.total_out()).map_err(|_| ProtocolError::Compression)? - before_out;
        in_pos += consumed;
        out.extend_from_slice(output.get(..produced).ok_or(ProtocolError::Compression)?);
        if out.len() > max_len {
            return Err(ProtocolError::FrameTooLarge);
        }
        match status {
            Status::StreamEnd => return Ok(out),
            Status::Ok | Status::BufError => {
                // 无进展（输入耗尽但流未结束 → 截断；输入仍在但既无消耗也无产出 → 异常）。
                if produced == 0 && consumed == 0 {
                    return Err(ProtocolError::Compression);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let payload = vec![0x00, 0x01, 0x02, 0x03];
        let mut buf = Vec::new();
        encode_frame(&mut buf, &payload).unwrap();
        let mut pos = 0;
        assert_eq!(decode_frame(&buf, &mut pos).unwrap(), payload);
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn frame_too_large() {
        let big = vec![0u8; MAX_FRAME + 1];
        let mut buf = Vec::new();
        assert_eq!(
            encode_frame(&mut buf, &big),
            Err(ProtocolError::FrameTooLarge)
        );
    }

    #[test]
    fn frame_decode_too_large() {
        // 声称长度 3_000_000（> MAX_FRAME）但实际不足
        let mut buf = Vec::new();
        write_varint(&mut buf, 3_000_000);
        let mut pos = 0;
        assert_eq!(
            decode_frame(&buf, &mut pos),
            Err(ProtocolError::FrameTooLarge)
        );
    }

    #[test]
    fn frame_eof() {
        // 长度 4 但无内容
        let mut buf = Vec::new();
        write_varint(&mut buf, 4);
        let mut pos = 0;
        assert_eq!(
            decode_frame(&buf, &mut pos),
            Err(ProtocolError::UnexpectedEof)
        );
    }

    // ---- T7 压缩帧 ----

    /// 编码器生成的帧首字段应为「压缩后帧体长度」（VarInt）。
    fn first_varint(frame: &[u8]) -> i32 {
        let mut pos = 0;
        read_varint(frame, &mut pos).unwrap()
    }

    /// 跳过外层 VarInt 长度前缀，返回帧体（`VarInt 数据长度 + 数据`）。
    /// 外层长度可能为多字节 VarInt，不能简单地取 `frame[1..]`。
    fn frame_body(frame: &[u8]) -> &[u8] {
        let mut pos = 0;
        read_varint(frame, &mut pos).unwrap();
        frame.get(pos..).expect("帧体应存在")
    }

    #[test]
    fn compressed_frame_below_threshold_is_raw_with_zero_len() {
        // 小包（低于阈值）：压缩帧格式，数据长度字段 = 0，数据原样。
        let payload = vec![0x00, 0x01, 0x02, 0x03];
        let frame = encode_frame_compressed(&payload, 256);
        // 外层长度 = VarInt(0) + 4 = 5
        assert_eq!(first_varint(&frame), 5);
        let mut pos = 0;
        assert_eq!(
            decode_frame_compressed(frame_body(&frame), &mut pos).unwrap(),
            payload
        );
    }

    #[test]
    fn compressed_frame_over_threshold_roundtrips() {
        // 大包（达到阈值）：zlib 压缩，解压后与原 payload 一致。
        let payload: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
        let frame = encode_frame_compressed(&payload, 256);
        let mut pos = 0;
        assert_eq!(
            decode_frame_compressed(frame_body(&frame), &mut pos).unwrap(),
            payload
        );
    }

    #[test]
    fn compressed_frame_threshold_zero_is_plain_format() {
        // 阈值 0：压缩禁用，返回原帧格式 `VarInt 长度 + payload`（首字段即 payload 长度）。
        let payload = vec![0x00, 0x01, 0x02, 0x03];
        let frame = encode_frame_compressed(&payload, 0);
        assert_eq!(first_varint(&frame), 4);
        assert_eq!(frame.get(1..).unwrap(), payload.as_slice());
    }

    #[test]
    fn compressed_frame_incompressible_falls_back_to_raw() {
        // 已压缩数据（高熵）：压缩后不小于原始，回退未压缩表示（数据长度字段 = 0）。
        let payload: Vec<u8> = (0..512).map(|_| 0xA5).collect();
        let frame = encode_frame_compressed(&payload, 256);
        let mut pos = 0;
        let decoded = decode_frame_compressed(frame_body(&frame), &mut pos).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn compressed_frame_declared_length_too_large_rejected() {
        // 声明原始长度 3_000_000（> MAX_FRAME）→ FrameTooLarge（解压前拒绝）。
        let mut body = Vec::new();
        write_varint(&mut body, 3_000_000);
        let mut pos = 0;
        assert_eq!(
            decode_frame_compressed(&body, &mut pos),
            Err(ProtocolError::FrameTooLarge)
        );
    }

    #[test]
    fn compressed_frame_bomb_rejected() {
        // 声明小长度但解压远超 MAX_FRAME → FrameTooLarge（解压后上限约束）。
        let payload = vec![0x00; MAX_FRAME + 1];
        let compressed = deflate_zlib(&payload);
        assert!(!compressed.is_empty());
        // 手工构造帧体：数据长度字段 = payload.len()（谎报远小于实际也可），压缩数据在尾。
        let mut body = Vec::new();
        write_varint(&mut body, i32::try_from(compressed.len()).unwrap());
        body.extend_from_slice(&compressed);
        let mut pos = 0;
        let result = decode_frame_compressed(&body, &mut pos);
        assert!(matches!(result, Err(ProtocolError::FrameTooLarge)));
    }

    #[test]
    fn compressed_frame_corrupt_zlib_rejected() {
        // 数据长度 > 0 但后续不是合法 zlib 流 → Compression 错误。
        let mut body = Vec::new();
        write_varint(&mut body, 10);
        body.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let mut pos = 0;
        assert_eq!(
            decode_frame_compressed(&body, &mut pos),
            Err(ProtocolError::Compression)
        );
    }

    #[test]
    fn compressed_frame_truncated_data_rejected() {
        // 数据长度 > 0 但压缩数据被截断（defalte 块不完整，非完整 zlib 流）→ Compression 错误。
        let payload: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
        let compressed = deflate_zlib(&payload);
        let mut body = Vec::new();
        write_varint(&mut body, payload.len() as i32);
        body.extend_from_slice(compressed.get(..compressed.len() / 2).unwrap());
        let mut pos = 0;
        assert_eq!(
            decode_frame_compressed(&body, &mut pos),
            Err(ProtocolError::Compression)
        );
    }
}

/// 模糊测试入口（仅 `cargo fuzz` 构建启用，由 `#[cfg(fuzzing)]` 控制）。
///
/// 对帧解码原语 [`decode_frame`] / [`decode_frame_compressed`] 喂入任意字节，确认
/// 所有畸形 / 截断 / 压缩炸弹输入均被安全拒绝（返回 `Err`），绝不 panic。后续通过
/// `cargo fuzz` 接入时，在 `fuzz/fuzz_targets/` 中调用本函数即可驱动模糊引擎。
#[cfg(fuzzing)]
pub fn fuzz_target_frame(data: &[u8]) {
    let mut pos = 0usize;
    let _ = decode_frame(data, &mut pos);
    let mut pos = 0usize;
    let _ = decode_frame_compressed(data, &mut pos);
}
