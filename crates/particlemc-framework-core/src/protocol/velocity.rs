// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Velocity modern forwarding（1.20.3+ 握手转发）校验。
//!
//! 经 Velocity 代理转发的玩家，其握手包末尾附加一段二进制转发数据（blob），
//! 由代理用共享密钥做 HMAC-SHA256 签名。本模块校验签名真伪并解析出玩家的
//! 真实 UUID 与皮肤属性，使后端获得与代理一致的用户身份。
//!
//! blob 布局（与 `protocol::packets::Intention` 的 `forwarding` 字段对应）：
//! `version(u8=1) + signature([u8;32]) + payload`，
//! `payload = uuid([u8;16]) + name(String) + timestamp(i64) + properties(Vec)`。
//! 签名为对 `version ++ payload` 的 HMAC-SHA256。

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::packets::Property;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// 时间戳最大允许偏差（毫秒）。
pub const MAX_SKEW: i64 = 5_000;

/// 校验失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VelocityError {
    /// blob 格式损坏（长度不足 / 版本不符）。
    Malformed,
    /// 签名错误（密钥不匹配或 blob 被篡改）。
    BadSignature,
    /// 时间戳与当前相差超过 [`MAX_SKEW`]。
    Expired,
}

/// 经 Velocity 转发解析出的玩家真实身份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedIdentity {
    /// 玩家真实 UUID（由 Mojang / 代理提供）。
    pub uuid: Uuid,
    /// 玩家名。
    pub name: String,
    /// 皮肤等属性。
    pub properties: Vec<Property>,
    /// 转发时间戳（毫秒）。
    pub timestamp: i64,
}

/// 校验转发 blob 并返回玩家身份。
///
/// - 用 `secret` 的 UTF-8 字节重算 HMAC-SHA256（覆盖 `version ++ payload`），
///   与 blob 中 `signature` 做常量时间比较（防时序攻击）。
/// - 解析 `payload` 得到 UUID / 名 / 属性。
/// - 校验 `timestamp` 与当前时间差 ≤ [`MAX_SKEW`]。
pub fn verify_forwarding(secret: &[u8], blob: &[u8]) -> Result<ForwardedIdentity, VelocityError> {
    let mut buf = ByteBuffer::new(blob.to_vec());
    let version = buf.get_u8().map_err(|_| VelocityError::Malformed)?;
    if version != 1 {
        return Err(VelocityError::Malformed);
    }
    let signature = buf.get_bytes(32).map_err(|_| VelocityError::Malformed)?;
    let payload = buf.as_slice()[buf.position()..].to_vec();

    // 重算签名：HMAC-SHA256(secret, version ++ payload)
    let mut data = Vec::with_capacity(payload.len() + 1);
    data.push(version);
    data.extend_from_slice(&payload);
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return Err(VelocityError::Malformed),
    };
    mac.update(&data);
    mac.verify_slice(&signature)
        .map_err(|_| VelocityError::BadSignature)?;

    // 解析 payload
    let mut p = ByteBuffer::new(payload);
    let uuid = p.get_uuid().map_err(|_| VelocityError::Malformed)?;
    let name = p.get_string().map_err(|_| VelocityError::Malformed)?;
    let timestamp = p.get_i64().map_err(|_| VelocityError::Malformed)?;
    let prop_count = p.get_varint().map_err(|_| VelocityError::Malformed)?;
    let prop_count_usize = usize::try_from(prop_count).map_err(|_| VelocityError::Malformed)?;
    let mut properties = Vec::with_capacity(prop_count_usize);
    for _ in 0..prop_count_usize {
        let pn = p.get_string().map_err(|_| VelocityError::Malformed)?;
        let pv = p.get_string().map_err(|_| VelocityError::Malformed)?;
        let has_sig = p.get_bool().map_err(|_| VelocityError::Malformed)?;
        let psig = if has_sig {
            Some(p.get_string().map_err(|_| VelocityError::Malformed)?)
        } else {
            None
        };
        properties.push(Property {
            name: pn,
            value: pv,
            signature: psig,
        });
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| VelocityError::Malformed)?
        .as_millis() as i64;
    if (now - timestamp).abs() > MAX_SKEW {
        return Err(VelocityError::Expired);
    }

    Ok(ForwardedIdentity {
        uuid,
        name,
        properties,
        timestamp,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use hmac::Hmac;
    use sha2::Sha256;

    fn make_blob(secret: &[u8], uuid: Uuid, name: &str, age_ms: i64) -> Vec<u8> {
        // payload
        let mut p = ByteBuffer::with_capacity(64);
        p.put_uuid(uuid);
        p.put_string(name);
        p.put_i64(age_ms);
        p.put_varint(0); // 0 properties
        let payload = p.into_inner();
        // signature = HMAC(version ++ payload)
        let mut data = Vec::new();
        data.push(1u8);
        data.extend_from_slice(&payload);
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(&data);
        let sig = mac.finalize().into_bytes().to_vec();
        // blob = version + signature + payload
        let mut blob = Vec::new();
        blob.push(1u8);
        blob.extend_from_slice(&sig);
        blob.extend_from_slice(&payload);
        blob
    }

    #[test]
    fn valid_forwarding_accepted() {
        let secret = b"super-secret";
        let uuid = Uuid::nil();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let blob = make_blob(secret, uuid, "Steve", now);
        let id = verify_forwarding(secret, &blob).unwrap();
        assert_eq!(id.uuid, uuid);
        assert_eq!(id.name, "Steve");
    }

    #[test]
    fn wrong_secret_rejected() {
        let uuid = Uuid::nil();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let blob = make_blob(b"secret-a", uuid, "Steve", now);
        assert_eq!(
            verify_forwarding(b"secret-b", &blob),
            Err(VelocityError::BadSignature)
        );
    }

    #[test]
    fn expired_rejected() {
        let secret = b"secret";
        let uuid = Uuid::nil();
        let old = 0i64; // 1970
        let blob = make_blob(secret, uuid, "Steve", old);
        assert_eq!(
            verify_forwarding(secret, &blob),
            Err(VelocityError::Expired)
        );
    }

    #[test]
    fn malformed_rejected() {
        assert_eq!(
            verify_forwarding(b"secret", b"garbage"),
            Err(VelocityError::Malformed)
        );
    }
}

/// 模糊测试入口（仅 `cargo fuzz` 构建启用，由 `#[cfg(fuzzing)]` 控制）。
///
/// 对 Velocity 转发校验 [`verify_forwarding`] 喂入任意 blob 字节，确认所有畸形 /
/// 篡改 / 过期输入均返回 [`VelocityError`]，绝不 panic。使用固定测试密钥，使模糊
/// 引擎可探索 HMAC 签名校验与 payload 解析（UUID / 名 / 时间戳 / 属性）路径。
#[cfg(fuzzing)]
pub fn fuzz_target_velocity(data: &[u8]) {
    let secret = b"fuzz-velocity-secret";
    let _ = verify_forwarding(secret, data);
}
