// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 服务器状态配置（MOTD 等状态响应内容）。
//!
//! 状态响应（StatusResponse）的 JSON 内容属于服务器应用层决策，框架只提供
//! 协议机制。本资源让应用可覆盖默认文案，避免硬编码进框架。

/// 状态响应配置。
///
/// - `protocol`：状态响应中声明的协议版本号（1.21.11 = 774）。
/// - `max_players`：状态响应中的最大玩家数。
/// - `motd`：状态响应中的描述文本。
#[derive(Debug, Clone)]
pub struct StatusConfig {
    /// 状态响应中声明的协议版本号。
    pub protocol: i32,
    /// 状态响应中的最大玩家数。
    pub max_players: i32,
    /// 状态响应中的描述文本（MOTD）。
    pub motd: String,
    /// 状态响应中的版本名称。
    pub version_name: String,
}

impl Default for StatusConfig {
    /// 默认值：1.21.11 / protocol 774 / 20 人 / "ParticleMC (Rust)"。
    fn default() -> Self {
        Self {
            protocol: 774,
            max_players: 20,
            motd: "ParticleMC (Rust)".to_string(),
            version_name: "1.21.11".to_string(),
        }
    }
}

impl StatusConfig {
    /// 序列化状态响应 JSON（客户端 Status 阶段解析）。
    ///
    /// 结构：`version.name` + `version.protocol` + `players.max` + `players.online` +
    /// `description.text`。
    pub fn to_status_json(&self, online: usize) -> String {
        format!(
            r#"{{"version":{{"name":"{vn}","protocol":{proto}}},"players":{{"max":{max},"online":{on}}},"description":{{"text":"{motd}"}}}}"#,
            vn = self.version_name,
            proto = self.protocol,
            max = self.max_players,
            on = online,
            motd = self.motd
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn to_status_json_matches_expected_shape() {
        let config = StatusConfig::default();
        let json = config.to_status_json(0);
        assert!(json.contains(r#""protocol":774"#));
        assert!(json.contains(r#""name":"1.21.11""#));
        assert!(json.contains(r#""max":20"#));
        assert!(json.contains(r#""online":0"#));
        assert!(json.contains(r#""text":"ParticleMC (Rust)""#));
    }

    #[test]
    fn custom_config_is_serialized() {
        let config = StatusConfig {
            protocol: 774,
            max_players: 100,
            motd: "Hello".to_string(),
            version_name: "1.21.11".to_string(),
        };
        let json = config.to_status_json(3);
        assert!(json.contains(r#""max":100"#));
        assert!(json.contains(r#""online":3"#));
        assert!(json.contains(r#""text":"Hello""#));
    }
}
