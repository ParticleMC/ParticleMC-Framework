// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 剩余常用 Player 事件。

use crate::event::r#trait::{CancellableEvent, EntityEvent, Event, PlayerEvent};
use crate::prelude::{Entity, Message};

/// 玩家 Anvil 输入事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerAnvilInput {
    pub player: Entity,
    pub input: String,
    pub cost: u32,
    pub cancelled: bool,
}

impl Event for PlayerAnvilInput {}
impl EntityEvent for PlayerAnvilInput {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerAnvilInput {}
impl CancellableEvent for PlayerAnvilInput {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

/// 玩家选择方块事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerPickBlock {
    pub player: Entity,
    pub cancelled: bool,
}

impl Event for PlayerPickBlock {}
impl EntityEvent for PlayerPickBlock {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerPickBlock {}
impl CancellableEvent for PlayerPickBlock {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

/// 玩家插件消息事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerPluginMessage {
    pub player: Entity,
    pub channel: String,
    pub data: Vec<u8>,
    pub cancelled: bool,
}

impl Event for PlayerPluginMessage {}
impl EntityEvent for PlayerPluginMessage {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerPluginMessage {}
impl CancellableEvent for PlayerPluginMessage {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

/// 玩家自定义配置点击事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerConfigCustomClick {
    pub player: Entity,
    pub button: u8,
    pub slot: u8,
    pub cancelled: bool,
}

impl Event for PlayerConfigCustomClick {}
impl EntityEvent for PlayerConfigCustomClick {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerConfigCustomClick {}
impl CancellableEvent for PlayerConfigCustomClick {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

/// 玩家自定义点击事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerCustomClick {
    pub player: Entity,
    pub button: u8,
    pub cancelled: bool,
}

impl Event for PlayerCustomClick {}
impl EntityEvent for PlayerCustomClick {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerCustomClick {}
impl CancellableEvent for PlayerCustomClick {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}

/// 玩家调试订阅请求事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerDebugSubscriptionsRequest {
    pub player: Entity,
    pub cancelled: bool,
}

impl Event for PlayerDebugSubscriptionsRequest {}
impl EntityEvent for PlayerDebugSubscriptionsRequest {
    fn entity(&self) -> Entity {
        self.player
    }
}
impl PlayerEvent for PlayerDebugSubscriptionsRequest {}
impl CancellableEvent for PlayerDebugSubscriptionsRequest {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
    fn set_cancelled(&mut self, cancelled: bool) {
        self.cancelled = cancelled;
    }
}
