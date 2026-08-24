// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 连接状态机与连接占位结构。
//!
//! [`ConnectionState`] 描述 Minecraft 协议握手后的状态流转（Handshake →
//! Status/Login → Configuration → Play），`transition` 仅在合法转换上返回 `Ok`，
//! 否则返回 [`ConnectionError`]。这是骨架层中具有真实逻辑的少量模块之一。

use std::fmt;

/// Minecraft 协议连接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// 握手阶段（客户端尚未选择后续意图）。
    Handshake,
    /// 状态查询（含 MOTD / 版本探测）。
    Status,
    /// 登录阶段（认证与压缩协商）。
    Login,
    /// 配置阶段（资源包 / 注册表同步）。
    Configuration,
    /// 正式游戏阶段。
    Play,
}

/// 状态机转换错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionError {
    /// 从 `from` 到 `to` 的转换不被协议允许。
    InvalidTransition {
        /// 转换前的状态。
        from: ConnectionState,
        /// 试图进入的状态。
        to: ConnectionState,
    },
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionError::InvalidTransition { from, to } => {
                write!(f, "非法的连接状态转换：{:?} → {:?}", from, to)
            }
        }
    }
}

impl std::error::Error for ConnectionError {}

impl ConnectionState {
    /// 返回状态名（用于日志与错误描述）。
    pub fn state_name(&self) -> &'static str {
        match self {
            ConnectionState::Handshake => "Handshake",
            ConnectionState::Status => "Status",
            ConnectionState::Login => "Login",
            ConnectionState::Configuration => "Configuration",
            ConnectionState::Play => "Play",
        }
    }

    /// 尝试转换到 `next` 状态。
    ///
    /// 合法转换包括：
    /// - Handshake → Status | Login
    /// - Login → Configuration
    /// - Configuration → Play
    /// - Status → Login（重连后进入登录）
    ///
    /// # 错误
    /// 非法转换返回 [`ConnectionError::InvalidTransition`]，不改变原状态。
    pub fn transition(&self, next: ConnectionState) -> Result<ConnectionState, ConnectionError> {
        use ConnectionState::*;
        let allowed = matches!(
            (*self, next),
            (Handshake, Status)
                | (Handshake, Login)
                | (Login, Configuration)
                | (Configuration, Play)
                | (Status, Login)
        );
        if allowed {
            Ok(next)
        } else {
            Err(ConnectionError::InvalidTransition {
                from: *self,
                to: next,
            })
        }
    }
}

/// 单个连接的占位结构（骨架阶段不持有真实 socket）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// 连接标识。
    pub id: u32,
    /// 当前协议状态。
    pub state: ConnectionState,
    /// 对端地址（文本形式）。
    pub address: String,
    /// 压缩是否已启用（登录流程下发 `LoginCompression` 后置位，T7）。
    pub compression_enabled: bool,
}

impl Connection {
    /// 构造一个处于 `state` 的连接占位。
    pub fn new(id: u32, address: &str, state: ConnectionState) -> Self {
        Self {
            id,
            state,
            address: address.to_string(),
            compression_enabled: false,
        }
    }

    /// 尝试推进连接状态，失败时不改变现有状态。
    pub fn transition(&mut self, next: ConnectionState) -> Result<(), ConnectionError> {
        self.state = self.state.transition(next)?;
        Ok(())
    }

    /// 标记该连接已启用压缩（供状态机记录，T7）。
    pub fn enable_compression(&mut self) {
        self.compression_enabled = true;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn legal_transitions_succeed() {
        use ConnectionState::*;
        assert_eq!(Handshake.transition(Status), Ok(Status));
        assert_eq!(Handshake.transition(Login), Ok(Login));
        assert_eq!(Login.transition(Configuration), Ok(Configuration));
        assert_eq!(Configuration.transition(Play), Ok(Play));
        assert_eq!(Status.transition(Login), Ok(Login));
    }

    #[test]
    fn illegal_transitions_fail() {
        use ConnectionState::*;
        assert_eq!(
            Handshake.transition(Play),
            Err(ConnectionError::InvalidTransition {
                from: Handshake,
                to: Play
            })
        );
        assert_eq!(
            Login.transition(Play),
            Err(ConnectionError::InvalidTransition {
                from: Login,
                to: Play
            })
        );
        assert_eq!(
            Play.transition(Handshake),
            Err(ConnectionError::InvalidTransition {
                from: Play,
                to: Handshake
            })
        );
        assert_eq!(
            Configuration.transition(Status),
            Err(ConnectionError::InvalidTransition {
                from: Configuration,
                to: Status
            })
        );
    }

    #[test]
    fn connection_transition_mutates_state_on_success() {
        let mut conn = Connection::new(1, "127.0.0.1", ConnectionState::Handshake);
        conn.transition(ConnectionState::Login).unwrap();
        assert_eq!(conn.state, ConnectionState::Login);
    }

    #[test]
    fn connection_transition_keeps_state_on_failure() {
        let mut conn = Connection::new(1, "127.0.0.1", ConnectionState::Handshake);
        let result = conn.transition(ConnectionState::Play);
        assert!(result.is_err());
        assert_eq!(conn.state, ConnectionState::Handshake);
    }
}
