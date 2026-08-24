// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 系统层：20 TPS tick 管线的 9 个系统。
//!
//! 各系统签名固定（使用 旧 ECS 方案 的 `Commands` / `Query` /
//! `MessageReader` / `MessageWriter` 等真实参数），为后续真实游戏逻辑提供稳定的
//! 契约入口。系统间的执行顺序由 [`crate::plugin::McServerPlugin`] 通过链式
//! `.after()` 依赖在 `Schedule` 调度中保证：network_receive → tick_begin →
//! player_input → player_movement → entity_ai → physics → chunk_dirty_sync →
//! tick_end → network_send。

pub mod attribute_sync;
pub mod block_interaction_validator;
pub mod chunk_dirty_sync;
pub mod chunk_send;
pub mod entity_ai;
pub mod entity_sync;
pub mod inventory_sync;
pub mod network_receive;
pub mod network_send;
pub mod packet_action;
pub mod physics;
pub mod player_input;
pub mod player_movement;
pub mod registry_sync;
pub mod scheduler_tick;
pub mod tick_begin;
pub mod tick_end;

pub use attribute_sync::{AttributeInbox, attribute_sync};
pub use block_interaction_validator::block_interaction_validator;
pub use chunk_dirty_sync::chunk_dirty_sync;
pub use chunk_send::chunk_send;
pub use entity_ai::entity_ai;
pub use entity_sync::entity_sync;
pub use inventory_sync::inventory_sync;
pub use network_receive::command_chat_system;
pub use network_receive::network_receive;
pub use network_send::network_send;
pub use packet_action::packet_action_system;
pub use physics::physics;
pub use player_input::player_input;
pub use player_movement::player_movement;
pub use registry_sync::registry_sync;
pub use scheduler_tick::{TickCounter, scheduler_tick};
pub use tick_begin::tick_begin;
pub use tick_end::tick_end;
