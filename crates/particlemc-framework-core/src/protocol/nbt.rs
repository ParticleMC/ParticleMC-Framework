// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Minecraft NBT（Named Binary Tag）网络格式编解码（big-endian）。
//!
//! 格式语义对齐框架的 `BinaryTagWriter` / `BinaryTagReader`
//! （`java/.../utils/nbt/`）：所有整数大端、字符串为 VarInt 字节长度前缀 + UTF-8。
//! 提供三种入口：
//!
//! - [`encode_root`]：带根名编码，输出 `0x0a + 0x00 + payload`（根名固定为空字符串）。
//! - [`encode_anonymous`]：anonymous 编码，不输出根名头，直接从 payload 开始
//!   （注册表同步 value 用，由调用方在上下文中声明类型为 Compound）。
//! - [`decode_root`]：解码带根名的完整 NBT，返回 `(根名, tag)`。
//! - [`decode_anonymous`]：解码 anonymous NBT（无根名头的 Compound payload），返回
//!   `(tag, 消费字节数)`，与 [`encode_anonymous`] 互补（见 `SystemChatPacket` 0x77）。
//!
//! Compound 条目按 Minecraft 规范为 `tag_type + name + payload`，以 `TAG_End` 结束。

use std::fmt;

use crate::protocol::varint;

/// `TAG_End`：Compound 条目终止符，也是空 List 的元素类型。
const TAG_END: u8 = 0;
/// 有符号 8 位整数。
const TAG_BYTE: u8 = 1;
/// 有符号 16 位整数。
const TAG_SHORT: u8 = 2;
/// 有符号 32 位整数。
const TAG_INT: u8 = 3;
/// 有符号 64 位整数。
const TAG_LONG: u8 = 4;
/// 32 位单精度浮点。
const TAG_FLOAT: u8 = 5;
/// 64 位双精度浮点。
const TAG_DOUBLE: u8 = 6;
/// 原始字节数组。
const TAG_BYTE_ARRAY: u8 = 7;
/// UTF-8 字符串。
const TAG_STRING: u8 = 8;
/// 同构元素列表。
const TAG_LIST: u8 = 9;
/// 键值对复合结构。
const TAG_COMPOUND: u8 = 10;
/// 32 位整数数组。
const TAG_INT_ARRAY: u8 = 11;
/// 64 位整数数组。
const TAG_LONG_ARRAY: u8 = 12;
/// 合法 tag 类型 id 的上界。
const MAX_TAG_ID: u8 = TAG_LONG_ARRAY;

/// NBT tag 值。
#[derive(Debug, Clone, PartialEq)]
pub enum NbtTag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<NbtTag>),
    Compound(Vec<(String, NbtTag)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// NBT 编解码错误。
#[derive(Debug, PartialEq, Eq)]
pub enum NbtError {
    /// 输入在读取过程中意外耗尽（数据截断或长度声明畸形）。
    UnexpectedEof,
    /// 遇到未知的 tag 类型 id（应为 1..=12）。
    UnknownTagId(u8),
    /// 列表元素类型非法：非空列表声明了 `TAG_End`，或根/复合结构类型不匹配。
    InvalidListType,
}

impl fmt::Display for NbtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NbtError::UnexpectedEof => write!(f, "NBT 数据不足：输入意外结束"),
            NbtError::UnknownTagId(id) => write!(f, "未知 NBT tag 类型 id：{id}"),
            NbtError::InvalidListType => write!(f, "NBT 列表元素类型非法"),
        }
    }
}

impl std::error::Error for NbtError {}

/// 带根名编码：输出 `0x0a + 0x00 + payload`（根名固定为空字符串）。
///
/// `compound` 必须为 [`NbtTag::Compound`]，否则返回 [`NbtError::InvalidListType`]。
pub fn encode_root(compound: &NbtTag) -> Result<Vec<u8>, NbtError> {
    let mut buf = Vec::new();
    buf.push(TAG_COMPOUND);
    // 根名固定为空字符串，其 VarInt 长度前缀恒为 0
    varint::write_varint(&mut buf, 0);
    write_compound_payload(&mut buf, compound)?;
    Ok(buf)
}

