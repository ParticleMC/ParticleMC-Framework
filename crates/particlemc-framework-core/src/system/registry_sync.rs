// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 注册表同步系统（WS5a-T2/T3）：当注册表发生运行时增删时，向处于 Play
//! 状态的客户端重发 `RegistryData`（0x07）包。
//!
//! # 设计
//!
//! - 各注册表 Resource 通过 [`Registry::insert`] / [`Registry::remove`] 修改后，
//!   会将自身 id 追加到 `dirty: Vec<u32>` 并标记脏。
//! - `McServerPlugin` 启动时收集全部脏标记写入全局 [`RegistrySyncState`] Resource。
//! - 本系统每 tick 消费 `RegistrySyncState`，向处于 Play 状态的全部客户端
//!   下发当前注册表的完整 `RegistryData` 包（协议语义：配置阶段或运行时覆盖
//!   时需全量重发）。
//!
//! # 冻结接口
//!
//! 本系统的输入/输出符合 `complete-framework-gaps` WS5a 冻结契约：
//! - 输入：`RegistrySyncState`（脏标记）、`ConnectionManager`、`ClientNetworks`
//! - 输出：通过 `enqueue_packet` 向 Play 状态客户端发送 `RegistryData`（0x07）

use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::protocol::packets::configuration::{RegistryData, RegistryEntry as ConfigRegistryEntry};
use crate::protocol::packets::encode_clientbound;
use crate::resource::connection_manager::ConnectionManager;
use crate::resource::registries::entity_type::EntityTypeRegistry;
use crate::resource::registries::registry::RegistryId;
use crate::resource::registries::registry::RegistrySyncState;
use crate::resource::registries::{
    BlockRegistry, EntityTypeDefinition, ItemRegistry, Registry as RegistryTrait,
};

/// 消费 [`RegistrySyncState`] 脏标记，向 Play 状态客户端重发对应注册表的
/// `RegistryData`（0x07）包。
///
/// # 行为
/// - 若没有脏标记，跳过不发。
/// - 若注册表处于 `Handshake` / `Status` 状态，不发送（Play 阶段才同步）。
/// - 某注册表不存在时跳过（不 panic）。
pub fn registry_sync(
    mut sync_state: crate::prelude::ResMut<RegistrySyncState>,
    connections: crate::prelude::Res<ConnectionManager>,
    mut clients: crate::prelude::ResMut<ClientNetworks>,
    block_registry: crate::prelude::Res<'_, BlockRegistry>,
    item_registry: crate::prelude::Res<'_, ItemRegistry>,
    entity_type_registry: crate::prelude::Res<'_, EntityTypeRegistry>,
) {
    let dirty = sync_state.take_dirty();
    if dirty.is_empty() {
        return;
    }

    // 向所有 Play 状态连接广播。
    let play_conns: Vec<u32> = connections
        .iter()
        .filter_map(|(id, rt)| {
            if rt.state == crate::network::connection::ConnectionState::Play {
                Some(*id)
            } else {
                None
            }
        })
        .collect();

    if play_conns.is_empty() {
        return;
    }

    for id in dirty {
        let packet = match id {
            RegistryId::Block => Some(build_registry_data_packet(
                "minecraft:block",
                &block_registry.0,
            )),
            RegistryId::Item => Some(build_registry_data_packet(
                "minecraft:item",
                &item_registry.0,
            )),
            RegistryId::EntityType => Some(build_entity_type_packet(&entity_type_registry.0)),
            RegistryId::Generic => None, // TODO: 通用注册表暂不支持运行时覆盖
        };

        if let Some(packet) = packet {
            let bytes = encode_clientbound(&packet);
            for &conn_id in &play_conns {
                enqueue_packet(&mut clients, conn_id, bytes.clone(), Priority::Normal);
            }
        }
    }
}

/// 从具名注册表（`Registry<T>` 承载）构建 `RegistryData` 包。
fn build_registry_data_packet<T: crate::resource::registries::registry::RegistryEntry>(
    registry_id: &str,
    registry: &RegistryTrait<T>,
) -> RegistryData {
    let entries: Vec<ConfigRegistryEntry> = (0..registry.len())
        .filter_map(|id| {
            registry
                .get_name(id as u32)
                .map(|name| ConfigRegistryEntry {
                    key: name.to_string(),
                    value: None,
                })
        })
        .collect();

    RegistryData {
        registry_id: registry_id.to_string(),
        entries,
    }
}

