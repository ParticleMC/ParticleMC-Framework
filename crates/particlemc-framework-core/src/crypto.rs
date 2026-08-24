//! 在线 Mojang 认证模块（WS5b，feature `online-auth`）。
//!
//! 本模块实现 Minecraft 在线认证握手流程：
//! 1. 服务端生成 RSA-1024 密钥对（持久化于 [`OnlineAuthContext`] Resource）
//! 2. Hello 包到达时，将公钥与 challenge token 发给客户端（`LoginHelloResponse`）
//! 3. 客户端用共享密钥加密回传 challenge token（`LoginChallenge`）
//! 4. 服务端 RSA 解密验证 challenge token 匹配
//! 5. 调用 Mojang `hasJoined` API 验证玩家身份（TLS）
//!
//! # 安全注意
//!
//! - 模块始终编译（保证 `network_receive` 系统参数类型存在），但仅 `online-auth`
//!   feature 启用时 [`OnlineAuthContext::enabled`] 为 true 且装载 RSA 私钥。
//! - `rsa`/`rustls` 均为纯 Rust 依赖，feature 关闭时不引入，默认构建离线语义零开销。
//! - 本模块不暴露玩家私钥；challenge token 仅用于握手阶段验证。

use std::time::Duration;

#[cfg(feature = "online-auth")]
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
#[cfg(feature = "online-auth")]
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

/// 待 hasJoined 异步验证的会话项。
///
/// 由 `network_receive`（同步）在 RSA 握手成功后入队，交由常驻
/// [`run_auth_worker`] 异步任务调用 Mojang 并下发 `LoginSuccess`/`LoginDisconnect`。
#[derive(Debug, Clone)]
pub struct PendingAuth {
    /// 连接 id。
    pub conn_id: u32,
    /// 玩家名（来自 Hello 解析）。
    pub username: String,
    /// 玩家 UUID（来自 Hello 解析；Mojang 验证后应与档案一致）。
    pub uuid: Uuid,
    /// RSA 解密得到的 16 字节共享密钥（AES-128-CFB8 流密钥，本骨架记录但暂未启用通道加密）。
    pub shared_secret: Vec<u8>,
    /// 该连接压缩阈值（0 表示未启用，T7）。worker 据此决定是否先下发 `LoginCompression`。
    pub compression_threshold: i32,
}

/// 在线认证上下文（始终存在，作为 ECS Resource）。
///
/// feature 关闭时 [`OnlineAuthContext::default`] 返回 `enabled = false`、
/// `pending_tx = None`，`network_receive` 据此跳过在线认证走离线语义。
///
/// 私钥以 PKCS#8 DER 字节存储（而非直接持有 `RsaPrivateKey`），使该结构在
/// feature 关闭时无需 `rsa` 类型即可满足 `Send + Sync + 'static` 资源约束。
#[derive(Debug, Clone)]
pub struct OnlineAuthContext {
    /// 是否启用在线认证。
    pub enabled: bool,
    /// RSA-1024 私钥 DER 字节（feature 开启且 `enabled` 时由生成器填充）。
    pub private_key_der: Option<Vec<u8>>,
    /// Mojang hasJoined 端点。
    pub has_joined_url: String,
    /// HTTP 客户端超时。
    pub timeout: Duration,
    /// 待验证会话发送端（feature 开启时由插件装配填入；关闭时为 `None`）。
    pub pending_tx: Option<UnboundedSender<PendingAuth>>,
}

impl Default for OnlineAuthContext {
    fn default() -> Self {
        Self {
            enabled: cfg!(feature = "online-auth"),
            private_key_der: None,
            has_joined_url: "https://sessionserver.mojang.com/session/minecraft/hasJoined"
                .to_string(),
            timeout: Duration::from_secs(5),
            pending_tx: None,
        }
    }
}

