//! 数据包编解码 trait（骨架占位）。
//!
//! 真实协议编解码将在后续增量实现；骨架阶段仅声明接口，默认实现一律返回
//! [`CodecError::NotImplemented`]，明确标识该能力尚未落地。

use std::fmt;

/// 编解码错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// 编解码能力尚未实现（骨架阶段）。
    NotImplemented,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::NotImplemented => write!(f, "数据包编解码尚未实现"),
        }
    }
}

impl std::error::Error for CodecError {}

/// 数据包编解码器（骨架占位）。
pub trait PacketCodec {
    /// 编码出站数据包（骨架阶段返回 [`CodecError::NotImplemented`]）。
    fn encode(&self, packet: &[u8]) -> Result<Vec<u8>, CodecError> {
        let _ = (self, packet);
        Err(CodecError::NotImplemented)
    }

    /// 解码入站字节流（骨架阶段返回 [`CodecError::NotImplemented`]）。
    fn decode(&self, data: &[u8]) -> Result<Vec<u8>, CodecError> {
        let _ = (self, data);
        Err(CodecError::NotImplemented)
    }
}