/// anonymous 编码：不输出根名头，直接从 payload 开始。
///
/// 输出即 [`encode_root`] 去掉 `0x0a + 0x00` 后的部分，由调用方在上下文中声明
/// 类型为 Compound（注册表同步 value 用）。
pub fn encode_anonymous(compound: &NbtTag) -> Result<Vec<u8>, NbtError> {
    let mut buf = Vec::new();
    write_compound_payload(&mut buf, compound)?;
    Ok(buf)
}

/// 解码 anonymous NBT（无根名头的 Compound payload），返回 `(tag, 消费字节数)`。
///
/// 与 [`encode_anonymous`] 严格对称：调用方已消费前导的 `TAG_COMPOUND`(0x0a) 字节，
/// 此函数从 Compound payload 起始位置（首个条目 type 或 `TAG_End`）读取，直到 `TAG_End`
/// 终止符。返回第二个值为实际消费的字节数，便于上层在后续字段（如 `SystemChatPacket`
/// 的 `overlay`）前推进游标。
///
/// 若输入并非以合法 Compound payload 起始（截断或非 TAG_End 提前结束），返回对应
/// [`NbtError`]；调用方通常将其映射为协议层错误。
pub fn decode_anonymous(bytes: &[u8]) -> Result<(NbtTag, usize), NbtError> {
    let mut pos = 0;
    let tag = read_compound(bytes, &mut pos)?;
    Ok((tag, pos))
}

/// 解码带根名的完整 NBT，返回 `(根名, tag)`。
pub fn decode_root(bytes: &[u8]) -> Result<(String, NbtTag), NbtError> {
    let mut pos = 0;
    let tag_id = read_tag_id(bytes, &mut pos)?;
    let name = read_string(bytes, &mut pos)?;
    let tag = read_payload(tag_id, bytes, &mut pos)?;
    Ok((name, tag))
}

/// tag 类型到其 id 的映射。
fn tag_id_of(tag: &NbtTag) -> u8 {
    match tag {
        NbtTag::Byte(_) => TAG_BYTE,
        NbtTag::Short(_) => TAG_SHORT,
        NbtTag::Int(_) => TAG_INT,
        NbtTag::Long(_) => TAG_LONG,
        NbtTag::Float(_) => TAG_FLOAT,
        NbtTag::Double(_) => TAG_DOUBLE,
        NbtTag::ByteArray(_) => TAG_BYTE_ARRAY,
        NbtTag::String(_) => TAG_STRING,
        NbtTag::List(_) => TAG_LIST,
        NbtTag::Compound(_) => TAG_COMPOUND,
        NbtTag::IntArray(_) => TAG_INT_ARRAY,
        NbtTag::LongArray(_) => TAG_LONG_ARRAY,
    }
}

/// 写出 tag 的 payload（不含类型 id）。
fn write_tag_payload(buf: &mut Vec<u8>, tag: &NbtTag) -> Result<(), NbtError> {
    match tag {
        NbtTag::Byte(v) => {
            let [b] = v.to_be_bytes();
            buf.push(b);
        }
        NbtTag::Short(v) => buf.extend_from_slice(&v.to_be_bytes()),
        NbtTag::Int(v) => buf.extend_from_slice(&v.to_be_bytes()),
        NbtTag::Long(v) => buf.extend_from_slice(&v.to_be_bytes()),
        NbtTag::Float(v) => buf.extend_from_slice(&v.to_be_bytes()),
        NbtTag::Double(v) => buf.extend_from_slice(&v.to_be_bytes()),
        NbtTag::ByteArray(v) => {
            write_len(buf, v.len())?;
            buf.extend_from_slice(v);
        }
        NbtTag::String(v) => write_string(buf, v)?,
        NbtTag::List(items) => {
            // 空列表元素类型写 TAG_End；非空列表由首元素决定（契约前提：元素类型一致）
            let elem_type = items.first().map(tag_id_of).unwrap_or(TAG_END);
            buf.push(elem_type);
            write_len(buf, items.len())?;
            for item in items {
                write_tag_payload(buf, item)?;
            }
        }
        NbtTag::Compound(_) => write_compound_payload(buf, tag)?,
        NbtTag::IntArray(v) => {
            write_len(buf, v.len())?;
            for item in v {
                buf.extend_from_slice(&item.to_be_bytes());
            }
        }
        NbtTag::LongArray(v) => {
            write_len(buf, v.len())?;
            for item in v {
                buf.extend_from_slice(&item.to_be_bytes());
            }
        }
    }
    Ok(())
}