impl OnlineAuthContext {
    /// 生成新的 RSA-1024 密钥对并装载进上下文（feature `online-auth` 专用）。
    #[cfg(feature = "online-auth")]
    pub fn generate() -> Self {
        use rand::rngs::OsRng;
        use rsa::RsaPrivateKey;

        let private_key = RsaPrivateKey::new(&mut OsRng, 1024)
            .expect("RSA-1024 密钥对生成失败（系统 CSPRNG 不可用）");
        let der = private_key
            .to_pkcs8_der()
            .map(|d| d.as_bytes().to_vec())
            .unwrap_or_default();
        Self {
            enabled: true,
            private_key_der: Some(der),
            ..Self::default()
        }
    }

    /// 从 DER 字节解析 RSA 私钥（feature `online-auth` 专用）。
    #[cfg(feature = "online-auth")]
    pub fn private_key(&self) -> Option<rsa::RsaPrivateKey> {
        self.private_key_der
            .as_ref()
            .and_then(|d| rsa::RsaPrivateKey::from_pkcs8_der(d).ok())
    }

    /// 是否启用在线认证。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// 在线认证错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    /// 密钥生成失败。
    KeyGenerationFailed(String),
    /// RSA 加密失败。
    EncryptionFailed(String),
    /// RSA 解密失败。
    DecryptionFailed(String),
    /// Challenge token 不匹配。
    InvalidChallengeToken,
    /// Mojang API 验证失败。
    HasJoinedFailed(String),
}

#[cfg(feature = "online-auth")]
mod crypto_impl {
    use super::*;
    use rand_core::{CryptoRng, RngCore};
    use rsa::RsaPublicKey;
    use rsa::pkcs1v15::Pkcs1v15Encrypt;
    use rsa::pkcs8::{DecodePublicKey, EncodePublicKey};

    /// 服务端 RSA 公钥 DER 字节（用于下发 `LoginHelloResponse`）。
    pub fn public_key_der(private_key: &rsa::RsaPrivateKey) -> Vec<u8> {
        private_key
            .to_public_key()
            .to_public_key_der()
            .map(|d| d.as_bytes().to_vec())
            .unwrap_or_default()
    }

    /// 生成 16 字节随机 challenge token（Minecraft 协议约定长度）。
    pub fn generate_verify_token<R: RngCore + CryptoRng>(rng: &mut R) -> [u8; 16] {
        let mut token = [0u8; 16];
        rng.fill_bytes(&mut token);
        token
    }

    /// 用 RSA 公钥加密明文（模拟客户端：加密共享密钥 / challenge token）。
    pub fn encrypt_with_rsa(
        public_key_der: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        let public_key = RsaPublicKey::from_public_key_der(public_key_der)
            .map_err(|e| CryptoError::EncryptionFailed(format!("公钥解析失败: {e}")))?;
        let mut rng = rand::rngs::OsRng;
        public_key
            .encrypt(&mut rng, Pkcs1v15Encrypt, plaintext)
            .map_err(|e| CryptoError::EncryptionFailed(e.to_string()))
    }

    /// 用 RSA 私钥解密（共享密钥 / challenge token）。
    pub fn decrypt_with_rsa(
        private_key: &rsa::RsaPrivateKey,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, CryptoError> {
        // rsa 0.9.10 `decrypt` 签名为 `decrypt(padding, ciphertext)`（2 参数，无 rng）；
        // 与 `encrypt(rng, padding, msg)`（3 参数）不对称，盲化 rng 由内部 `DummyRng` 处理。
        private_key
            .decrypt(Pkcs1v15Encrypt, ciphertext)
            .map_err(|e| CryptoError::DecryptionFailed(e.to_string()))
    }

    /// 验证 challenge token：RSA 解密客户端回传的 token，与本地明文期望值比较。
    pub fn verify_challenge_token(
        private_key: &rsa::RsaPrivateKey,
        encrypted_token: &[u8],
        expected_token: &[u8],
    ) -> Result<(), CryptoError> {
        let decrypted = decrypt_with_rsa(private_key, encrypted_token)?;
        if decrypted != expected_token {
            return Err(CryptoError::InvalidChallengeToken);
        }
        Ok(())
    }

    /// 比对 Mojang hasJoined 响应体中的 `id` 字段与预期 UUID（纯函数，便于单测）。
    pub fn verify_uuid_response(
        body: &serde_json::Value,
        expected: Uuid,
    ) -> Result<Uuid, CryptoError> {
        let verified = body
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CryptoError::HasJoinedFailed("响应缺少 id 字段".to_string()))?;
        let expected_str = expected.simple().to_string();
        if verified.replace('-', "") == expected_str.replace('-', "") {
            Ok(expected)
        } else {
            Err(CryptoError::HasJoinedFailed("UUID 不匹配".to_string()))
        }
    }