/// 从实体类型注册表构建 `RegistryData` 包。
fn build_entity_type_packet(
    registry: &crate::resource::registries::registry::Registry<EntityTypeDefinition>,
) -> RegistryData {
    let entries: Vec<ConfigRegistryEntry> = (0..registry.len())
        .filter_map(|id| {
            registry
                .get_name(id as u32)
                .map(|name| ConfigRegistryEntry {
                    key: name.to_string(),
                    value: None,
                })
        })
        .collect();

    RegistryData {
        registry_id: "minecraft:entity_type".to_string(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::network::bridge::empty_bridge;
    use crate::network::client::ClientNetwork;
    use crate::network::connection::ConnectionState;
    use crate::network::listener::OutboundMessage;
    use crate::plugin::McServerPlugin;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::framing::decode_frame;
    use std::time::Duration;

    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::with_preload());
        let (bridge, _frame_tx, _outbound) = empty_bridge();
        app.world_mut().insert_resource(bridge);
        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(50),
            ));
        app.update();
        app
    }

    fn spawn_play_client(
        app: &mut App,
        conn_id: u32,
    ) -> tokio::sync::mpsc::Receiver<OutboundMessage> {
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(64);
        app.world_mut()
            .resource_mut::<ClientNetworks>()
            .unwrap()
            .clients
            .insert(conn_id, ClientNetwork::new());
        {
            let bridge = app
                .world()
                .resource::<crate::network::bridge::NetworkBridge>()
                .unwrap();
            bridge.outbound.lock().unwrap().insert(conn_id, out_tx);
        }
        let cm = app.world_mut().resource_mut::<ConnectionManager>().unwrap();
        let rt = cm.open(conn_id, None);
        rt.state = ConnectionState::Play;
        out_rx
    }

    fn capture_packets(rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Frame(frame) = msg {
                let mut pos = 0usize;
                while pos < frame.len() {
                    match decode_frame(&frame, &mut pos) {
                        Ok(payload) => out.push(payload),
                        Err(_) => break,
                    }
                }
            }
        }
        out
    }

    fn payload_id(payload: &[u8]) -> i32 {
        let mut buf = ByteBuffer::new(payload.to_vec());
        buf.get_varint().unwrap_or(-1)
    }

    #[test]
    fn inserting_block_triggers_registry_sync() {
        let mut app = build_app();
        let mut out_tx = spawn_play_client(&mut app, 1);

        // 直接标记脏，不依赖 insert（避免命名冲突）
        let sync_state = app.world_mut().resource_mut::<RegistrySyncState>().unwrap();
        sync_state.mark_dirty(RegistryId::Block);

        app.update();

        // 取出所有出站包
        let payloads = capture_packets(&mut out_tx);
        let ids: Vec<i32> = payloads.iter().map(|p| payload_id(p)).collect();

        assert!(
            ids.contains(&0x07),
            "应下发 RegistryData(0x07)，实际 {ids:?}"
        );
    }

    #[test]
    fn no_dirty_no_sync() {
        let mut app = build_app();
        let mut out_tx = spawn_play_client(&mut app, 1);

        // 无脏标记，不触发同步
        app.update();

        let payloads = capture_packets(&mut out_tx);
        let ids: Vec<i32> = payloads.iter().map(|p| payload_id(p)).collect();
        assert!(
            !ids.contains(&0x07),
            "无脏标记时不应下发 RegistryData(0x07)，实际 {ids:?}"
        );
    }

    #[test]
    fn non_play_connection_ignored() {
        let mut app = build_app();
        let mut out_tx_1 = spawn_play_client(&mut app, 1);
        let mut out_tx_2 = spawn_play_client(&mut app, 2);

        // 将 conn 2 降级为 Handshake
        let cm = app.world_mut().resource_mut::<ConnectionManager>().unwrap();
        cm.get_mut(2).unwrap().state = ConnectionState::Handshake;

        // 标记脏
        let sync_state = app.world_mut().resource_mut::<RegistrySyncState>().unwrap();
        sync_state.mark_dirty(RegistryId::Block);

        app.update();

        // conn 1（Play）应收包，conn 2（Handshake）不应收
        let payloads_1 = capture_packets(&mut out_tx_1);
        let ids_1: Vec<i32> = payloads_1.iter().map(|p| payload_id(p)).collect();
        assert!(ids_1.contains(&0x07), "Play 连接应收到 0x07");

        let payloads_2 = capture_packets(&mut out_tx_2);
        let ids_2: Vec<i32> = payloads_2.iter().map(|p| payload_id(p)).collect();
        assert!(!ids_2.contains(&0x07), "Handshake 连接不应收到 0x07");
    }
}
