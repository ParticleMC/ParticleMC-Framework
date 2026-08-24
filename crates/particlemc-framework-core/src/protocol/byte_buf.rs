//! 字节缓冲：游标式读 / 写，所有越界一律返回 [`ProtocolError::UnexpectedEof`]。
//!
//! 读游标只读、不修改底层 `buf`；写游标追加到末尾。所有整数按大端（网络字节序）
//! 编解码，与 Minecraft 线格式一致。方块坐标位置（26/12/38 位）按已知位宽提取，
//! 此处使用经证明安全的收窄（见模块顶部 allow 注释）。

#![allow(clippy::cast_possible_truncation)]

use uuid::Uuid;

use crate::protocol::error::ProtocolError;
use crate::protocol::varint::{read_varint, read_varlong, write_varint, write_varlong};

/// 字符串最大长度（Minecraft 限制，单位：UTF-8 字节数）。
const MAX_STRING_LEN: usize = 32_767;

/// 游标式字节缓冲。读 / 写共享一个底层 `buf`，`pos` 为当前读位置。
#[derive(Debug, Clone)]
pub struct ByteBuffer {
    buf: Vec<u8>,
    pos: usize,
}

impl ByteBuffer {
    /// 用已有字节构造（读模式）。
    pub fn new(buf: Vec<u8>) -> Self {
        Self { buf, pos: 0 }
    }