    /// 向 Mojang hasJoined API 发起验证请求（异步）。
    ///
    /// 返回 `Ok(uuid)` 表示玩家身份已验证且与预期一致。
    pub async fn verify_has_joined(
        ctx: &OnlineAuthContext,
        username: &str,
        uuid: Uuid,
    ) -> Result<Uuid, CryptoError> {
        let url = format!(
            "{}?username={}&uuid={}",
            ctx.has_joined_url,
            urlencoding::encode(username),
            uuid.simple()
        );

        let client = reqwest::Client::builder()
            .timeout(ctx.timeout)
            .build()
            .map_err(|e| CryptoError::HasJoinedFailed(e.to_string()))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| CryptoError::HasJoinedFailed(e.to_string()))?;

        if !response.status().is_success() {
            return Err(CryptoError::HasJoinedFailed(format!(
                "Mojang 返回状态码 {}",
                response.status()
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| CryptoError::HasJoinedFailed(e.to_string()))?;

        verify_uuid_response(&body, uuid)
    }

    /// 常驻异步 worker：消费待验证会话，调用 `hasJoined`，下发 `LoginSuccess` 或 `LoginDisconnect`。
    ///
    /// 解密共享密钥已在校验分支完成并随 [`PendingAuth`] 带入；此处仅做网络验证与发包。
    pub async fn run_auth_worker(
        mut rx: UnboundedReceiver<PendingAuth>,
        outbound: crate::network::listener::OutboundMap,
        has_joined_url: String,
        timeout: Duration,
        private_key_der: Option<Vec<u8>>,
    ) {
        use crate::network::listener::OutboundMessage;
        use crate::protocol::framing::encode_frame;
        use crate::protocol::packets::{
            LoginCompression, LoginDisconnect, LoginSuccess, encode_clientbound,
        };

        while let Some(item) = rx.recv().await {
            let ctx = OnlineAuthContext {
                enabled: true,
                private_key_der: private_key_der.clone(),
                has_joined_url: has_joined_url.clone(),
                timeout,
                pending_tx: None,
            };

            let verified = verify_has_joined(&ctx, &item.username, item.uuid).await;

            if let Ok(guard) = outbound.lock() {
                if let Some(tx) = guard.get(&item.conn_id) {
                    match verified {
                        Ok(_) => {
                            if item.compression_threshold > 0 {
                                let _ = tx.try_send(OutboundMessage::EnableCompression);
                                let mut frame = Vec::new();
                                if encode_frame(
                                    &mut frame,
                                    &encode_clientbound(&LoginCompression {
                                        threshold: item.compression_threshold,
                                    }),
                                )
                                .is_ok()
                                {
                                    let _ = tx.try_send(OutboundMessage::Frame(frame));
                                }
                            }
                            let mut frame = Vec::new();
                            if encode_frame(
                                &mut frame,
                                &encode_clientbound(&LoginSuccess {
                                    uuid: item.uuid,
                                    username: item.username.clone(),
                                    properties: Vec::new(),
                                }),
                            )
                            .is_ok()
                            {
                                let _ = tx.try_send(OutboundMessage::Frame(frame));
                            }
                        }
                        Err(_) => {
                            let reason = r#"{"text":"Authentication failed"}"#.to_string();
                            let mut frame = Vec::new();
                            if encode_frame(
                                &mut frame,
                                &encode_clientbound(&LoginDisconnect { reason }),
                            )
                            .is_ok()
                            {
                                let _ = tx.try_send(OutboundMessage::Frame(frame));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(feature = "online-auth")]
pub use crypto_impl::{
    decrypt_with_rsa, encrypt_with_rsa, generate_verify_token, public_key_der, run_auth_worker,
    verify_challenge_token, verify_has_joined, verify_uuid_response,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "online-auth"))]
    #[test]
    fn default_context_is_offline() {
        let ctx = OnlineAuthContext::default();
        assert!(!ctx.enabled);
        assert!(ctx.private_key_der.is_none());
        assert!(ctx.pending_tx.is_none());
    }

    #[cfg(feature = "online-auth")]
    #[test]
    fn generate_keypair_enables_context() {
        let ctx = OnlineAuthContext::generate();
        assert!(ctx.enabled);
        assert!(ctx.private_key_der.is_some());
        assert!(ctx.private_key().is_some());
    }

    #[cfg(feature = "online-auth")]
    #[test]
    fn verify_challenge_token_roundtrip() {
        let ctx = OnlineAuthContext::generate();
        let private_key = ctx.private_key().unwrap();
        let public_der = public_key_der(&private_key);

        let token = b"challenge-token-123";
        let encrypted = encrypt_with_rsa(&public_der, token).unwrap();
        // 服务端用私钥解密并与预期比较
        verify_challenge_token(&private_key, &encrypted, token).expect("token 应匹配");
    }

    #[cfg(feature = "online-auth")]
    #[test]
    fn verify_challenge_token_rejects_wrong() {
        let ctx = OnlineAuthContext::generate();
        let private_key = ctx.private_key().unwrap();
        let public_der = public_key_der(&private_key);

        let token = b"correct-token";
        let encrypted = encrypt_with_rsa(&public_der, token).unwrap();
        let wrong = b"wrong-token!!";
        assert!(
            verify_challenge_token(&private_key, &encrypted, wrong).is_err(),
            "应拒绝错误的 token"
        );
    }

    #[cfg(feature = "online-auth")]
    #[test]
    fn shared_secret_rsa_roundtrip() {
        let ctx = OnlineAuthContext::generate();
        let private_key = ctx.private_key().unwrap();
        let public_der = public_key_der(&private_key);

        let secret = [0x11u8; 16];
        let encrypted = encrypt_with_rsa(&public_der, &secret).unwrap();
        let decrypted = decrypt_with_rsa(&private_key, &encrypted).unwrap();
        assert_eq!(decrypted, secret.to_vec());
    }

    #[cfg(feature = "online-auth")]
    #[test]
    fn has_joined_invalid_url_is_rejected() {
        // 指向不存在的本地端点，验证失败路径返回 Err（覆盖"验证失败拒绝"）。
        let ctx = OnlineAuthContext {
            enabled: true,
            private_key_der: None,
            has_joined_url: "http://127.0.0.1:1/does-not-exist".to_string(),
            timeout: Duration::from_millis(200),
            pending_tx: None,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .unwrap();
        let result = rt.block_on(verify_has_joined(
            &ctx,
            "Steve",
            Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
        ));
        assert!(result.is_err(), "无效端点应拒绝验证");
    }

    #[cfg(feature = "online-auth")]
    #[test]
    fn verify_uuid_response_matches() {
        let uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let body = serde_json::json!({ "id": "0123456789abcdef0123456789abcdef", "name": "Steve" });
        assert_eq!(verify_uuid_response(&body, uuid).unwrap(), uuid);

        let wrong = serde_json::json!({ "id": "deadbeefdeadbeefdeadbeefdeadbeef" });
        assert!(verify_uuid_response(&wrong, uuid).is_err());
    }
}
