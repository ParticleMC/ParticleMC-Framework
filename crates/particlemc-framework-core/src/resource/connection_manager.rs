//! 连接管理器：真实持有在线连接的运行时状态。
//!
//! 每个连接以 `conn_id`（监听任务分配的 32 位整数）为键，记录其对应的玩家实体、
//! 当前协议状态、对端地址与 Velocity 转发身份。连接关闭时移除对应记录。

use std::collections::HashMap;
use std::net::SocketAddr;

use crate::prelude::Entity;

use crate::network::connection::ConnectionState;
use crate::protocol::velocity::ForwardedIdentity;
use particlemc_framework_ecs::scheduler::WorldId;
use uuid::Uuid;

/// 单条连接的运行时状态。
#[derive(Debug, Clone)]
pub struct ConnectionRuntime {
    /// 该连接对应的玩家实体（登录成功后写入）。
    pub entity: Option<Entity>,
    /// 当前协议状态（由游戏侧 `network_receive` 推进）。
    pub state: ConnectionState,
    /// 对端地址（监听任务可附带，缺失时为 `None`）。
    pub addr: Option<SocketAddr>,
    /// 经 Velocity 转发解析出的玩家真实身份（直连为 `None`）。
    pub forwarded: Option<ForwardedIdentity>,
    /// 压缩是否已启用（`LoginCompression` 下发后、收到 `LoginAcknowledged` 时置位，T7）。
    pub compression_enabled: bool,
    /// 玩家实体所在实例 World 的 [`WorldId`]（登录落子时写入，供 `player_input` /
    /// `inventory_sync` 等跨 World 定位其所属实例）。
    pub world_id: WorldId,
    /// 玩家 UUID（登录握手解析，供 `PlayerInfo` 等发包读取，无需回查实例实体）。
    pub uuid: Option<Uuid>,
    /// 玩家名（登录握手解析，供退出的 `PlayerQuit` 等事件携带）。
    pub username: Option<String>,
    /// 在线认证挑战 token（发送 `LoginHelloResponse` 时生成，收到 `LoginChallenge` 时验证）。
    #[cfg(feature = "online-auth")]
    pub pending_challenge_token: Option<Vec<u8>>,
    /// 玩家客户端配置的视距（区块数），用于实体移动广播的距离裁剪。默认 10（Minecraft 默认值）。
    pub view_distance: u8,
}

impl ConnectionRuntime {
    /// 以初始状态构造运行时。
    pub fn new(state: ConnectionState, addr: Option<SocketAddr>) -> Self {
        Self {
            entity: None,
            state,
            addr,
            forwarded: None,
            compression_enabled: false,
            world_id: WorldId(0),
            uuid: None,
            username: None,
            view_distance: 10,
            #[cfg(feature = "online-auth")]
            pending_challenge_token: None,
        }
    }

    /// 标记该连接已启用压缩（T7）。
    pub fn enable_compression(&mut self) {
        self.compression_enabled = true;
    }

    /// 更新玩家客户端配置的视距（区块数）。
    pub fn set_view_distance(&mut self, view_distance: u8) {
        self.view_distance = view_distance;
    }
}

/// 连接管理器：以 `conn_id` 为键维护全部在线连接的运行时。
#[derive(Default)]
pub struct ConnectionManager {
    /// 在线连接运行时表。
    connections: HashMap<u32, ConnectionRuntime>,
}

impl ConnectionManager {
    /// 记录一个新连接上线，返回其可变运行时（不存在则新建）。
    pub fn open(&mut self, conn_id: u32, addr: Option<SocketAddr>) -> &mut ConnectionRuntime {
        self.connections
            .entry(conn_id)
            .or_insert_with(|| ConnectionRuntime::new(ConnectionState::Handshake, addr))
    }

    /// 记录一个连接下线（移除运行时）。
    pub fn close(&mut self, conn_id: &u32) {
        self.connections.remove(conn_id);
    }

    /// 查询某连接的运行时（不可变）。
    pub fn get(&self, conn_id: u32) -> Option<&ConnectionRuntime> {
        self.connections.get(&conn_id)
    }

    /// 查询某连接的运行时（可变）。
    pub fn get_mut(&mut self, conn_id: u32) -> Option<&mut ConnectionRuntime> {
        self.connections.get_mut(&conn_id)
    }

    /// 返回该连接对应的玩家实体（若存在）。
    pub fn entity_of(&self, conn_id: u32) -> Option<Entity> {
        self.connections.get(&conn_id).and_then(|rt| rt.entity)
    }

    /// 当前在线连接数量。
    pub fn active_count(&self) -> usize {
        self.connections.len()
    }

    /// 是否没有任何在线连接。
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// 迭代全部在线连接（conn_id → 运行时）。
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &ConnectionRuntime)> {
        self.connections.iter()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn open_then_query_records_connection() {
        let mut mgr = ConnectionManager::default();
        assert!(mgr.is_empty());
        mgr.open(7, None);
        assert_eq!(mgr.active_count(), 1);
        assert_eq!(mgr.get(7).unwrap().state, ConnectionState::Handshake);
    }

    #[test]
    fn close_removes_connection() {
        let mut mgr = ConnectionManager::default();
        mgr.open(7, None);
        mgr.close(&7);
        assert!(mgr.is_empty());
        assert!(mgr.get(7).is_none());
    }

    #[test]
    fn entity_of_returns_player_entity() {
        let mut mgr = ConnectionManager::default();
        let rt = mgr.open(3, None);
        rt.entity = Some(Entity::from_raw_u32(42));
        assert_eq!(mgr.entity_of(3), Some(Entity::from_raw_u32(42)));
        assert_eq!(mgr.entity_of(99), None);
    }

    #[test]
    fn view_distance_default_is_10() {
        let mut mgr = ConnectionManager::default();
        let rt = mgr.open(1, None);
        assert_eq!(rt.view_distance, 10);
    }

    #[test]
    fn set_view_distance_updates_value() {
        let mut mgr = ConnectionManager::default();
        let rt = mgr.open(1, None);
        rt.set_view_distance(8);
        assert_eq!(rt.view_distance, 8);
        rt.set_view_distance(16);
        assert_eq!(rt.view_distance, 16);
    }
}
