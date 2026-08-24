//! 协议层错误类型。
//!
//! 每个变体都应由单元测试命中（Err 分支全覆盖），以保证编解码失败时不会 panic，
//! 而是返回可被上层记录并忽略的错误。

use std::fmt;

/// 协议编解码错误。
#[derive(Debug)]
pub enum ProtocolError {
    /// VarInt/VarLong 超过最大允许字节数。
    VarIntTooLong,
    /// 读游标越过缓冲区末尾。
    UnexpectedEof,
    /// 帧声明长度超过 [`super::framing::MAX_FRAME`]。
    FrameTooLarge,
    /// 字符串 UTF-8 解码失败。
    Utf8,
    /// 状态机当前状态下出现未定义的 packet id。
    UnknownPacket { state: &'static str, id: i32 },
    /// 连接状态非法（例如在不允许的状态收到某包）。
    InvalidState,
    /// 压缩相关错误（当前离线模式未启用压缩，保留占位）。
    Compression,
    /// 底层 IO 错误（包装 `std::io::Error`）。
    Io(std::io::Error),
    /// 协议值超出合法范围（例如数量 / 物品 id 越界或为负）。
    InvalidValue,
    /// 数据组件 patch 不被当前实现支持（v1 仅接受空 patch）。
    UnsupportedComponents,
}

impl PartialEq for ProtocolError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ProtocolError::VarIntTooLong, ProtocolError::VarIntTooLong) => true,
            (ProtocolError::UnexpectedEof, ProtocolError::UnexpectedEof) => true,
            (ProtocolError::FrameTooLarge, ProtocolError::FrameTooLarge) => true,
            (ProtocolError::Utf8, ProtocolError::Utf8) => true,
            (
                ProtocolError::UnknownPacket { state: s1, id: i1 },
                ProtocolError::UnknownPacket { state: s2, id: i2 },
            ) => s1 == s2 && i1 == i2,
            (ProtocolError::InvalidState, ProtocolError::InvalidState) => true,
            (ProtocolError::Compression, ProtocolError::Compression) => true,
            (ProtocolError::InvalidValue, ProtocolError::InvalidValue) => true,
            (ProtocolError::UnsupportedComponents, ProtocolError::UnsupportedComponents) => true,
            _ => false,
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::VarIntTooLong => write!(f, "VarInt 超出最大长度（5 字节）"),
            ProtocolError::UnexpectedEof => write!(f, "缓冲区越界：数据不足"),
            ProtocolError::FrameTooLarge => write!(f, "帧长度超过上限"),
            ProtocolError::Utf8 => write!(f, "UTF-8 解码失败"),
            ProtocolError::UnknownPacket { state, id } => {
                write!(f, "未知数据包：state={state} id={id}")
            }
            ProtocolError::InvalidState => write!(f, "连接状态非法"),
            ProtocolError::Compression => write!(f, "压缩相关错误"),
            ProtocolError::InvalidValue => write!(f, "协议值超出合法范围"),
            ProtocolError::UnsupportedComponents => {
                write!(f, "数据组件 patch 不被当前实现支持（v1 仅接受空 patch）")
            }
            ProtocolError::Io(e) => write!(f, "IO 错误：{e}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<std::io::Error> for ProtocolError {
    fn from(e: std::io::Error) -> Self {
        ProtocolError::Io(e)
    }
}

impl From<std::string::FromUtf8Error> for ProtocolError {
    fn from(_: std::string::FromUtf8Error) -> Self {
        ProtocolError::Utf8
    }
}

impl From<std::str::Utf8Error> for ProtocolError {
    fn from(_: std::str::Utf8Error) -> Self {
        ProtocolError::Utf8
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn display_varint_too_long() {
        assert_eq!(
            format!("{}", ProtocolError::VarIntTooLong),
            "VarInt 超出最大长度（5 字节）"
        );
    }

    #[test]
    fn display_eof() {
        assert_eq!(
            format!("{}", ProtocolError::UnexpectedEof),
            "缓冲区越界：数据不足"
        );
    }

    #[test]
    fn display_frame_too_large() {
        assert_eq!(
            format!("{}", ProtocolError::FrameTooLarge),
            "帧长度超过上限"
        );
    }

    #[test]
    fn display_utf8() {
        assert_eq!(format!("{}", ProtocolError::Utf8), "UTF-8 解码失败");
    }

    #[test]
    fn display_unknown_packet() {
        assert_eq!(
            format!(
                "{}",
                ProtocolError::UnknownPacket {
                    state: "Login",
                    id: 99
                }
            ),
            "未知数据包：state=Login id=99"
        );
    }

    #[test]
    fn display_invalid_state() {
        assert_eq!(format!("{}", ProtocolError::InvalidState), "连接状态非法");
    }

    #[test]
    fn display_compression() {
        assert_eq!(format!("{}", ProtocolError::Compression), "压缩相关错误");
    }

    #[test]
    fn display_io() {
        let e = ProtocolError::Io(std::io::Error::other("boom"));
        assert!(format!("{e}").contains("boom"));
    }

    #[test]
    fn eq_variants() {
        assert_eq!(ProtocolError::Utf8, ProtocolError::Utf8);
        assert_eq!(
            ProtocolError::UnknownPacket { state: "S", id: 1 },
            ProtocolError::UnknownPacket { state: "S", id: 1 }
        );
        assert!(ProtocolError::Utf8 != ProtocolError::UnexpectedEof);
    }

    #[test]
    fn from_utf8_error() {
        let raw = vec![0xff, 0xfe];
        let err: ProtocolError = String::from_utf8(raw).unwrap_err().into();
        assert_eq!(err, ProtocolError::Utf8);
    }
}
