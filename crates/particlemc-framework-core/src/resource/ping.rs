// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 服务器列表 Ping 子系统（对齐 Java `net.minestom.server.ping` 语义，T14）。
//!
//! Java `Status` 经 `StructCodec` 序列化为 MOTD JSON（`{version:{name,protocol},
//! players:{online,max},description,favicon?}`）。本模块 v1 以手写 JSON 序列化
//! 等价结构：`description` 用 [`Component::plain_text`] 字符串承载（Java 为
//! adventure `Component` 的 GSON 对象，v1 简化并注明）。
//!
//! `ServerListPing` 汇总 MOTD 展示所需的全部字段（版本名 / 协议号 / 在线与
//! 最大人数 / 描述 / 可选 favicon），`Status` 包装其携带
//! [`Status::to_json_string`] 序列化。协议号默认 774（Minecraft 1.21.11）。
//!
//! 变更标识符：`complete-missing-subsystems`（R14）。

use crate::text_component::Component;

/// 服务器列表 Ping 数据（MOTD 响应载荷）。
#[derive(Debug, Clone, PartialEq)]
pub struct ServerListPing {
    /// 版本显示名（如 `"1.21.11"`）。
    pub version_name: String,
    /// 网络协议号（1.21.11 = 774）。
    pub protocol: i32,
    /// 在线玩家人数。
    pub players_online: i32,
    /// 最大玩家人数。
    pub players_max: i32,
    /// 描述（MOTD）组件。
    pub description: Component,
    /// 可选 favicon（`data:image/png;base64,...`）。
    pub favicon: Option<String>,
}

impl Default for ServerListPing {
    /// 对齐 Java `Status.DEFAULT_DESCRIPTION`（`"ParticleMC Server"`）与
    /// `VersionInfo.DEFAULT`（1.21.11 / 774）的默认构造。
    fn default() -> Self {
        Self {
            version_name: "1.21.11".to_string(),
            protocol: 774,
            players_online: 0,
            players_max: 20,
            description: Component::text("ParticleMC Server"),
            favicon: None,
        }
    }
}

/// 服务器状态包装（对齐 Java `Status` record 的最小等价）。
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// MOTD Ping 数据。
    pub ping: ServerListPing,
}

impl Status {
    /// 以给定 Ping 数据构造状态。
    pub fn new(ping: ServerListPing) -> Self {
        Self { ping }
    }

    /// 序列化为 MOTD JSON 字符串（手写，无第三方依赖）。
    ///
    /// 结构对齐 Java `Status.CODEC`：
    /// `{"version":{"name":..,"protocol":..},"players":{"online":..,"max":..},
    /// "description":".."[,"favicon":".."]}`。`description` 经
    /// [`Component::plain_text`] 承载（v1 简化，未做 NBT/组件对象展开）。
    pub fn to_json_string(&self) -> String {
        let mut json = format!(
            "{{\"version\":{{\"name\":\"{}\",\"protocol\":{}}},\
             \"players\":{{\"online\":{},\"max\":{}}},\
             \"description\":\"{}\"",
            json_escape(&self.ping.version_name),
            self.ping.protocol,
            self.ping.players_online,
            self.ping.players_max,
            json_escape(&self.ping.description.plain_text()),
        );
        if let Some(favicon) = &self.ping.favicon {
            json.push_str(&format!(",\"favicon\":\"{}\"", json_escape(favicon)));
        }
        json.push('}');
        json
    }
}

/// 对字符串做 JSON 转义（`"` / `\` / 控制字符），返回安全字面量。
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // 其余控制字符以 \u00XX 转义（格式化无 panic 路径）。
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_ping_has_sane_fields() {
        let ping = ServerListPing::default();
        assert_eq!(ping.version_name, "1.21.11");
        assert_eq!(ping.protocol, 774);
        assert_eq!(ping.players_online, 0);
        assert_eq!(ping.players_max, 20);
        assert_eq!(ping.description.plain_text(), "ParticleMC Server");
        assert_eq!(ping.favicon, None);
    }

    #[test]
    fn to_json_contains_expected_keys() {
        let ping = ServerListPing {
            version_name: "1.21.11".to_string(),
            protocol: 774,
            players_online: 3,
            players_max: 20,
            description: Component::text("欢迎来到服务器"),
            favicon: None,
        };
        let json = Status::new(ping).to_json_string();
        // 结构 key 存在。
        for key in [
            "\"version\"",
            "\"name\"",
            "\"protocol\":774",
            "\"players\"",
            "\"online\":3",
            "\"max\":20",
            "\"description\"",
        ] {
            assert!(json.contains(key), "JSON 应含 `{key}`，实际：{json}");
        }
        // 中文描述原样保留。
        assert!(json.contains("欢迎来到服务器"));
        // 无 favicon 时不含该 key。
        assert!(!json.contains("favicon"));
    }

    #[test]
    fn to_json_with_favicon_and_escaped_text() {
        let ping = ServerListPing {
            version_name: "dev".to_string(),
            protocol: 774,
            players_online: 0,
            players_max: 1,
            description: Component::text("引号\"与\\反斜杠"),
            favicon: Some("data:image/png;base64,AAA".to_string()),
        };
        let json = Status::new(ping).to_json_string();
        assert!(json.contains("\"favicon\":\"data:image/png;base64,AAA\""));
        assert!(json.contains("引号\\\"与\\\\反斜杠"));
    }
}