/// 写出 Compound 的 entries 与 `TAG_End` 终止符（即 Compound 的 payload）。
fn write_compound_payload(buf: &mut Vec<u8>, tag: &NbtTag) -> Result<(), NbtError> {
    let NbtTag::Compound(entries) = tag else {
        return Err(NbtError::InvalidListType);
    };
    for (name, value) in entries {
        buf.push(tag_id_of(value));
        write_string(buf, name)?;
        write_tag_payload(buf, value)?;
    }
    buf.push(TAG_END);
    Ok(())
}

/// 以 VarInt 写入 `usize` 长度（网络 NBT 长度前缀为 VarInt）。
fn write_len(buf: &mut Vec<u8>, len: usize) -> Result<(), NbtError> {
    // 长度超出 i32 范围（>2GiB 的字符串/数组）视为不可编码
    let len = i32::try_from(len).map_err(|_| NbtError::UnexpectedEof)?;
    varint::write_varint(buf, len);
    Ok(())
}

/// 写入 VarInt 字节长度前缀 + UTF-8 字节（NBT 字符串）。
fn write_string(buf: &mut Vec<u8>, s: &str) -> Result<(), NbtError> {
    write_len(buf, s.len())?;
    buf.extend_from_slice(s.as_bytes());
    Ok(())
}

/// 读取 1 字节并推进游标。
fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, NbtError> {
    let b = data.get(*pos).copied().ok_or(NbtError::UnexpectedEof)?;
    *pos += 1;
    Ok(b)
}

/// 读取恰好 `len` 字节的切片并推进游标；不足返回 [`NbtError::UnexpectedEof`]。
fn read_exact<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], NbtError> {
    let end = pos.checked_add(len).ok_or(NbtError::UnexpectedEof)?;
    let slice = data.get(*pos..end).ok_or(NbtError::UnexpectedEof)?;
    *pos = end;
    Ok(slice)
}

/// 读取恰好 `N` 字节数组（大端数值的底层字节）。
fn read_array<const N: usize>(data: &[u8], pos: &mut usize) -> Result<[u8; N], NbtError> {
    let slice = read_exact(data, pos, N)?;
    slice.try_into().map_err(|_| NbtError::UnexpectedEof)
}

/// 读取 VarInt 长度并转为 `usize`。
///
/// 负数（`usize` 溢出）或声明长度超过剩余字节数均视为畸形输入直接拒绝，
/// 后者避免基于超大长度做内存预分配。
fn read_len(data: &[u8], pos: &mut usize) -> Result<usize, NbtError> {
    let len = varint::read_varint(data, pos).map_err(|_| NbtError::UnexpectedEof)?;
    let len = usize::try_from(len).map_err(|_| NbtError::UnexpectedEof)?;
    let remaining = data.len().saturating_sub(*pos);
    if len > remaining {
        return Err(NbtError::UnexpectedEof);
    }
    Ok(len)
}

/// 读取 NBT 字符串（VarInt 字节长度 + UTF-8）。
fn read_string(data: &[u8], pos: &mut usize) -> Result<String, NbtError> {
    let len = read_len(data, pos)?;
    let s = read_exact(data, pos, len)?;
    std::str::from_utf8(s)
        .map(str::to_owned)
        .map_err(|_| NbtError::UnexpectedEof)
}

/// 读取 tag 类型 id 并校验其在 1..=12 范围内。
fn read_tag_id(data: &[u8], pos: &mut usize) -> Result<u8, NbtError> {
    let id = read_u8(data, pos)?;
    if id == TAG_END || id > MAX_TAG_ID {
        return Err(NbtError::UnknownTagId(id));
    }
    Ok(id)
}

