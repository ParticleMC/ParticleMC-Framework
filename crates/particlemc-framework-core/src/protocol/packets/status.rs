//! 状态阶段数据包（MOTD / 版本探测）。

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::packet::Packet;

/// 状态请求（serverbound, id 0x00），无字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusRequest;

impl Packet for StatusRequest {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(StatusRequest)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 状态 Ping（serverbound, id 0x01），携带时间戳。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ping {
    /// 客户端发送的时间戳（原样回显）。
    pub payload: i64,
}

impl Packet for Ping {
    fn packet_id(&self) -> i32 {
        0x01
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Ping {
            payload: buf.get_i64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.payload);
        Ok(())
    }
}

/// 状态响应（clientbound, id 0x00），JSON 字符串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    /// 服务器状态 JSON（MOTD / 版本 / 玩家数）。
    pub json: String,
}

impl Packet for StatusResponse {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(StatusResponse {
            json: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.json);
        Ok(())
    }
}

/// 状态 Ping 响应（clientbound, id 0x01），回显时间戳。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingResponse {
    pub payload: i64,
}

impl Packet for PingResponse {
    fn packet_id(&self) -> i32 {
        0x01
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PingResponse {
            payload: buf.get_i64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.payload);
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
    fn status_response_roundtrip() {
        // MOTD JSON 往返。
        roundtrip(&StatusResponse {
            json:
                r#"{"version":{"name":"1.21.11","protocol":774},"players":{"max":20,"online":0}}"#
                    .to_string(),
        });
    }

    #[test]
    fn ping_roundtrip() {
        roundtrip(&Ping {
            payload: -123_456_789,
        });
    }

    #[test]
    fn ping_response_roundtrip() {
        roundtrip(&PingResponse {
            payload: 987_654_321,
        });
    }
}
