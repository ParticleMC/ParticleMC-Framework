//! 登录阶段数据包（离线模式：跳过加密请求）。

use uuid::Uuid;

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::packet::Packet;
use crate::protocol::packets::Property;

/// 登录 Hello（serverbound, id 0x00）。
///
/// 1.20.5+ 格式：Name(String) + HasUuid(Boolean) + UUID(可选 128 位)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// 玩家名。
    pub name: String,
    /// 客户端携带的 UUID（现代离线模式由客户端发送；缺失时由服务端按名派生）。
    pub uuid: Option<Uuid>,
}

impl Packet for Hello {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let name = buf.get_string()?;
        let has_uuid = buf.get_bool()?;
        let uuid = if has_uuid {
            Some(buf.get_uuid()?)
        } else {
            None
        };
        Ok(Hello { name, uuid })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.name);
        match &self.uuid {
            Some(u) => {
                buf.put_bool(true);
                buf.put_uuid(*u);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 登录断开（clientbound, id 0x00），reason 为 JSON 聊天组件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginDisconnect {
    pub reason: String,
}

impl Packet for LoginDisconnect {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(LoginDisconnect {
            reason: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.reason);
        Ok(())
    }
}

/// 登录成功（clientbound, id 0x02）。
///
/// 字段：UUID(128 位) + Username(String) + Properties(VarInt 计数 + 条目)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSuccess {
    pub uuid: Uuid,
    pub username: String,
    /// 皮肤 / 皮肤签名等属性（离线模式通常为空；Velocity 转发时由代理提供）。
    pub properties: Vec<Property>,
}

impl Packet for LoginSuccess {
    fn packet_id(&self) -> i32 {
        0x02
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let uuid = buf.get_uuid()?;
        let username = buf.get_string()?;
        let count = buf.get_varint()?;
        let count_usize = usize::try_from(count).map_err(|_| ProtocolError::UnexpectedEof)?;
        let mut properties = Vec::with_capacity(count_usize);
        for _ in 0..count_usize {
            let name = buf.get_string()?;
            let value = buf.get_string()?;
            let has_sig = buf.get_bool()?;
            let signature = if has_sig {
                Some(buf.get_string()?)
            } else {
                None
            };
            properties.push(Property {
                name,
                value,
                signature,
            });
        }
        Ok(LoginSuccess {
            uuid,
            username,
            properties,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.uuid);
        buf.put_string(&self.username);
        buf.put_varint(self.properties.len() as i32);
        for p in &self.properties {
            buf.put_string(&p.name);
            buf.put_string(&p.value);
            match &p.signature {
                Some(s) => {
                    buf.put_bool(true);
                    buf.put_string(s);
                }
                None => buf.put_bool(false),
            }
        }
        Ok(())
    }
}

/// 登录压缩（clientbound, id 0x03）。
///
/// 字段：Threshold(VarInt)。客户端收到后启用压缩，后续收发均走压缩帧格式
/// （见 [`crate::protocol::framing::encode_frame_compressed`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginCompression {
    pub threshold: i32,
}

impl Packet for LoginCompression {
    fn packet_id(&self) -> i32 {
        0x03
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(LoginCompression {
            threshold: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.threshold);
        Ok(())
    }
}

/// 登录挑战响应（serverbound, id 0x01）。
///
/// 在线模式：客户端回传的加密共享密钥与验证 token。
/// 字段：SharedSecretLength(VarInt) + SharedSecret(Bytes) + VerifyTokenLength(VarInt) + VerifyToken(16 bytes)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginChallenge {
    /// 加密后的共享密钥（AES-CBC 加密后的 RSA 加密结果）。
    pub shared_secret: Vec<u8>,
    /// 验证 token（16 字节，AES-CFB8 加密）。
    pub verify_token: Vec<u8>,
}

impl Packet for LoginChallenge {
    fn packet_id(&self) -> i32 {
        0x01
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let secret_len = buf.get_varint()?;
        let secret_len_usize =
            usize::try_from(secret_len).map_err(|_| ProtocolError::UnexpectedEof)?;
        let shared_secret = buf.get_bytes(secret_len_usize)?;

        let token_len = buf.get_varint()?;
        let token_len_usize =
            usize::try_from(token_len).map_err(|_| ProtocolError::UnexpectedEof)?;
        let verify_token = buf.get_bytes(token_len_usize)?;

        Ok(LoginChallenge {
            shared_secret,
            verify_token,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.shared_secret.len() as i32);
        buf.put_bytes(&self.shared_secret);
        buf.put_varint(self.verify_token.len() as i32);
        buf.put_bytes(&self.verify_token);
        Ok(())
    }
}

/// 登录 Hello 响应（clientbound, id 0x01）。
///
/// 在线模式：服务端下发公钥与验证 token，等待客户端回传挑战响应。
/// 字段：PublicKeyLength(VarInt) + PublicKey(Bytes) + VerifyTokenLength(VarInt) + VerifyToken(16 bytes)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginHelloResponse {
    /// 服务端 RSA 公钥字节。
    pub public_key: Vec<u8>,
    /// 验证 token（16 字节，用于后续 challenge 验证）。
    pub verify_token: Vec<u8>,
}

impl Packet for LoginHelloResponse {
    fn packet_id(&self) -> i32 {
        0x01
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let key_len = buf.get_varint()?;
        let key_len_usize = usize::try_from(key_len).map_err(|_| ProtocolError::UnexpectedEof)?;
        let public_key = buf.get_bytes(key_len_usize)?;

        let token_len = buf.get_varint()?;
        let token_len_usize =
            usize::try_from(token_len).map_err(|_| ProtocolError::UnexpectedEof)?;
        let verify_token = buf.get_bytes(token_len_usize)?;

        Ok(LoginHelloResponse {
            public_key,
            verify_token,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.public_key.len() as i32);
        buf.put_bytes(&self.public_key);
        buf.put_varint(self.verify_token.len() as i32);
        buf.put_bytes(&self.verify_token);
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
    fn hello_roundtrip_without_uuid() {
        // 旧式离线模式：客户端不携带 UUID。
        roundtrip(&Hello {
            name: "Steve".to_string(),
            uuid: None,
        });
    }

    #[test]
    fn hello_roundtrip_with_uuid() {
        // 现代离线模式：客户端携带 UUID。
        roundtrip(&Hello {
            name: "Alex".to_string(),
            uuid: Some(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef)),
        });
    }

    #[test]
    fn login_success_roundtrip_empty_properties() {
        // 离线模式通常无属性。
        roundtrip(&LoginSuccess {
            uuid: Uuid::from_u128(0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff),
            username: "Steve".to_string(),
            properties: Vec::new(),
        });
    }

    #[test]
    fn login_success_roundtrip_with_properties() {
        // Velocity 转发场景：带签名与不带签名的属性。
        roundtrip(&LoginSuccess {
            uuid: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            username: "Alex".to_string(),
            properties: vec![
                Property {
                    name: "textures".to_string(),
                    value: "eyJ0ZXh0dXJlcyI6e319".to_string(),
                    signature: Some("c2lnbmF0dXJl".to_string()),
                },
                Property {
                    name: "preferredLanguage".to_string(),
                    value: "zh_cn".to_string(),
                    signature: None,
                },
            ],
        });
    }

    #[test]
    fn login_compression_roundtrip() {
        roundtrip(&LoginCompression { threshold: 256 });
    }

    #[test]
    fn login_disconnect_roundtrip() {
        // reason 为 JSON 聊天组件。
        roundtrip(&LoginDisconnect {
            reason: r#"{"text":"服务器维护中"}"#.to_string(),
        });
    }

    #[test]
    fn login_hello_response_roundtrip() {
        // 服务端发 LoginHello 响应：公钥 + verify_token。
        roundtrip(&LoginHelloResponse {
            public_key: vec![1, 2, 3, 4, 5],
            verify_token: vec![0u8; 16],
        });
    }

    #[test]
    fn login_challenge_roundtrip() {
        // 客户端回传 LoginChallenge：共享密钥 + verify_token。
        roundtrip(&LoginChallenge {
            shared_secret: vec![0xaa, 0xbb, 0xcc],
            verify_token: vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        });
    }
}