/// 按 `tag_id` 读取对应类型的 payload。
fn read_payload(tag_id: u8, data: &[u8], pos: &mut usize) -> Result<NbtTag, NbtError> {
    match tag_id {
        TAG_BYTE => Ok(NbtTag::Byte(i8::from_be_bytes(read_array(data, pos)?))),
        TAG_SHORT => Ok(NbtTag::Short(i16::from_be_bytes(read_array(data, pos)?))),
        TAG_INT => Ok(NbtTag::Int(i32::from_be_bytes(read_array(data, pos)?))),
        TAG_LONG => Ok(NbtTag::Long(i64::from_be_bytes(read_array(data, pos)?))),
        TAG_FLOAT => Ok(NbtTag::Float(f32::from_be_bytes(read_array(data, pos)?))),
        TAG_DOUBLE => Ok(NbtTag::Double(f64::from_be_bytes(read_array(data, pos)?))),
        TAG_BYTE_ARRAY => {
            let len = read_len(data, pos)?;
            Ok(NbtTag::ByteArray(read_exact(data, pos, len)?.to_vec()))
        }
        TAG_STRING => Ok(NbtTag::String(read_string(data, pos)?)),
        TAG_LIST => read_list(data, pos),
        TAG_COMPOUND => read_compound(data, pos),
        TAG_INT_ARRAY => {
            let len = read_len(data, pos)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(i32::from_be_bytes(read_array(data, pos)?));
            }
            Ok(NbtTag::IntArray(values))
        }
        TAG_LONG_ARRAY => {
            let len = read_len(data, pos)?;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(i64::from_be_bytes(read_array(data, pos)?));
            }
            Ok(NbtTag::LongArray(values))
        }
        _ => Err(NbtError::UnknownTagId(tag_id)),
    }
}

/// 读取 List：元素类型 id + 长度（VarInt）+ 各元素 payload。
fn read_list(data: &[u8], pos: &mut usize) -> Result<NbtTag, NbtError> {
    let elem_type = read_u8(data, pos)?;
    let len = read_len(data, pos)?;
    if len == 0 {
        return Ok(NbtTag::List(Vec::new()));
    }
    if elem_type == TAG_END {
        return Err(NbtError::InvalidListType);
    }
    if elem_type > MAX_TAG_ID {
        return Err(NbtError::UnknownTagId(elem_type));
    }
    let mut items = Vec::with_capacity(len);
    for _ in 0..len {
        items.push(read_payload(elem_type, data, pos)?);
    }
    Ok(NbtTag::List(items))
}

