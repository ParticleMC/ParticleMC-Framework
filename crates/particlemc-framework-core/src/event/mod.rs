//! 事件层（`Message`）与事件总线。
//!
//! 以 自研 ECS 的 `Message` 机制（即旧版 `Event`）描述游戏循环与网络层之间的
//! 桥接事件。所有事件均派生 `Message` + `Debug` + `Clone`，字段全部公开可读。
//!
//! 另提供 [`bus`] 模块的事件总线（[`EventBus`] / [`Listener`] / [`EventContext`]），
//! 用于注册监听器并按注册顺序派发事件。
//!
//! 注：自研 ECS 已将 `Event` 重命名为 `Message`，派生宏相应为 `#[derive(Message)]`，
//! 注册使用 `App::add_message`，发送使用 `World::write_message`。

pub mod bus;
pub mod dispatcher;
pub mod entity;
pub mod filter;
pub mod instance;
pub mod inventory;
pub mod item;
pub mod player;
pub mod r#trait;

use crate::prelude::{Entity, Message};

use crate::component::{Block, BlockState, DamageSource, Position};
use crate::network::connection::ConnectionState;
use crate::resource::{DamageType, EntityType};

pub use bus::{EventBus, EventContext, Listener, ListenerId};
pub use dispatcher::{EventDispatcher, ListenerId as DispatcherListenerId};
pub use r#trait::{CancellableEvent, EntityEvent, Event, InstanceEvent, PlayerEvent};

/// 方块被破坏事件。
#[derive(Message, Debug, Clone)]
pub struct BlockBreak {
    /// 被破坏方块的位置。
    pub position: Position,
    /// 被破坏方块的旧状态。
    pub block: BlockState,
}

/// 方块被放置事件。
#[derive(Message, Debug, Clone)]
pub struct BlockPlace {
    /// 被放置方块的位置。
    pub position: Position,
    /// 被放置方块的新状态。
    pub block: BlockState,
}

/// 玩家加入事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerJoin {
    /// 玩家实体。
    pub entity: Entity,
    /// 玩家用户名。
    pub username: String,
}

/// 玩家离开事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerQuit {
    /// 玩家实体。
    pub entity: Entity,
    /// 玩家用户名。
    pub username: String,
}

/// 玩家移动事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerMove {
    /// 玩家实体。
    pub entity: Entity,
    /// 移动前位置。
    pub from: Position,
    /// 移动后位置。
    pub to: Position,
}

/// 实体受伤事件。
#[derive(Message, Debug, Clone)]
pub struct EntityDamage {
    /// 受伤实体。
    pub entity: Entity,
    /// 伤害量。
    pub amount: f32,
    /// 伤害来源（T7 追加：实体 / 位置 / 类型 / 未知）。
    pub source: DamageSource,
    /// 伤害类型（T7 追加：注册表条目，`None` 表示未指定）。
    pub damage_type: Option<DamageType>,
}

/// 实体死亡事件。
#[derive(Message, Debug, Clone)]
pub struct EntityDeath {
    /// 死亡实体。
    pub entity: Entity,
}

/// 网络层 → 游戏循环的桥接事件。
///
/// 监听任务把解析好的入站帧推入 `NetworkBridge.inbound`；`network_receive` 系统
/// 消费这些帧，或直接派发为 [`NetworkEvent::Packet`]，或为玩家移动派生出
/// [`NetworkEvent::PlayerMoveRaw`]，连接断开时产生 [`NetworkEvent::Closed`]。
#[derive(Message, Debug, Clone)]
pub enum NetworkEvent {
    /// 一帧完整入站数据包（已解析出 packet_id 与包体，状态由监听任务标注）。
    Packet {
        /// 连接标识。
        conn_id: u32,
        /// 收到该包时连接所处的协议状态。
        state: ConnectionState,
        /// 包 id。
        packet_id: i32,
        /// 包体（不含 packet_id）。
        payload: Vec<u8>,
    },
    /// 玩家移动原始上报（由 `network_receive` 从 `PlayerPosition` 等派生）。
    PlayerMoveRaw {
        /// 连接标识。
        conn_id: u32,
        /// 上报坐标。
        position: Position,
        /// 偏航角（弧度）。
        yaw: f32,
        /// 俯仰角（弧度）。
        pitch: f32,
        /// 是否踩在地面上。
        grounded: bool,
    },
    /// 连接关闭。
    Closed(u32),
}

/// 玩家进入 Play 状态事件。
///
/// 由 `network_receive` 在连接完成 Configuration → Play 转换（收到
/// `FinishConfiguration` C2S）后写入；`chunk_send` 与 `entity_sync` 系统消费，
/// 分别触发出生区块批次发送与玩家实体广播。
#[derive(Message, Debug, Clone)]
pub struct EnterPlayEvent {
    /// 连接标识。
    pub conn_id: u32,
    /// 进入游玩的玩家实体。
    pub entity: Entity,
}

// ---------- 框架动作事件（implement-framework-capabilities T28）----------
// 由 `packet_action_system` 消费 `ClientNetworks.packet_inbox` 中的动作类
// serverbound 包后派发，供应用侧监听（经 `EventBus` 注册 `Listener`）。

/// 玩家与实体交互（serverbound `InteractEntity`，id 0x19）。
#[derive(Message, Debug, Clone)]
pub struct EntityInteract {
    /// 交互的玩家实体。
    pub player: Entity,
    /// 被交互实体的协议 id（客户端侧 id，非 ECS `Entity`）。
    pub target: i32,
    /// 是否潜行。
    pub sneaking: bool,
}

