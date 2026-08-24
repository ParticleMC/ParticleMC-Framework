//! 命令发送者抽象（见 `.specs/implement-command-framework/`）。
//!
//! 仅承载身份与类型信息，文本反馈仍走 `emit` 回调（与 inventory 一致）。

/// 命令发送者：标识命令的来源（玩家 / 控制台 / 服务器）。
///
/// 仅提供身份信息；执行器经 `emit` 回发文本，系统层把玩家发送者接到连接。
pub trait CommandSender: Send + Sync {
    /// 发送者显示名（玩家为用户名，控制台/服务器为固定标识）。
    fn name(&self) -> &str;
    /// 是否为玩家来源。
    fn is_player(&self) -> bool;
    /// 玩家来源返回其实体 id；否则返回 `None`。
    fn entity_id(&self) -> Option<u32>;
}

/// 控制台发送者（命令由服务端控制台触发）。
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsoleSender;

impl CommandSender for ConsoleSender {
    fn name(&self) -> &str {
        "Console"
    }
    fn is_player(&self) -> bool {
        false
    }
    fn entity_id(&self) -> Option<u32> {
        None
    }
}

/// 服务器发送者（命令由服务器内部逻辑触发，如 `/reload`）。
///
/// 无玩家连接，反馈不回发网络（见 `CommandManager::execute_server_command`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct ServerSender;

impl CommandSender for ServerSender {
    fn name(&self) -> &str {
        "Server"
    }
    fn is_player(&self) -> bool {
        false
    }
    fn entity_id(&self) -> Option<u32> {
        None
    }
}

/// 玩家发送者（命令由已登录玩家触发）。
#[derive(Debug, Clone)]
pub struct PlayerSender {
    /// 玩家实体 id（即 [`crate::component::Player`] 的 ECS 实体 id）。
    pub entity_id: u32,
    /// 玩家用户名。
    pub username: String,
}

impl CommandSender for PlayerSender {
    fn name(&self) -> &str {
        &self.username
    }
    fn is_player(&self) -> bool {
        true
    }
    fn entity_id(&self) -> Option<u32> {
        Some(self.entity_id)
    }
}