/// 读取 Compound：循环条目（type + name + payload）直到 `TAG_End`。
fn read_compound(data: &[u8], pos: &mut usize) -> Result<NbtTag, NbtError> {
    let mut entries = Vec::new();
    loop {
        let id = read_u8(data, pos)?;
        if id == TAG_END {
            return Ok(NbtTag::Compound(entries));
        }
        if id > MAX_TAG_ID {
            return Err(NbtError::UnknownTagId(id));
        }
        let name = read_string(data, pos)?;
        let value = read_payload(id, data, pos)?;
        entries.push((name, value));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// 覆盖 12 种基础类型的示例 Compound。
    fn sample_compound() -> NbtTag {
        NbtTag::Compound(vec![
            ("byte".into(), NbtTag::Byte(-5)),
            ("short".into(), NbtTag::Short(-300)),
            ("int".into(), NbtTag::Int(1_000_000)),
            ("long".into(), NbtTag::Long(-9_000_000_000)),
            ("float".into(), NbtTag::Float(3.5)),
            ("double".into(), NbtTag::Double(123.456)),
            ("byte_array".into(), NbtTag::ByteArray(vec![0, 1, 2, 255])),
            ("string".into(), NbtTag::String("hello 世界".into())),
            (
                "list".into(),
                NbtTag::List(vec![NbtTag::Int(1), NbtTag::Int(2), NbtTag::Int(3)]),
            ),
            (
                "compound".into(),
                NbtTag::Compound(vec![("nested".into(), NbtTag::Int(42))]),
            ),
            ("int_array".into(), NbtTag::IntArray(vec![1, -2, 3])),
            ("long_array".into(), NbtTag::LongArray(vec![1, -2, 3])),
        ])
    }

    #[test]
    fn basic_12_types_roundtrip() {
        let tag = sample_compound();
        let bytes = encode_root(&tag).unwrap();
        assert_eq!(decode_root(&bytes).unwrap(), (String::new(), tag));
    }

    #[test]
    fn empty_compound_encoding() {
        assert_eq!(
            encode_root(&NbtTag::Compound(vec![])).unwrap(),
            vec![0x0a, 0x00, 0x00]
        );
        assert_eq!(
            encode_anonymous(&NbtTag::Compound(vec![])).unwrap(),
            vec![0x00]
        );
    }

    #[test]
    fn encode_root_header() {
        let tag = NbtTag::Compound(vec![("x".into(), NbtTag::Int(1))]);
        let bytes = encode_root(&tag).unwrap();
        assert_eq!(bytes.first(), Some(&0x0a));
        assert_eq!(bytes.get(1), Some(&0x00));
    }

    #[test]
    fn encode_anonymous_no_root_header() {
        let tag = sample_compound();
        let root = encode_root(&tag).unwrap();
        let anonymous = encode_anonymous(&tag).unwrap();
        // anonymous 即 root 去掉 [0x0a, 0x00] 后的 payload
        assert_eq!(anonymous, root.get(2..).unwrap());
        // 补回根头后应能完整解码
        let mut full = vec![0x0a, 0x00];
        full.extend_from_slice(&anonymous);
        assert_eq!(decode_root(&full).unwrap(), (String::new(), tag));
    }

    #[test]
    fn decode_anonymous_roundtrip() {
        // decode_anonymous 是 encode_anonymous 的严格对称：先编码再解码应还原 Compound。
        let tag = sample_compound();
        let anonymous = encode_anonymous(&tag).unwrap();
        let (decoded, consumed) = decode_anonymous(&anonymous).unwrap();
        assert_eq!(decoded, tag);
        // 消费字节数应等于整个 anonymous payload 长度（Compound 自界定到 TAG_End）。
        assert_eq!(consumed, anonymous.len());
    }

    #[test]
    fn decode_anonymous_consumed_with_trailing() {
        // 模拟 SystemChatPacket 布局：anonymous NBT 后还跟着 1 字节 overlay。
        // decode_anonymous 只消费 NBT 部分，不越界到尾部字段。
        let compound = NbtTag::Compound(vec![("text".into(), NbtTag::String("hi".into()))]);
        let mut bytes = encode_anonymous(&compound).unwrap();
        bytes.push(0x01); // overlay = true
        let (tag, consumed) = decode_anonymous(&bytes).unwrap();
        assert_eq!(tag, compound);
        assert_eq!(consumed, bytes.len() - 1);
    }

    #[test]
    fn decode_anonymous_truncated() {
        // 缺少 TAG_End 终止符的残缺 Compound → UnexpectedEof。
        let mut bytes = vec![0x08]; // TAG_String
        bytes.extend_from_slice(&[0x00, 0x01]); // 长度 1
        // 字符串字节缺失 → 截断
        assert_eq!(decode_anonymous(&bytes), Err(NbtError::UnexpectedEof));
    }

    #[test]
    fn nested_compound_list_roundtrip() {
        let inner = NbtTag::Compound(vec![
            ("name".into(), NbtTag::String("sword".into())),
            ("damage".into(), NbtTag::Int(7)),
            (
                "enchants".into(),
                NbtTag::List(vec![
                    NbtTag::Compound(vec![
                        ("id".into(), NbtTag::Short(16)),
                        ("lvl".into(), NbtTag::Short(2)),
                    ]),
                    NbtTag::Compound(vec![
                        ("id".into(), NbtTag::Short(5)),
                        ("lvl".into(), NbtTag::Short(1)),
                    ]),
                ]),
            ),
        ]);
        let outer = NbtTag::Compound(vec![
            (
                "id".into(),
                NbtTag::String("minecraft:diamond_sword".into()),
            ),
            ("tag".into(), inner),
            (
                "extra".into(),
                NbtTag::List(vec![NbtTag::Double(1.5), NbtTag::Double(2.5)]),
            ),
        ]);
        let bytes = encode_root(&outer).unwrap();
        assert_eq!(decode_root(&bytes).unwrap(), (String::new(), outer));
    }

    #[test]
    fn empty_list_roundtrip() {
        let tag = NbtTag::Compound(vec![("empty".into(), NbtTag::List(vec![]))]);
        let bytes = encode_root(&tag).unwrap();
        assert_eq!(decode_root(&bytes).unwrap(), (String::new(), tag));
    }

    #[test]
    fn unknown_tag_id() {
        assert_eq!(decode_root(&[0x00]), Err(NbtError::UnknownTagId(0)));
        assert_eq!(decode_root(&[0x0d]), Err(NbtError::UnknownTagId(13)));
        assert_eq!(decode_root(&[0xff]), Err(NbtError::UnknownTagId(255)));
    }

    #[test]
    fn truncated_input() {
        assert_eq!(decode_root(&[]), Err(NbtError::UnexpectedEof));
        // 缺根名长度字节
        assert_eq!(decode_root(&[0x0a]), Err(NbtError::UnexpectedEof));
        // 根名声明长度 5 但无内容
        assert_eq!(decode_root(&[0x0a, 0x05]), Err(NbtError::UnexpectedEof));
        // 条目类型为 Byte 但缺 name 长度字节
        assert_eq!(
            decode_root(&[0x0a, 0x00, 0x01]),
            Err(NbtError::UnexpectedEof)
        );
    }

    #[test]
    fn invalid_list_type() {
        // 非空 List 声明元素类型 TAG_End(0)
        assert_eq!(
            decode_root(&[0x0a, 0x00, 0x09, 0x00, 0x00, 0x02, 0x00, 0x00]),
            Err(NbtError::InvalidListType)
        );
    }

    #[test]
    fn list_unknown_element_type() {
        // List 元素类型 13（未知），声明长度 1
        assert_eq!(
            decode_root(&[0x0a, 0x00, 0x09, 0x00, 0x0d, 0x01, 0x00]),
            Err(NbtError::UnknownTagId(13))
        );
    }

    #[test]
    fn negative_list_length() {
        // List 长度 VarInt 为 -1（5 字节编码）
        let bytes = [0x0a, 0x00, 0x09, 0x01, 0xff, 0xff, 0xff, 0xff, 0x0f];
        assert_eq!(decode_root(&bytes), Err(NbtError::UnexpectedEof));
    }

    #[test]
    fn invalid_utf8_string() {
        // 字符串 payload 长度为 1 但字节为非法 UTF-8
        let bytes = [0x0a, 0x00, 0x08, 0x00, 0x01, 0xff];
        assert_eq!(decode_root(&bytes), Err(NbtError::UnexpectedEof));
    }

    #[test]
    fn truncated_array() {
        // IntArray 声明 4 个元素但只提供 2 个
        let bytes = [
            0x0a, 0x00, 0x0b, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02,
        ];
        assert_eq!(decode_root(&bytes), Err(NbtError::UnexpectedEof));
    }

    #[test]
    fn encode_root_rejects_non_compound() {
        assert_eq!(encode_root(&NbtTag::Int(5)), Err(NbtError::InvalidListType));
        assert_eq!(
            encode_anonymous(&NbtTag::Byte(1)),
            Err(NbtError::InvalidListType)
        );
    }

    #[test]
    fn display_messages() {
        assert_eq!(
            format!("{}", NbtError::UnexpectedEof),
            "NBT 数据不足：输入意外结束"
        );
        assert_eq!(
            format!("{}", NbtError::UnknownTagId(9)),
            "未知 NBT tag 类型 id：9"
        );
        assert_eq!(
            format!("{}", NbtError::InvalidListType),
            "NBT 列表元素类型非法"
        );
    }
}

/// 模糊测试入口（仅 `cargo fuzz` 构建启用，由 `#[cfg(fuzzing)]` 控制）。
///
/// 对 NBT 解码 [`decode_root`] / [`decode_anonymous`] 喂入任意字节，确认所有畸形 /
/// 截断 / 未知 tag id 输入均返回 [`NbtError`]，绝不 panic。NBT 是经典模糊测试目标，
/// 解析器须对一切敌意输入稳健（递归 Compound/List 亦受输入长度约束，无栈爆炸风险）。
#[cfg(fuzzing)]
pub fn fuzz_target_nbt(data: &[u8]) {
    let _ = decode_root(data);
    let _ = decode_anonymous(data);
}
