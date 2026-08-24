//! 数据包 trait：每个状态的数据包实现编码 / 解码与 packet id 查询。

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;

/// 一个 Minecraft 数据包。
///
/// 实现者不得 panic；未知字段应忽略或存入 `extra`。`decode` 读取的是不含
/// packet_id 的包体（packet_id 由派发层 [`super::dispatch`] 处理）。
pub trait Packet {
    /// 该包的 packet id（按 1.21.11 协议）。
    fn packet_id(&self) -> i32;

    /// 从读游标解码包体。
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError>
    where
        Self: Sized;

    /// 将包体编码进写游标。
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError>;
}
