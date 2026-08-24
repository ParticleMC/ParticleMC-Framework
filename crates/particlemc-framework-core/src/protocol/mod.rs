//! 协议层：变长整数、字节缓冲、帧封装、数据包 trait 与派发。
//!
//! - [`error`]：协议错误类型（每个变体可被单测命中）。
//! - [`varint`]：VarInt / VarLong 编解码。
//! - [`byte_buf`]：游标式字节缓冲。
//! - [`framing`]：帧（VarInt 长度 + payload）封装。
//! - [`nbt`]：NBT 网络格式编解码。
//! - [`packet`]：数据包 trait。
//! - [`packets`]：各连接状态的最小真实数据包（T2）。
//! - [`dispatch`]：按 `(ConnectionState, packet_id)` 派发（T2）。

pub mod byte_buf;
pub mod dispatch;
pub mod error;
pub mod framing;
pub mod nbt;
pub mod packet;
pub mod packets;
pub mod varint;
pub mod velocity;

pub use velocity::{ForwardedIdentity, VelocityError, verify_forwarding};