    /// 预分配写缓冲。
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
            pos: 0,
        }
    }

    /// 取出内部字节（写模式完成后使用）。
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    /// 已写内容切片（写模式）。读游标不影响此切片长度。
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// 缓冲区总字节数。
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// 剩余可读字节数。
    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// 当前读位置。
    pub fn position(&self) -> usize {
        self.pos
    }

    fn peek(&self, index: usize) -> Result<u8, ProtocolError> {
        self.buf
            .get(index)
            .copied()
            .ok_or(ProtocolError::UnexpectedEof)
    }

    // ---- 读游标 ----

    pub fn get_u8(&mut self) -> Result<u8, ProtocolError> {
        let b = self.peek(self.pos)?;
        self.pos += 1;
        Ok(b)
    }

    pub fn get_i8(&mut self) -> Result<i8, ProtocolError> {
        let b = self.get_u8()?;
        Ok(i8::from_ne_bytes([b]))
    }

    pub fn get_u16(&mut self) -> Result<u16, ProtocolError> {
        let b0 = self.get_u8()?;
        let b1 = self.get_u8()?;
        Ok(u16::from_be_bytes([b0, b1]))
    }

    pub fn get_i16(&mut self) -> Result<i16, ProtocolError> {
        let b0 = self.get_u8()?;
        let b1 = self.get_u8()?;
        Ok(i16::from_be_bytes([b0, b1]))
    }

    pub fn get_i32(&mut self) -> Result<i32, ProtocolError> {
        let mut arr = [0u8; 4];
        for item in &mut arr {
            *item = self.get_u8()?;
        }
        Ok(i32::from_be_bytes(arr))
    }

    pub fn get_i64(&mut self) -> Result<i64, ProtocolError> {
        let mut arr = [0u8; 8];
        for item in &mut arr {
            *item = self.get_u8()?;
        }
        Ok(i64::from_be_bytes(arr))
    }

    pub fn get_f32(&mut self) -> Result<f32, ProtocolError> {
        let mut arr = [0u8; 4];
        for item in &mut arr {
            *item = self.get_u8()?;
        }
        Ok(f32::from_be_bytes(arr))
    }

    pub fn get_f64(&mut self) -> Result<f64, ProtocolError> {
        let mut arr = [0u8; 8];
        for item in &mut arr {
            *item = self.get_u8()?;
        }
        Ok(f64::from_be_bytes(arr))
    }

    pub fn get_bool(&mut self) -> Result<bool, ProtocolError> {
        Ok(self.get_u8()? != 0)
    }

    pub fn get_uuid(&mut self) -> Result<Uuid, ProtocolError> {
        let mut arr = [0u8; 16];
        for item in &mut arr {
            *item = self.get_u8()?;
        }
        Ok(Uuid::from_bytes(arr))
    }

    pub fn get_varint(&mut self) -> Result<i32, ProtocolError> {
        read_varint(&self.buf, &mut self.pos)
    }

    /// 读取 `n` 个原始字节并推进游标（用于转发 blob 等不定长二进制段）。
    pub fn get_bytes(&mut self, n: usize) -> Result<Vec<u8>, ProtocolError> {
        let start = self.pos;
        let end = start + n;
        let slice = self
            .buf
            .get(start..end)
            .ok_or(ProtocolError::UnexpectedEof)?;
        self.pos = end;
        Ok(slice.to_vec())
    }

    pub fn get_varlong(&mut self) -> Result<i64, ProtocolError> {
        read_varlong(&self.buf, &mut self.pos)
    }

    pub fn get_string(&mut self) -> Result<String, ProtocolError> {
        let len = self.get_varint()?;
        let len_usize = usize::try_from(len).map_err(|_| ProtocolError::UnexpectedEof)?;
        if len_usize > MAX_STRING_LEN {
            return Err(ProtocolError::Utf8);
        }
        let start = self.pos;
        let end = start + len_usize;
        let bytes = self
            .buf
            .get(start..end)
            .ok_or(ProtocolError::UnexpectedEof)?;
        self.pos = end;
        String::from_utf8(bytes.to_vec()).map_err(|_| ProtocolError::Utf8)
    }

    /// 解码方块坐标（Minecraft Position 编码：x 26 位 @38、z 26 位 @12、y 12 位 @0）。
    pub fn get_position(&mut self) -> Result<(i32, i32, i32), ProtocolError> {
        let val = self.get_i64()?;
        let x = (val >> 38) as i32;
        let y = (val & 0xFFF) as i32;
        let z = (val.wrapping_shl(26) >> 38) as i32;
        Ok((x, y, z))
    }

    // ---- 写游标 ----

    pub fn put_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn put_i8(&mut self, v: i8) {
        self.buf.push(i8::to_ne_bytes(v)[0]);
    }

    pub fn put_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    pub fn put_bool(&mut self, v: bool) {
        self.buf.push(if v { 1u8 } else { 0u8 });
    }

    pub fn put_uuid(&mut self, v: Uuid) {
        self.buf.extend_from_slice(v.as_bytes());
    }

    pub fn put_varint(&mut self, v: i32) {
        write_varint(&mut self.buf, v);
    }

    /// 追加原始字节段（用于转发 blob 等不定长二进制段）。
    pub fn put_bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn put_varlong(&mut self, v: i64) {
        write_varlong(&mut self.buf, v);
    }

    pub fn put_string(&mut self, v: &str) {
        let bytes = v.as_bytes();
        // 调用方保证长度不超过 MAX_STRING_LEN；此处收窄为 i32 属安全。
        let len = bytes.len() as i32;
        self.put_varint(len);
        self.buf.extend_from_slice(bytes);
    }

    /// 编码方块坐标（Minecraft Position 编码：x 26 位 @38、z 26 位 @12、y 12 位 @0）。
    pub fn put_position(&mut self, x: i32, y: i32, z: i32) {
        let val: i64 =
            ((x as i64 & 0x3FF_FFFF) << 38) | ((z as i64 & 0x3FF_FFFF) << 12) | (y as i64 & 0xFFF);
        self.put_i64(val);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_integers() {
        let mut b = ByteBuffer::with_capacity(64);
        b.put_i8(-5);
        b.put_i16(-300);
        b.put_i32(-70_000);
        b.put_i64(-9_000_000_000);
        b.put_u16(60_000);
        b.put_bool(true);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(b.get_i8().unwrap(), -5);
        assert_eq!(b.get_i16().unwrap(), -300);
        assert_eq!(b.get_i32().unwrap(), -70_000);
        assert_eq!(b.get_i64().unwrap(), -9_000_000_000);
        assert_eq!(b.get_u16().unwrap(), 60_000);
        assert!(b.get_bool().unwrap());
    }

    #[test]
    fn roundtrip_float() {
        let mut b = ByteBuffer::with_capacity(16);
        b.put_f32(0.5);
        b.put_f64(123.456);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(b.get_f32().unwrap(), 0.5);
        assert_eq!(b.get_f64().unwrap(), 123.456);
    }

    #[test]
    fn roundtrip_string() {
        let mut b = ByteBuffer::with_capacity(32);
        b.put_string("hello 世界");
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(b.get_string().unwrap(), "hello 世界");
    }

    #[test]
    fn roundtrip_uuid() {
        let id = Uuid::nil();
        let mut b = ByteBuffer::with_capacity(16);
        b.put_uuid(id);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(b.get_uuid().unwrap(), id);
    }

    #[test]
    fn eof_on_short_buffer() {
        let mut b = ByteBuffer::new(vec![0x01]);
        assert_eq!(b.get_i32(), Err(ProtocolError::UnexpectedEof));
    }

    #[test]
    fn roundtrip_position() {
        let mut b = ByteBuffer::with_capacity(16);
        b.put_position(12, 64, 20);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(b.get_position().unwrap(), (12, 64, 20));
    }

    #[test]
    fn string_too_long_rejected() {
        // 声明长度 40000 但无内容
        let mut b = ByteBuffer::with_capacity(8);
        b.put_varint(40_000);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(b.get_string(), Err(ProtocolError::Utf8));
    }
}

/// 模糊测试入口（仅 `cargo fuzz` 构建启用，由 `#[cfg(fuzzing)]` 控制）。
///
/// 对 [`ByteBuffer`] 各读方法喂入任意字节，确认所有越界 / 畸形输入均返回 `Err`，
/// 绝不 panic。覆盖整数 / 浮点 / 布尔 / UUID / VarInt / VarLong / 字符串 / 坐标 /
/// 原始字节段等全部线格式读取路径——这些均直接处理不可信的网络输入。
#[cfg(fuzzing)]
pub fn fuzz_target_byte_buffer(data: &[u8]) {
    let mut buf = ByteBuffer::new(data.to_vec());
    let _ = buf.get_u8();
    let _ = buf.get_i8();
    let _ = buf.get_u16();
    let _ = buf.get_i16();
    let _ = buf.get_i32();
    let _ = buf.get_i64();
    let _ = buf.get_f32();
    let _ = buf.get_f64();
    let _ = buf.get_bool();
    let _ = buf.get_uuid();
    let _ = buf.get_varint();
    let _ = buf.get_varlong();
    let _ = buf.get_string();
    let _ = buf.get_position();
    let _ = buf.get_bytes(16);
}