/// 玩家动作（serverbound `PlayerAction`，id 0x28）：挖掘 / 丢弃 / 使用等。
#[derive(Message, Debug, Clone)]
pub struct PlayerActionEvent {
    /// 动作玩家实体。
    pub player: Entity,
    /// 动作状态码（`play::PlayerAction` 的 `status`）。
    pub status: i32,
    /// 动作目标方块位置。
    pub position: (i32, i32, i32),
}

/// 方块交互校验失败事件（由 [`super::system::block_interaction_validator`] 写入）。
///
/// 当客户端上报的方块位置无法通过服务端射线校验或距离校验时，
/// 该事件被写入，供应用侧记录日志或触发反作弊响应。
#[derive(Message, Debug, Clone)]
pub struct BlockInteractionRejected {
    /// 发生违规的玩家实体。
    pub player: Entity,
    /// 客户端上报的方块位置。
    pub position: (i32, i32, i32),
    /// 动作状态码。
    pub status: i32,
}

/// 玩家使用物品（serverbound `UseItem`，id 0x40）。
#[derive(Message, Debug, Clone)]
pub struct PlayerUseItem {
    /// 玩家实体。
    pub player: Entity,
    /// 使用的手（0=主手，1=副手）。
    pub hand: i32,
}

/// 玩家动画（serverbound `Animation`，id 0x3c）：挥臂等。
#[derive(Message, Debug, Clone)]
pub struct PlayerAnimation {
    /// 玩家实体。
    pub player: Entity,
    /// 动画所用手。
    pub hand: i32,
}

/// 玩家与方块交互事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerBlockInteract {
    /// 交互玩家实体。
    pub player: Entity,
    /// 交互方块的位置。
    pub position: Position,
    /// 被交互的方块。
    pub block: Block,
}

/// 实体生成事件。
///
/// 由 [`crate::resource::EntitySpawner::spawn_entity`] 等入口在生成实体后写入，
/// 供 `entity_sync` 等系统消费并向其他玩家广播生成同步。
#[derive(Message, Debug, Clone)]
pub struct EntitySpawn {
    /// 生成的实体。
    pub entity: Entity,
    /// 实体类型。
    pub entity_type: EntityType,
    /// 生成位置。
    pub position: Position,
}

/// 实体移除事件。
///
/// 由 [`crate::resource::EntitySpawner::despawn_entity`] 等入口在销毁实体后写入，
/// 供 `entity_sync` 等系统消费并向其他玩家广播销毁同步。
#[derive(Message, Debug, Clone)]
pub struct EntityRemove {
    /// 被移除的实体。
    pub entity: Entity,
}

/// 玩家聊天事件。
#[derive(Message, Debug, Clone)]
pub struct PlayerChat {
    /// 发言玩家实体。
    pub player: Entity,
    /// 聊天文本。
    pub message: String,
}

/// 方块更新事件。
#[derive(Message, Debug, Clone)]
pub struct BlockUpdate {
    /// 更新位置。
    pub position: Position,
    /// 更新后的方块。
    pub block: Block,
    /// 更新前的方块。
    pub old_block: Block,
}

/// 实体移动事件。
#[derive(Message, Debug, Clone)]
pub struct EntityMove {
    /// 移动的实体。
    pub entity: Entity,
    /// 移动前位置。
    pub from: Position,
    /// 移动后位置。
    pub to: Position,
}

/// 游戏循环 → 网络层的出站事件。
///
/// 已弃用：实际发包统一经 `ClientNetwork` 队列模型在 `network_send` 中
/// `flush_all` 完成，本事件通道不再承载出站数据，仅保留类型以兼容旧引用。
#[deprecated(note = "已被 ClientNetwork 队列模型取代，实际发包统一走 network_send/flush_all")]
#[derive(Debug, Clone)]
pub struct PacketSendEvent {
    /// 目标连接标识。
    pub conn_id: u32,
    /// 包 id。
    pub packet_id: i32,
    /// 包体（不含 packet_id）。
    pub payload: Vec<u8>,
}

// 手动实现 `Message` 而非 `#[derive(Message)]`：derive 宏展开出的 impl 会引用
// 被弃用的类型名，触发弃用告警且无法用 `#[allow(deprecated)]` 覆盖，故改为显式实现。
#[allow(deprecated)]
impl Message for PacketSendEvent {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::prelude::{MessageReader, ResMut};

    /// 收集测试期间派发的 `EntityDamage` 消息。
    #[derive(Default)]
    struct CapturedDamage(Vec<EntityDamage>);

    fn collect_damage(reader: MessageReader<EntityDamage>, mut out: ResMut<CapturedDamage>) {
        out.0.extend(reader.read().cloned());
    }

    #[test]
    fn entity_damage_message_is_delivered_after_update() {
        let mut app = App::new();
        app.add_message::<EntityDamage>()
            .init_resource::<CapturedDamage>()
            .add_systems(collect_damage);

        let target = app.world_mut().spawn_empty().id();
        app.world_mut().write(EntityDamage {
            entity: target,
            amount: 5.0,
            source: DamageSource::Entity(7),
            damage_type: None,
        });

        // 自研 ECS 消息时序：write_message 后跨两帧——第一次 update 置
        // Ready，第二次 update 的 First 才把消息推给 MessageReader。
        app.update();
        app.update();

        let got = app.world().resource::<CapturedDamage>().unwrap().0.clone();
        assert_eq!(got.len(), 1);
        let event = got.first().unwrap();
        assert_eq!(event.entity, target);
        assert_eq!(event.amount, 5.0);
        assert_eq!(event.source, DamageSource::Entity(7));
        assert!(event.damage_type.is_none());
    }
}
