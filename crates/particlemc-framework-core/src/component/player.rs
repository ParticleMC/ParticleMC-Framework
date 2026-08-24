//! 玩家标识组件。
//!
//! 记录玩家的 `Uuid` 与用户名，用于在 ECS 中区分玩家实体。`Player` 持有
//! `String`，因此为 `Clone` 而非 `Copy`。
//!
//! 字段均私有化，通过访问器方法读取——与 [`super::position::Position`] 的
//! `x()` / `y()` 等访问器模式保持一致，避免外部直接依赖内部字段布局。

use crate::prelude::Component;
use std::fmt;
use uuid::Uuid;

/// 玩家游戏模式（权威建模用：如创造模式允许 CLONE(3) 克隆）。
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[component(storage = "sparse")]
pub enum GameMode {
    /// 创造模式：允许克隆等作弊操作。
    Creative,
    /// 生存模式（默认）。
    #[default]
    Survival,
    /// 冒险模式。
    Adventure,
    /// 旁观模式。
    Spectator,
}

/// 玩家标识组件。
#[derive(Default, Component, Debug, Clone, PartialEq, Eq)]
#[component(storage = "sparse")]
pub struct Player {
    /// 玩家唯一标识。
    uuid: Uuid,
    /// 玩家用户名。
    username: String,
    /// 玩家游戏模式（默认生存）。
    game_mode: GameMode,
}

impl Player {
    /// 以 `Uuid` 与用户名构造玩家标识（游戏模式默认为生存）。
    pub fn new(uuid: Uuid, username: &str) -> Self {
        Self {
            uuid,
            username: username.to_string(),
            game_mode: GameMode::Survival,
        }
    }

    /// 返回玩家 UUID。
    #[inline]
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// 返回玩家用户名。
    #[inline]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// 返回玩家游戏模式。
    #[inline]
    pub fn game_mode(&self) -> GameMode {
        self.game_mode
    }

    /// 设置玩家游戏模式。
    #[inline]
    pub fn set_game_mode(&mut self, mode: GameMode) {
        self.game_mode = mode;
    }

    /// 是否创造模式（CLONE(3) 创造克隆的前置校验）。
    #[inline]
    pub fn is_creative(&self) -> bool {
        self.game_mode == GameMode::Creative
    }
}

impl fmt::Display for Player {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Player({}, {})", self.uuid, self.username)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_correct_values() {
        let uuid = Uuid::new_v4();
        let player = Player::new(uuid, "Steve");
        assert_eq!(player.uuid(), uuid);
        assert_eq!(player.username(), "Steve");
        assert_eq!(player.game_mode(), GameMode::Survival);
        assert!(!player.is_creative());
    }

    #[test]
    fn set_game_mode_updates_and_is_creative_works() {
        let mut player = Player::new(Uuid::nil(), "Alex");
        assert!(!player.is_creative());

        player.set_game_mode(GameMode::Creative);
        assert_eq!(player.game_mode(), GameMode::Creative);
        assert!(player.is_creative());

        player.set_game_mode(GameMode::Spectator);
        assert_eq!(player.game_mode(), GameMode::Spectator);
        assert!(!player.is_creative());
    }

    #[test]
    fn default_is_survival() {
        let player = Player::default();
        assert_eq!(player.game_mode(), GameMode::Survival);
    }

    #[test]
    fn display_formats_correctly() {
        let uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let player = Player::new(uuid, "Steve");
        assert_eq!(
            format!("{}", player),
            "Player(01234567-89ab-cdef-0123-456789abcdef, Steve)"
        );
    }

    #[test]
    fn clone_preserves_all_fields() {
        let mut player = Player::new(Uuid::new_v4(), "Test");
        player.set_game_mode(GameMode::Creative);
        let cloned = player.clone();
        assert_eq!(cloned.uuid(), player.uuid());
        assert_eq!(cloned.username(), player.username());
        assert_eq!(cloned.game_mode(), player.game_mode());
    }
}
