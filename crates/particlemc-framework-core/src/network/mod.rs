//! 网络层：连接状态机、真实 TCP 监听、桥接与三层混合发包模型。

pub mod audience;
pub mod bridge;
pub mod client;
pub mod connection;
pub mod listener;
pub mod particle;
pub mod sound;

// 旧的 `PacketCodec` 占位 trait 已被 `protocol` 模块（真实编解码）与
// `network::client` 三层发包模型取代，不再作为独立 trait 导出。

pub use audience::{Audience, ConsoleAudience, MultiAudience, PlayerAudience};
pub use bridge::{NetworkBridge, empty_bridge};
pub use client::{
    ChunkSender, ClientNetwork, ClientNetworks, Priority, broadcast, enqueue_packet, flush_all,
};
pub use connection::{Connection, ConnectionError, ConnectionState};
pub use listener::{ConnectionListener, RawFrame};
