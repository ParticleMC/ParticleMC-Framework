// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 握手阶段（serverbound）数据包。

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::packet::Packet;

/// 握手意图包（Handshake, serverbound, id 0x00）。
///
/// 在 `next_state` 之后容错读取 Velocity modern forwarding blob：若帧内还有
/// 剩余字节，读取 VarInt 标志（1=有转发），随后 VarInt 长度 + 对应长度字节数组。
/// 若已到帧末尾则视为无转发（直连）。EOF 不报错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intention {
    /// 客户端协议版本（1.21.11 = 774）。
    pub protocol_version: i32,
    /// 服务器地址（可能含 Velocity 旧式转发分隔符，本实现按现代转发处理）。
    pub server_address: String,
    /// 服务器端口。
    pub port: u16,
    /// 意图状态：1=Status，2=Login。
    pub next_state: i32,
    /// Velocity modern forwarding blob（现代转发模式下由代理附加）。
    pub forwarding: Option<Vec<u8>>,
}

impl Packet for Intention {
    fn packet_id(&self) -> i32 {
        0x00
    }

    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let protocol_version = buf.get_varint()?;
        let server_address = buf.get_string()?;
        let port = buf.get_u16()?;
        let next_state = buf.get_varint()?;
        let forwarding = if buf.remaining() > 0 {
            let marker = buf.get_varint()?;
            if marker == 1 {
                let len = buf.get_varint()?;
                let len_usize = usize::try_from(len).map_err(|_| ProtocolError::UnexpectedEof)?;
                Some(buf.get_bytes(len_usize)?)
            } else {
                None
            }
        } else {
            None
        };
        Ok(Intention {
            protocol_version,
            server_address,
            port,
            next_state,
            forwarding,
        })
    }

    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.protocol_version);
        buf.put_string(&self.server_address);
        buf.put_u16(self.port);
        buf.put_varint(self.next_state);
        if let Some(blob) = &self.forwarding {
            buf.put_varint(1);
            buf.put_varint(blob.len() as i32);
            buf.put_bytes(blob);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// encode → decode 往返后应与原值一致。
    fn roundtrip<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let mut buf = ByteBuffer::with_capacity(64);
        p.encode(&mut buf).unwrap();
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = P::decode(&mut buf).unwrap();
        assert_eq!(*p, decoded);
    }

    #[test]
    fn intention_roundtrip_without_forwarding() {
        // 直连场景：无 Velocity 转发 blob。
        roundtrip(&Intention {
            protocol_version: 774,
            server_address: "localhost".to_string(),
            port: 25565,
            next_state: 2,
            forwarding: None,
        });
    }

    #[test]
    fn intention_roundtrip_with_forwarding() {
        // 代理转发场景：带现代 forwarding blob。
        roundtrip(&Intention {
            protocol_version: 774,
            server_address: "localhost".to_string(),
            port: 25565,
            next_state: 2,
            forwarding: Some(vec![0xde, 0xad, 0xbe, 0xef, 0x01, 0x02]),
        });
    }
}
