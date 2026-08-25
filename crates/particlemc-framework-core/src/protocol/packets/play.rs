// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 游玩阶段数据包（protocol 774 / 1.21.11 真实映射）。
//!
//! 库存相关 clientbound 包（`WindowItemsPacket` / `SetSlotPacket` / `EntityEquipmentPacket`）
//! 见 `.specs/implement-item-inventory/`（物品与物品栏任务规格）。
//!
//! 分两组：
//! - serverbound：0x00..=0x41 全覆盖（`TeleportConfirm`、`QueryBlockNbt`、`Status`、
//!   `KeepAlive`、`PlayerPosition`、`PlayerPositionAndRotation`、`Look`、
//!   `PlayerPositionStatus`、`PlayerLoaded`、`ClickContainer`、`CloseContainer`、
//!   `ClientCommandChat`(0x06)、`ClientSignedCommandChat`(0x07)、`ClientHeldItemChange`
//!   等全部 66 个包）；0x0b 权威映射为 `ClientStatusPacket`；
//! - clientbound：`SpawnEntity`、`Position`、`UpdateHealth`、`PlayerInfo`、
//!   `GameStateChange`、`ChunkBatchStart`、`ChunkBatchFinished`、`MapChunk`、`Login`、
//!   `SystemChatPacket`（0x77，命令反馈）。
//!
//! 命令聊天入口包（0x06 / 0x07）与系统聊天反馈包（0x77）见
//! `.specs/implement-command-framework/`（命令框架任务规格）。
//!
//! 库存点击 serverbound 包（click_container）见 `.specs/implement-item-click/`（物品点击任务规格）。
//!
//! 变更标识：`complete-partial-framework-capabilities`（T6：复杂包真实化——`DeclareCommands`
//! 命令树节点、`DeclareRecipes` 配方属性、`PlayerChatMessage` 完整字段与过滤掩码、
//! `DeathCombatEvent`/`ShowDialog` 组件承载、`SetCooldown`/`RecipeBookAdd`/
//! `RecipeBookSettings` 真实线格式）。

use uuid::Uuid;

use crate::component::EntityMetadataValue;
use crate::item_stack::{ItemStack, decode_item_stack, encode_item_stack};
use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::nbt::{self, NbtTag};
use crate::protocol::packet::Packet;
use crate::protocol::packets::Property;
use crate::resource::command::argument::ArgumentParserType;
use crate::resource::recipe::{RecipeDisplay, RecipeProperty, StonecutterRecipe};
use crate::text_component::Component;

// ============================ serverbound ============================

/// 确认传送（serverbound, id 0x00，wire 名 `teleport_confirm`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeleportConfirm {
    pub teleport_id: i32,
}

impl Packet for TeleportConfirm {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(TeleportConfirm {
            teleport_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.teleport_id);
        Ok(())
    }
}

/// 区块批接收确认（serverbound, id 0x0a，wire 名 `chunk_batch_received`）。
///
/// 客户端上报「每 tick 期望接收的区块数」，用于服务端节奏控制。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkBatchReceived {
    pub chunks_per_tick: f32,
}

impl Packet for ChunkBatchReceived {
    fn packet_id(&self) -> i32 {
        0x0a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ChunkBatchReceived {
            chunks_per_tick: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.chunks_per_tick);
        Ok(())
    }
}

/// 客户端状态（serverbound, id 0x0b，wire 名 `client_status`）。
///
/// 线格式仅 `action`(VarInt)：0 = `PERFORM_RESPAWN`（请求重生），
/// 1 = `REQUEST_STATS`（请求统计）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// 动作（VarInt）。
    pub action: i32,
}

impl Packet for Status {
    fn packet_id(&self) -> i32 {
        0x0b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Status {
            action: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.action);
        Ok(())
    }
}

/// 命令聊天（serverbound, id 0x06，wire 名 `chat_command`）。
///
/// 客户端输入的命令文本（可能以 `/` 前缀开头，也可能无前缀）。完整 1.21.11 线格式还包含
/// 时间戳、盐、参数签名、消息计数与 acknowledged 位集，但本框架命令路由只需 `message`，
/// 故仅解码首字段 `message`，其余尾部字段直接忽略。见 `.specs/implement-command-framework/`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCommandChatPacket {
    /// 命令文本（含或不带 `/` 前缀，由 `CommandManager` 按无 `/` 处理）。
    pub message: String,
}

impl Packet for ClientCommandChatPacket {
    fn packet_id(&self) -> i32 {
        0x06
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let message = buf.get_string()?;
        // 后续字段（时间戳 / 盐 / 签名 / acknowledged）本框架命令路由不需要，忽略。
        Ok(Self { message })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.message);
        Ok(())
    }
}

/// 签名命令聊天（serverbound, id 0x07，wire 名 `chat_command_signed`）。
///
/// 与 [`ClientCommandChatPacket`] 同构，区别在客户端使用签名聊天模式。本框架同样仅取
/// `message`，忽略尾部签名与时间字段。见 `.specs/implement-command-framework/`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSignedCommandChatPacket {
    /// 命令文本（含或不带 `/` 前缀）。
    pub message: String,
}

impl Packet for ClientSignedCommandChatPacket {
    fn packet_id(&self) -> i32 {
        0x07
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let message = buf.get_string()?;
        // 后续字段（时间戳 / 盐 / 签名）本框架命令路由不需要，忽略。
        Ok(Self { message })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.message);
        Ok(())
    }
}

/// 心跳回复（serverbound, id 0x1b，wire 名 `keep_alive`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAlive {
    pub keep_alive_id: i64,
}

impl Packet for KeepAlive {
    fn packet_id(&self) -> i32 {
        0x1b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(KeepAlive {
            keep_alive_id: buf.get_i64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.keep_alive_id);
        Ok(())
    }
}

/// 玩家移动（serverbound, id 0x1d，wire 名 `position`）。
///
/// `grounded` 对应线格式 flags 字节的 bit0（on ground），横向碰撞位（bit1）本框架
/// 不关注，解码时仅取 on-ground 语义。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub grounded: bool,
}

impl Packet for PlayerPosition {
    fn packet_id(&self) -> i32 {
        0x1d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerPosition {
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            grounded: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_bool(self.grounded);
        Ok(())
    }
}

/// 玩家移动 + 旋转（serverbound, id 0x1e，wire 名 `position_look`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPositionAndRotation {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub grounded: bool,
}

impl Packet for PlayerPositionAndRotation {
    fn packet_id(&self) -> i32 {
        0x1e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerPositionAndRotation {
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
            grounded: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        buf.put_bool(self.grounded);
        Ok(())
    }
}

/// 玩家旋转（serverbound, id 0x1f，wire 名 `look`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl Packet for Look {
    fn packet_id(&self) -> i32 {
        0x1f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Look {
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 玩家位置状态（serverbound, id 0x20，wire 名 `player_position`）。
///
/// 线格式为单字节 `flags`：bit0 = on ground（`FLAG_ON_GROUND`），bit1 = 横向碰撞
/// （`FLAG_HORIZONTAL_COLLISION`）。对应 1.21.11 的 `ClientPlayerPositionStatusPacket`，
/// 即旧版 `flying` 包演进后的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerPositionStatus {
    /// 状态标志位（BitField）。
    pub flags: u8,
}

impl PlayerPositionStatus {
    /// on ground 标志位（bit0）。
    pub const FLAG_ON_GROUND: u8 = 0x01;
    /// 横向碰撞标志位（bit1）。
    pub const FLAG_HORIZONTAL_COLLISION: u8 = 0x02;

    /// 由两个布尔位构造 `flags`。
    pub fn new(on_ground: bool, horizontal_collision: bool) -> Self {
        let mut flags = 0u8;
        if on_ground {
            flags |= Self::FLAG_ON_GROUND;
        }
        if horizontal_collision {
            flags |= Self::FLAG_HORIZONTAL_COLLISION;
        }
        Self { flags }
    }

    /// 是否在地面（bit0）。
    pub fn on_ground(&self) -> bool {
        self.flags & Self::FLAG_ON_GROUND != 0
    }

    /// 是否有横向碰撞（bit1）。
    pub fn horizontal_collision(&self) -> bool {
        self.flags & Self::FLAG_HORIZONTAL_COLLISION != 0
    }
}

impl Packet for PlayerPositionStatus {
    fn packet_id(&self) -> i32 {
        0x20
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerPositionStatus {
            flags: buf.get_u8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_u8(self.flags);
        Ok(())
    }
}

/// 客户端已加载完成（serverbound, id 0x2b，wire 名 `player_loaded`），空包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerLoaded;

impl Packet for PlayerLoaded {
    fn packet_id(&self) -> i32 {
        0x2b
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerLoaded)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 库存点击（serverbound, id 0x11，wire 名 `click_container`）。
///
/// 框架层采用权威重算：服务端仅依据 (slot, button, mode) 与自身 cursor 计算库存，
/// `changed_slots` / `carried_item` 由客户端预测、服务端不予采信，仅解码消费。
/// 见 `.specs/implement-item-click/`。
#[derive(Debug, Clone, PartialEq)]
pub struct ClickContainer {
    /// 窗口 id（VarInt）。玩家库存窗口恒为 0；其余容器窗口从 1 起递增。
    pub window_id: i32,
    /// 状态 id（VarInt）。服务端回推沿用 0；乐观锁用途，本包仅透传。
    pub state_id: i32,
    /// 窗口序槽号（Short）。THROW 模式（mode=4）下 -999 表示操作来自光标。
    pub slot: i16,
    /// 按钮（Byte）。语义随 `mode` 变化（如左/右键、热键槽号等）。
    pub button: i8,
    /// 点击模式（VarInt）。见 [`ClickMode`]；非法值由上层校验。
    pub mode: i32,
    /// 变更槽列表（窗口序槽, 物品）。框架不采信，仅解码消费。
    pub changed_slots: Vec<(i16, ItemStack)>,
    /// 光标携带物品。框架不采信，仅解码消费。
    pub carried_item: ItemStack,
}

/// 点击模式（mode 字段的语义枚举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickMode {
    /// 普通拾取 / 放置（左键 0，右键 1）。
    Pickup = 0,
    /// 快速移动到其他库存（Shift 点击）。
    QuickMove = 1,
    /// 与指定槽位交换（数字键 / 拖拽）。
    Swap = 2,
    /// 克隆（创造模式，仅同类物品）。
    Clone = 3,
    /// 丢弃（按键 Q / 拖拽丢弃）。slot=-999 表示来自光标。
    Throw = 4,
    /// 快速合成（拖拽摆盘）。
    QuickCraft = 5,
    /// 全部拾取（双击）。
    PickupAll = 6,
}

impl ClickMode {
    /// VarInt → 枚举；未定义值返回 `None`（不 panic）。
    pub fn from_i32(v: i32) -> Option<ClickMode> {
        match v {
            0 => Some(ClickMode::Pickup),
            1 => Some(ClickMode::QuickMove),
            2 => Some(ClickMode::Swap),
            3 => Some(ClickMode::Clone),
            4 => Some(ClickMode::Throw),
            5 => Some(ClickMode::QuickCraft),
            6 => Some(ClickMode::PickupAll),
            _ => None,
        }
    }
    /// 枚举 → VarInt。
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

impl Packet for ClickContainer {
    fn packet_id(&self) -> i32 {
        0x11
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let window_id = buf.get_varint()?;
        let state_id = buf.get_varint()?;
        let slot = buf.get_i16()?;
        let button = buf.get_i8()?;
        let mode = buf.get_varint()?;
        // changed_slots 计数（VarInt，i32）。负数为非法协议值。
        let len = buf.get_varint()?;
        if len < 0 {
            return Err(ProtocolError::InvalidValue);
        }
        let n = usize::try_from(len).map_err(|_| ProtocolError::InvalidValue)?;
        let mut changed_slots = Vec::with_capacity(n);
        for _ in 0..n {
            let s = buf.get_i16()?;
            let item = decode_item_stack(buf)?;
            changed_slots.push((s, item));
        }
        let carried_item = decode_item_stack(buf)?;
        Ok(ClickContainer {
            window_id,
            state_id,
            slot,
            button,
            mode,
            changed_slots,
            carried_item,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        buf.put_varint(self.state_id);
        buf.put_i16(self.slot);
        buf.put_i8(self.button);
        buf.put_varint(self.mode);
        // changed_slots 计数 usize → i32 属潜在缩窄，用 TryFrom 溢出即报错。
        let len =
            i32::try_from(self.changed_slots.len()).map_err(|_| ProtocolError::InvalidValue)?;
        buf.put_varint(len);
        for (s, item) in &self.changed_slots {
            buf.put_i16(*s);
            encode_item_stack(item, buf)?;
        }
        encode_item_stack(&self.carried_item, buf)
    }
}

/// 关闭容器（serverbound, id 0x12，wire 名 `close_container`）。
///
/// 线格式仅 `window_id`(VarInt)。收到即清空玩家光标（见 `.specs/implement-item-inventory/`）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloseContainer {
    /// 窗口 id（VarInt）。玩家库存窗口恒为 0；其余容器窗口从 1 起。
    pub window_id: i32,
}

impl Packet for CloseContainer {
    fn packet_id(&self) -> i32 {
        0x12
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let window_id = buf.get_varint()?;
        Ok(Self { window_id })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        Ok(())
    }
}

/// 手持物品切换（serverbound, id 0x34，wire 名 `set_held_item`）。
///
/// 线格式仅 `slot`(SHORT，0-8 热键栏序号，可为负表示非法)。
/// 见 `.specs/implement-item-inventory/`。
#[derive(Debug, Clone, PartialEq)]
pub struct ClientHeldItemChange {
    /// 目标手持槽（SHORT）。服务端须范围校验（0-8）。
    pub slot: i16,
}

impl Packet for ClientHeldItemChange {
    fn packet_id(&self) -> i32 {
        0x34
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let slot = buf.get_i16()?;
        Ok(Self { slot })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i16(self.slot);
        Ok(())
    }
}

// ---- serverbound 新增包（0x01..=0x41，按序位排列）----

/// 查询方块 NBT（serverbound, id 0x01，wire 名 `query_block_nbt`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryBlockNbt {
    /// 事务 id（VarInt），供查询响应回指。
    pub transaction_id: i32,
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
}

impl Packet for QueryBlockNbt {
    fn packet_id(&self) -> i32 {
        0x01
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(QueryBlockNbt {
            transaction_id: buf.get_varint()?,
            block_position: buf.get_position()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.transaction_id);
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        Ok(())
    }
}

/// 选择 Bundle 内物品（serverbound, id 0x02，wire 名 `select_bundle_item`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectBundleItem {
    /// 槽位（VarInt）。
    pub slot: i32,
    /// 选中子物品下标（VarInt）。
    pub selected_index: i32,
}

impl Packet for SelectBundleItem {
    fn packet_id(&self) -> i32 {
        0x02
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SelectBundleItem {
            slot: buf.get_varint()?,
            selected_index: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.slot);
        buf.put_varint(self.selected_index);
        Ok(())
    }
}

/// 修改难度（serverbound, id 0x03，wire 名 `change_difficulty`）。
///
/// `difficulty` 枚举：0=和平，1=简单，2=普通，3=困难（VarInt）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeDifficulty {
    /// 难度 id（VarInt）。
    pub difficulty: i32,
    /// 是否锁定难度（Bool）。
    pub locked: bool,
}

impl Packet for ChangeDifficulty {
    fn packet_id(&self) -> i32 {
        0x03
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ChangeDifficulty {
            difficulty: buf.get_varint()?,
            locked: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.difficulty);
        buf.put_bool(self.locked);
        Ok(())
    }
}

/// 修改游戏模式（serverbound, id 0x04，wire 名 `change_game_mode`）。
///
/// `gamemode` 枚举（Byte）：0=生存，1=创造，2=冒险，3=旁观。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeGameMode {
    /// 游戏模式 id（Byte）。
    pub gamemode: i8,
}

impl Packet for ChangeGameMode {
    fn packet_id(&self) -> i32 {
        0x04
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ChangeGameMode {
            gamemode: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.gamemode);
        Ok(())
    }
}

/// 聊天确认（serverbound, id 0x05，wire 名 `chat_ack`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatAck {
    /// 消息偏移量（VarInt）。
    pub offset: i32,
}

impl Packet for ChatAck {
    fn packet_id(&self) -> i32 {
        0x05
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ChatAck {
            offset: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.offset);
        Ok(())
    }
}

/// 普通聊天消息（serverbound, id 0x08，wire 名 `chat_message`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// 消息文本（String）。
    pub message: String,
    /// 消息时间戳（Long）。
    pub timestamp: i64,
    /// 盐（Long）。
    pub salt: i64,
    /// 消息签名（optional，256 字节原始字节）。
    pub signature: Option<Vec<u8>>,
    /// 确认偏移量（VarInt）。
    pub ack_offset: i32,
    /// 已确认消息位集（FixedBitSet(20)，固定 3 字节）。
    pub ack_list: [u8; 3],
    /// 校验和（Byte）。
    pub checksum: i8,
}

impl Packet for ChatMessage {
    fn packet_id(&self) -> i32 {
        0x08
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let message = buf.get_string()?;
        let timestamp = buf.get_i64()?;
        let salt = buf.get_i64()?;
        let signature = if buf.get_bool()? {
            Some(buf.get_bytes(256)?)
        } else {
            None
        };
        let ack_offset = buf.get_varint()?;
        let mut ack_list = [0u8; 3];
        for item in ack_list.iter_mut() {
            *item = buf.get_u8()?;
        }
        let checksum = buf.get_i8()?;
        Ok(ChatMessage {
            message,
            timestamp,
            salt,
            signature,
            ack_offset,
            ack_list,
            checksum,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.message);
        buf.put_i64(self.timestamp);
        buf.put_i64(self.salt);
        match &self.signature {
            Some(sig) => {
                if sig.len() != 256 {
                    return Err(ProtocolError::InvalidValue);
                }
                buf.put_bool(true);
                buf.put_bytes(sig);
            }
            None => buf.put_bool(false),
        }
        buf.put_varint(self.ack_offset);
        buf.put_bytes(&self.ack_list);
        buf.put_i8(self.checksum);
        Ok(())
    }
}

/// 聊天会话更新（serverbound, id 0x09，wire 名 `chat_session_update`）。
///
/// 会话结构：`session_id`(UUID) + 公钥（过期时刻 Long + DER 公钥 ByteArray +
/// 签名 ByteArray）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSessionUpdate {
    /// 会话 id（UUID）。
    pub session_id: Uuid,
    /// 公钥过期时刻（epoch 毫秒，Long）。
    pub expires_at: i64,
    /// RSA 公钥 DER 字节（ByteArray）。
    pub public_key: Vec<u8>,
    /// 公钥签名（ByteArray）。
    pub signature: Vec<u8>,
}

impl Packet for ChatSessionUpdate {
    fn packet_id(&self) -> i32 {
        0x09
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let session_id = buf.get_uuid()?;
        let expires_at = buf.get_i64()?;
        let public_key = read_byte_array(buf)?;
        let signature = read_byte_array(buf)?;
        Ok(ChatSessionUpdate {
            session_id,
            expires_at,
            public_key,
            signature,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.session_id);
        buf.put_i64(self.expires_at);
        write_byte_array(buf, &self.public_key)?;
        write_byte_array(buf, &self.signature)
    }
}

/// tick 结束（serverbound, id 0x0c，wire 名 `tick_end`），空包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickEnd;

impl Packet for TickEnd {
    fn packet_id(&self) -> i32 {
        0x0c
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(TickEnd)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 客户端设置（serverbound, id 0x0d，wire 名 `client_settings`）。
///
/// `chat_mode` 枚举：0=全部显示，1=仅系统，2=不显示；`main_hand`：0=左手，1=右手；
/// `particle_setting`：0=全部，1=减少，2=最少。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 语言标签（String）。
    pub locale: String,
    /// 视距（Byte）。
    pub view_distance: i8,
    /// 聊天模式（VarInt）。
    pub chat_mode: i32,
    /// 是否显示聊天颜色（Bool）。
    pub chat_colors: bool,
    /// 已显示的皮肤部位位掩码（Byte）。
    pub displayed_skin_parts: i8,
    /// 主手（VarInt）。
    pub main_hand: i32,
    /// 是否启用文本过滤（Bool）。
    pub enable_text_filtering: bool,
    /// 是否允许列入服务器列表（Bool）。
    pub allow_server_listings: bool,
    /// 粒子设置（VarInt）。
    pub particle_setting: i32,
}

impl Packet for Settings {
    fn packet_id(&self) -> i32 {
        0x0d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Settings {
            locale: buf.get_string()?,
            view_distance: buf.get_i8()?,
            chat_mode: buf.get_varint()?,
            chat_colors: buf.get_bool()?,
            displayed_skin_parts: buf.get_i8()?,
            main_hand: buf.get_varint()?,
            enable_text_filtering: buf.get_bool()?,
            allow_server_listings: buf.get_bool()?,
            particle_setting: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.locale);
        buf.put_i8(self.view_distance);
        buf.put_varint(self.chat_mode);
        buf.put_bool(self.chat_colors);
        buf.put_i8(self.displayed_skin_parts);
        buf.put_varint(self.main_hand);
        buf.put_bool(self.enable_text_filtering);
        buf.put_bool(self.allow_server_listings);
        buf.put_varint(self.particle_setting);
        Ok(())
    }
}

/// 命令补全请求（serverbound, id 0x0e，wire 名 `tab_complete`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabComplete {
    /// 事务 id（VarInt）。
    pub transaction_id: i32,
    /// 待补全文本（String）。
    pub text: String,
}

impl Packet for TabComplete {
    fn packet_id(&self) -> i32 {
        0x0e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(TabComplete {
            transaction_id: buf.get_varint()?,
            text: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.transaction_id);
        buf.put_string(&self.text);
        Ok(())
    }
}

/// 配置确认（serverbound, id 0x0f，wire 名 `configuration_ack`），空包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfigurationAck;

impl Packet for ConfigurationAck {
    fn packet_id(&self) -> i32 {
        0x0f
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ConfigurationAck)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 点击窗口按钮（serverbound, id 0x10，wire 名 `click_window_button`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClickWindowButton {
    /// 窗口 id（VarInt）。
    pub window_id: i32,
    /// 按钮 id（VarInt）。
    pub button_id: i32,
}

impl Packet for ClickWindowButton {
    fn packet_id(&self) -> i32 {
        0x10
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClickWindowButton {
            window_id: buf.get_varint()?,
            button_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        buf.put_varint(self.button_id);
        Ok(())
    }
}

/// 窗口槽状态（serverbound, id 0x13，wire 名 `window_slot_state`）。
///
/// 当客户端在合成器界面（crafter）切换某槽启用/禁用时发送。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSlotState {
    /// 槽位（VarInt）。
    pub slot: i32,
    /// 窗口 id（VarInt）。
    pub window_id: i32,
    /// 新状态（Bool）。
    pub new_state: bool,
}

impl Packet for WindowSlotState {
    fn packet_id(&self) -> i32 {
        0x13
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WindowSlotState {
            slot: buf.get_varint()?,
            window_id: buf.get_varint()?,
            new_state: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.slot);
        buf.put_varint(self.window_id);
        buf.put_bool(self.new_state);
        Ok(())
    }
}

/// Cookie 响应（serverbound, id 0x14，wire 名 `cookie_response`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieResponse {
    /// Cookie 键（String）。
    pub key: String,
    /// Cookie 值（optional ByteArray）。
    pub value: Option<Vec<u8>>,
}

impl Packet for CookieResponse {
    fn packet_id(&self) -> i32 {
        0x14
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let key = buf.get_string()?;
        let value = if buf.get_bool()? {
            Some(read_byte_array(buf)?)
        } else {
            None
        };
        Ok(CookieResponse { key, value })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.key);
        match &self.value {
            Some(v) => {
                buf.put_bool(true);
                write_byte_array(buf, v)?;
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 插件消息（serverbound, id 0x15，wire 名 `plugin_message`）。
///
/// 与配置阶段同名包同构，但属于 Play 状态（channel ≤ 256 字符）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPluginMessage {
    /// 通道标识（如 `minecraft:brand`）。
    pub channel: String,
    /// 消息体字节（RAW_BYTES，剩余全部）。
    pub data: Vec<u8>,
}

impl Packet for ClientPluginMessage {
    fn packet_id(&self) -> i32 {
        0x15
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let channel = buf.get_string()?;
        let data = buf.get_bytes(buf.remaining())?;
        Ok(ClientPluginMessage { channel, data })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.channel);
        buf.put_bytes(&self.data);
        Ok(())
    }
}

/// 调试订阅请求（serverbound, id 0x16，wire 名 `debug_subscription_request`）。
///
/// `subscriptions` 为 VarInt 计数的订阅 id 数组（Set）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugSubscriptionRequest {
    /// 订阅 id 列表。
    pub subscriptions: Vec<i32>,
}

impl Packet for DebugSubscriptionRequest {
    fn packet_id(&self) -> i32 {
        0x16
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let subscriptions = read_varint_array(buf, |b| b.get_varint())?;
        Ok(DebugSubscriptionRequest { subscriptions })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.subscriptions, |b, id| {
            b.put_varint(*id);
            Ok(())
        })
    }
}

/// 编辑书本（serverbound, id 0x17，wire 名 `edit_book`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditBook {
    /// 槽位（VarInt）。
    pub slot: i32,
    /// 页面列表（VarInt 计数 + String×N）。
    pub pages: Vec<String>,
    /// 书名（optional String，仅成书签名时存在）。
    pub title: Option<String>,
}

impl Packet for EditBook {
    fn packet_id(&self) -> i32 {
        0x17
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let slot = buf.get_varint()?;
        let pages = read_varint_array(buf, |b| b.get_string())?;
        let title = if buf.get_bool()? {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(EditBook { slot, pages, title })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.slot);
        write_varint_array(buf, &self.pages, |b, p| {
            b.put_string(p);
            Ok(())
        })?;
        match &self.title {
            Some(t) => {
                buf.put_bool(true);
                buf.put_string(t);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 查询实体 NBT（serverbound, id 0x18，wire 名 `query_entity_nbt`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryEntityNbt {
    /// 事务 id（VarInt）。
    pub transaction_id: i32,
    /// 实体 id（VarInt）。
    pub entity_id: i32,
}

impl Packet for QueryEntityNbt {
    fn packet_id(&self) -> i32 {
        0x18
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(QueryEntityNbt {
            transaction_id: buf.get_varint()?,
            entity_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.transaction_id);
        buf.put_varint(self.entity_id);
        Ok(())
    }
}

/// 实体交互类型（`InteractEntity` 的 type 分派）。
#[derive(Debug, Clone, PartialEq)]
pub enum InteractType {
    /// 交互（type 0）：使用哪只手。
    Interact { hand: i32 },
    /// 攻击（type 1）：无附加字段。
    Attack,
    /// 在相对坐标处交互（type 2）：命中坐标 + 手。
    InteractAt {
        target_x: f32,
        target_y: f32,
        target_z: f32,
        hand: i32,
    },
}

/// 交互实体（serverbound, id 0x19，wire 名 `interact_entity`）。
///
/// 线格式：`target_id`(VarInt) + `type`(VarInt，见 [`InteractType`]) + `sneaking`(Bool)。
#[derive(Debug, Clone, PartialEq)]
pub struct InteractEntity {
    /// 目标实体 id（VarInt）。
    pub target_id: i32,
    /// 交互类型与负载。
    pub interact_type: InteractType,
    /// 是否潜行（Bool）。
    pub sneaking: bool,
}

impl Packet for InteractEntity {
    fn packet_id(&self) -> i32 {
        0x19
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let target_id = buf.get_varint()?;
        let type_id = buf.get_varint()?;
        let interact_type = match type_id {
            0 => InteractType::Interact {
                hand: buf.get_varint()?,
            },
            1 => InteractType::Attack,
            2 => InteractType::InteractAt {
                target_x: buf.get_f32()?,
                target_y: buf.get_f32()?,
                target_z: buf.get_f32()?,
                hand: buf.get_varint()?,
            },
            _ => return Err(ProtocolError::InvalidValue),
        };
        let sneaking = buf.get_bool()?;
        Ok(InteractEntity {
            target_id,
            interact_type,
            sneaking,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.target_id);
        match &self.interact_type {
            InteractType::Interact { hand } => {
                buf.put_varint(0);
                buf.put_varint(*hand);
            }
            InteractType::Attack => buf.put_varint(1),
            InteractType::InteractAt {
                target_x,
                target_y,
                target_z,
                hand,
            } => {
                buf.put_varint(2);
                buf.put_f32(*target_x);
                buf.put_f32(*target_y);
                buf.put_f32(*target_z);
                buf.put_varint(*hand);
            }
        }
        buf.put_bool(self.sneaking);
        Ok(())
    }
}

/// 生成结构（serverbound, id 0x1a，wire 名 `generate_structure`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerateStructure {
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 生成层级（VarInt）。
    pub level: i32,
    /// 是否保留 jigsaw（Bool）。
    pub keep_jigsaws: bool,
}

impl Packet for GenerateStructure {
    fn packet_id(&self) -> i32 {
        0x1a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(GenerateStructure {
            block_position: buf.get_position()?,
            level: buf.get_varint()?,
            keep_jigsaws: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_varint(self.level);
        buf.put_bool(self.keep_jigsaws);
        Ok(())
    }
}

/// 锁定难度（serverbound, id 0x1c，wire 名 `lock_difficulty`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockDifficulty {
    /// 是否锁定（Bool）。
    pub locked: bool,
}

impl Packet for LockDifficulty {
    fn packet_id(&self) -> i32 {
        0x1c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(LockDifficulty {
            locked: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_bool(self.locked);
        Ok(())
    }
}

/// 载具移动（serverbound, id 0x21，wire 名 `vehicle_move`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleMove {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    /// 是否在地面（Bool）。
    pub on_ground: bool,
}

impl Packet for VehicleMove {
    fn packet_id(&self) -> i32 {
        0x21
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(VehicleMove {
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 划船（serverbound, id 0x22，wire 名 `steer_boat`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SteerBoat {
    /// 左桨是否转动（Bool）。
    pub left_paddle: bool,
    /// 右桨是否转动（Bool）。
    pub right_paddle: bool,
}

impl Packet for SteerBoat {
    fn packet_id(&self) -> i32 {
        0x22
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SteerBoat {
            left_paddle: buf.get_bool()?,
            right_paddle: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_bool(self.left_paddle);
        buf.put_bool(self.right_paddle);
        Ok(())
    }
}

/// 从方块拾取物品（serverbound, id 0x23，wire 名 `pick_item_from_block`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickItemFromBlock {
    /// 方块坐标（Position 打包）。
    pub pos: (i32, i32, i32),
    /// 是否附带数据（Bool）。
    pub include_data: bool,
}

impl Packet for PickItemFromBlock {
    fn packet_id(&self) -> i32 {
        0x23
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PickItemFromBlock {
            pos: buf.get_position()?,
            include_data: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.pos;
        buf.put_position(x, y, z);
        buf.put_bool(self.include_data);
        Ok(())
    }
}

/// 从实体拾取物品（serverbound, id 0x24，wire 名 `pick_item_from_entity`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickItemFromEntity {
    /// 实体 id（VarInt）。
    pub entity_id: i32,
    /// 是否附带数据（Bool）。
    pub include_data: bool,
}

impl Packet for PickItemFromEntity {
    fn packet_id(&self) -> i32 {
        0x24
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PickItemFromEntity {
            entity_id: buf.get_varint()?,
            include_data: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_bool(self.include_data);
        Ok(())
    }
}

/// Ping 请求（serverbound, id 0x25，wire 名 `ping_request`）。
///
/// 客户端以此测量延迟，服务端应回发相同负载的 clientbound Ping。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingRequest {
    /// 编号（Long）。
    pub number: i64,
}

impl Packet for PingRequest {
    fn packet_id(&self) -> i32 {
        0x25
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PingRequest {
            number: buf.get_i64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.number);
        Ok(())
    }
}

/// 放置合成配方（serverbound, id 0x26，wire 名 `place_recipe`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceRecipe {
    /// 窗口 id（Byte）。
    pub window_id: i8,
    /// 配方展示 id（VarInt）。
    pub recipe_display_id: i32,
    /// 是否全部合成（Bool）。
    pub make_all: bool,
}

impl Packet for PlaceRecipe {
    fn packet_id(&self) -> i32 {
        0x26
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlaceRecipe {
            window_id: buf.get_i8()?,
            recipe_display_id: buf.get_varint()?,
            make_all: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.window_id);
        buf.put_varint(self.recipe_display_id);
        buf.put_bool(self.make_all);
        Ok(())
    }
}

/// 玩家能力（serverbound, id 0x27，wire 名 `player_abilities`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAbilities {
    /// 能力标志位（Byte）。
    pub flags: i8,
}

impl Packet for PlayerAbilities {
    fn packet_id(&self) -> i32 {
        0x27
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerAbilities {
            flags: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.flags);
        Ok(())
    }
}

/// 玩家动作（serverbound, id 0x28，wire 名 `player_action`）。
///
/// `status` 枚举（VarInt）：0=开始挖掘，1=取消挖掘，2=完成挖掘，3=丢整堆，
/// 4=丢单件，5=更新物品状态，6=交换手持，7=刺击。`block_face` 为 Byte（
/// 0=下，1=上，2=北，3=南，4=西，5=东）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAction {
    /// 动作（VarInt）。
    pub status: i32,
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 方块面（Byte）。
    pub block_face: i8,
    /// 序列号（VarInt）。
    pub sequence: i32,
}

impl Packet for PlayerAction {
    fn packet_id(&self) -> i32 {
        0x28
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerAction {
            status: buf.get_varint()?,
            block_position: buf.get_position()?,
            block_face: buf.get_i8()?,
            sequence: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.status);
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_i8(self.block_face);
        buf.put_varint(self.sequence);
        Ok(())
    }
}

/// 实体动作（serverbound, id 0x29，wire 名 `entity_action`）。
///
/// `action` 枚举（VarInt）：0=离开床，1=开始疾跑，2=停止疾跑，3=开始跳马，
/// 4=停止跳马，5=打开马匹背包，6=开始鞘翅滑翔。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityAction {
    /// 玩家 id（VarInt）。
    pub player_id: i32,
    /// 动作（VarInt）。
    pub action: i32,
    /// 马匹跳跃加成（VarInt）。
    pub horse_jump_boost: i32,
}

impl Packet for EntityAction {
    fn packet_id(&self) -> i32 {
        0x29
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityAction {
            player_id: buf.get_varint()?,
            action: buf.get_varint()?,
            horse_jump_boost: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.player_id);
        buf.put_varint(self.action);
        buf.put_varint(self.horse_jump_boost);
        Ok(())
    }
}

/// 玩家输入（serverbound, id 0x2a，wire 名 `player_input`）。
///
/// 单字节位掩码：bit0-6 分别表示前进 / 后退 / 左 / 右 / 跳跃 / 潜行 / 疾跑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    /// 输入标志位（Byte）。
    pub flags: i8,
}

impl Input {
    /// 前进（bit0）。
    pub const FLAG_FORWARD: i8 = 0x01;
    /// 后退（bit1）。
    pub const FLAG_BACKWARD: i8 = 0x02;
    /// 左移（bit2）。
    pub const FLAG_LEFT: i8 = 0x04;
    /// 右移（bit3）。
    pub const FLAG_RIGHT: i8 = 0x08;
    /// 跳跃（bit4）。
    pub const FLAG_JUMP: i8 = 0x10;
    /// 潜行（bit5）。
    pub const FLAG_SHIFT: i8 = 0x20;
    /// 疾跑（bit6）。
    pub const FLAG_SPRINT: i8 = 0x40;

    /// 由 7 个布尔键位构造 `flags`。
    pub fn new(
        forward: bool,
        backward: bool,
        left: bool,
        right: bool,
        jump: bool,
        shift: bool,
        sprint: bool,
    ) -> Self {
        let mut flags = 0i8;
        if forward {
            flags |= Self::FLAG_FORWARD;
        }
        if backward {
            flags |= Self::FLAG_BACKWARD;
        }
        if left {
            flags |= Self::FLAG_LEFT;
        }
        if right {
            flags |= Self::FLAG_RIGHT;
        }
        if jump {
            flags |= Self::FLAG_JUMP;
        }
        if shift {
            flags |= Self::FLAG_SHIFT;
        }
        if sprint {
            flags |= Self::FLAG_SPRINT;
        }
        Self { flags }
    }

    /// 是否前进（bit0）。
    pub fn forward(&self) -> bool {
        self.flags & Self::FLAG_FORWARD != 0
    }
    /// 是否后退（bit1）。
    pub fn backward(&self) -> bool {
        self.flags & Self::FLAG_BACKWARD != 0
    }
    /// 是否左移（bit2）。
    pub fn left(&self) -> bool {
        self.flags & Self::FLAG_LEFT != 0
    }
    /// 是否右移（bit3）。
    pub fn right(&self) -> bool {
        self.flags & Self::FLAG_RIGHT != 0
    }
    /// 是否跳跃（bit4）。
    pub fn jump(&self) -> bool {
        self.flags & Self::FLAG_JUMP != 0
    }
    /// 是否潜行（bit5）。
    pub fn shift(&self) -> bool {
        self.flags & Self::FLAG_SHIFT != 0
    }
    /// 是否疾跑（bit6）。
    pub fn sprint(&self) -> bool {
        self.flags & Self::FLAG_SPRINT != 0
    }
}

impl Packet for Input {
    fn packet_id(&self) -> i32 {
        0x2a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Input {
            flags: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.flags);
        Ok(())
    }
}

/// Pong 响应（serverbound, id 0x2c，wire 名 `pong`）。
///
/// 客户端对 clientbound Ping 的回复，`id` 与请求一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    /// id（Int）。
    pub id: i32,
}

impl Packet for Pong {
    fn packet_id(&self) -> i32 {
        0x2c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Pong { id: buf.get_i32()? })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i32(self.id);
        Ok(())
    }
}

/// 设置配方书状态（serverbound, id 0x2d，wire 名 `set_recipe_book_state`）。
///
/// `book_type` 枚举（VarInt）：0=合成台，1=熔炉，2=高炉，3=烟熏炉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetRecipeBookState {
    /// 配方书类型（VarInt）。
    pub book_type: i32,
    /// 是否展开（Bool）。
    pub book_open: bool,
    /// 是否启用过滤器（Bool）。
    pub filter_active: bool,
}

impl Packet for SetRecipeBookState {
    fn packet_id(&self) -> i32 {
        0x2d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetRecipeBookState {
            book_type: buf.get_varint()?,
            book_open: buf.get_bool()?,
            filter_active: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.book_type);
        buf.put_bool(self.book_open);
        buf.put_bool(self.filter_active);
        Ok(())
    }
}

/// 配方书查看配方（serverbound, id 0x2e，wire 名 `recipe_book_seen_recipe`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecipeBookSeenRecipe {
    /// 配方索引（VarInt）。
    pub recipe_id: i32,
}

impl Packet for RecipeBookSeenRecipe {
    fn packet_id(&self) -> i32 {
        0x2e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(RecipeBookSeenRecipe {
            recipe_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.recipe_id);
        Ok(())
    }
}

/// 命名物品（serverbound, id 0x2f，wire 名 `name_item`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameItem {
    /// 物品新名称（String）。
    pub item_name: String,
}

impl Packet for NameItem {
    fn packet_id(&self) -> i32 {
        0x2f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(NameItem {
            item_name: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.item_name);
        Ok(())
    }
}

/// 资源包状态（serverbound, id 0x30，wire 名 `resource_pack_status`）。
///
/// `status` 枚举（VarInt）：0=加载成功，1=拒绝，2=下载失败，3=已接受，
/// 4=已下载，5=URL 无效，6=重载失败，7=已丢弃。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePackStatus {
    /// 资源包 id（UUID）。
    pub id: Uuid,
    /// 状态（VarInt）。
    pub status: i32,
}

impl Packet for ResourcePackStatus {
    fn packet_id(&self) -> i32 {
        0x30
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ResourcePackStatus {
            id: buf.get_uuid()?,
            status: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.id);
        buf.put_varint(self.status);
        Ok(())
    }
}

/// 进度标签页（serverbound, id 0x31，wire 名 `advancement_tab`）。
///
/// `action` 枚举（VarInt）：0=打开标签页（后跟 String 标识），1=关闭界面（无负载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancementTab {
    /// 动作（VarInt）。
    pub action: i32,
    /// 标签页标识（仅在 action=0 时存在）。
    pub tab_identifier: Option<String>,
}

impl Packet for AdvancementTab {
    fn packet_id(&self) -> i32 {
        0x31
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let action = buf.get_varint()?;
        let tab_identifier = if action == 0 {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(AdvancementTab {
            action,
            tab_identifier,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.action);
        if self.action == 0 {
            match &self.tab_identifier {
                Some(t) => buf.put_string(t),
                None => return Err(ProtocolError::InvalidValue),
            }
        }
        Ok(())
    }
}

/// 选择交易（serverbound, id 0x32，wire 名 `select_trade`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectTrade {
    /// 选中槽位（VarInt）。
    pub selected_slot: i32,
}

impl Packet for SelectTrade {
    fn packet_id(&self) -> i32 {
        0x32
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SelectTrade {
            selected_slot: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.selected_slot);
        Ok(())
    }
}

/// 设置信标效果（serverbound, id 0x33，wire 名 `set_beacon_effect`）。
///
/// 主 / 次效果均为 optional 药水类型 id（VarInt），`None` 表示未选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetBeaconEffect {
    /// 主效果（optional VarInt）。
    pub primary_effect: Option<i32>,
    /// 次效果（optional VarInt）。
    pub secondary_effect: Option<i32>,
}

impl Packet for SetBeaconEffect {
    fn packet_id(&self) -> i32 {
        0x33
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let primary_effect = if buf.get_bool()? {
            Some(buf.get_varint()?)
        } else {
            None
        };
        let secondary_effect = if buf.get_bool()? {
            Some(buf.get_varint()?)
        } else {
            None
        };
        Ok(SetBeaconEffect {
            primary_effect,
            secondary_effect,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match self.primary_effect {
            Some(v) => {
                buf.put_bool(true);
                buf.put_varint(v);
            }
            None => buf.put_bool(false),
        }
        match self.secondary_effect {
            Some(v) => {
                buf.put_bool(true);
                buf.put_varint(v);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 更新命令方块（serverbound, id 0x35，wire 名 `update_command_block`）。
///
/// `mode` 枚举（VarInt）：0=顺序，1=自动，2=红石。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCommandBlock {
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 命令文本（String）。
    pub command: String,
    /// 模式（VarInt）。
    pub mode: i32,
    /// 标志位（Byte）。
    pub flags: i8,
}

impl Packet for UpdateCommandBlock {
    fn packet_id(&self) -> i32 {
        0x35
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateCommandBlock {
            block_position: buf.get_position()?,
            command: buf.get_string()?,
            mode: buf.get_varint()?,
            flags: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_string(&self.command);
        buf.put_varint(self.mode);
        buf.put_i8(self.flags);
        Ok(())
    }
}

/// 更新命令方块矿车（serverbound, id 0x36，wire 名 `update_command_block_minecart`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCommandBlockMinecart {
    /// 实体 id（VarInt）。
    pub entity_id: i32,
    /// 命令文本（String）。
    pub command: String,
    /// 是否记录输出（Bool）。
    pub track_output: bool,
}

impl Packet for UpdateCommandBlockMinecart {
    fn packet_id(&self) -> i32 {
        0x36
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateCommandBlockMinecart {
            entity_id: buf.get_varint()?,
            command: buf.get_string()?,
            track_output: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_string(&self.command);
        buf.put_bool(self.track_output);
        Ok(())
    }
}

/// 创造模式库存动作（serverbound, id 0x37，wire 名 `creative_inventory_action`）。
#[derive(Debug, Clone, PartialEq)]
pub struct CreativeInventoryAction {
    /// 槽位（Short，-999 表示丢弃光标物品）。
    pub slot: i16,
    /// 放置的物品（ItemStack）。
    pub item: ItemStack,
}

impl Packet for CreativeInventoryAction {
    fn packet_id(&self) -> i32 {
        0x37
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(CreativeInventoryAction {
            slot: buf.get_i16()?,
            item: decode_item_stack(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i16(self.slot);
        encode_item_stack(&self.item, buf)
    }
}

/// 更新拼图方块（serverbound, id 0x38，wire 名 `update_jigsaw_block`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateJigsawBlock {
    /// 方块坐标（Position 打包）。
    pub location: (i32, i32, i32),
    /// 名称（String）。
    pub name: String,
    /// 目标（String）。
    pub target: String,
    /// 池（String）。
    pub pool: String,
    /// 最终状态（String）。
    pub final_state: String,
    /// 连接类型（String）。
    pub joint_type: String,
    /// 选择优先级（VarInt）。
    pub selection_priority: i32,
    /// 放置优先级（VarInt）。
    pub placement_priority: i32,
}

impl Packet for UpdateJigsawBlock {
    fn packet_id(&self) -> i32 {
        0x38
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateJigsawBlock {
            location: buf.get_position()?,
            name: buf.get_string()?,
            target: buf.get_string()?,
            pool: buf.get_string()?,
            final_state: buf.get_string()?,
            joint_type: buf.get_string()?,
            selection_priority: buf.get_varint()?,
            placement_priority: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.location;
        buf.put_position(x, y, z);
        buf.put_string(&self.name);
        buf.put_string(&self.target);
        buf.put_string(&self.pool);
        buf.put_string(&self.final_state);
        buf.put_string(&self.joint_type);
        buf.put_varint(self.selection_priority);
        buf.put_varint(self.placement_priority);
        Ok(())
    }
}

/// 更新结构方块（serverbound, id 0x39，wire 名 `update_structure_block`）。
///
/// `action` 枚举（VarInt）：0=更新数据，1=保存，2=加载，3=检测尺寸；
/// `mode`：0=保存，1=加载，2=角落，3=数据；`mirror`：0=无，1=左右，2=前后；
/// `rotation`（VarInt）仅取 0-3 的 90° 旋转。`offset` / `size` 各为 3×Byte。
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStructureBlock {
    /// 方块坐标（Position 打包）。
    pub location: (i32, i32, i32),
    /// 动作（VarInt）。
    pub action: i32,
    /// 模式（VarInt）。
    pub mode: i32,
    /// 名称（String）。
    pub name: String,
    /// 偏移（3×Byte）。
    pub offset: (i8, i8, i8),
    /// 尺寸（3×Byte）。
    pub size: (i8, i8, i8),
    /// 镜像（VarInt）。
    pub mirror: i32,
    /// 旋转（VarInt，0-3）。
    pub rotation: i32,
    /// 元数据（String）。
    pub metadata: String,
    /// 完整度（Float）。
    pub integrity: f32,
    /// 种子（Long）。
    pub seed: i64,
    /// 标志位（Byte）。
    pub flags: i8,
}

impl Packet for UpdateStructureBlock {
    fn packet_id(&self) -> i32 {
        0x39
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateStructureBlock {
            location: buf.get_position()?,
            action: buf.get_varint()?,
            mode: buf.get_varint()?,
            name: buf.get_string()?,
            offset: (buf.get_i8()?, buf.get_i8()?, buf.get_i8()?),
            size: (buf.get_i8()?, buf.get_i8()?, buf.get_i8()?),
            mirror: buf.get_varint()?,
            rotation: buf.get_varint()?,
            metadata: buf.get_string()?,
            integrity: buf.get_f32()?,
            seed: buf.get_i64()?,
            flags: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.location;
        buf.put_position(x, y, z);
        buf.put_varint(self.action);
        buf.put_varint(self.mode);
        buf.put_string(&self.name);
        let (ox, oy, oz) = self.offset;
        buf.put_i8(ox);
        buf.put_i8(oy);
        buf.put_i8(oz);
        let (sx, sy, sz) = self.size;
        buf.put_i8(sx);
        buf.put_i8(sy);
        buf.put_i8(sz);
        buf.put_varint(self.mirror);
        buf.put_varint(self.rotation);
        buf.put_string(&self.metadata);
        buf.put_f32(self.integrity);
        buf.put_i64(self.seed);
        buf.put_i8(self.flags);
        Ok(())
    }
}

/// 设置测试方块（serverbound, id 0x3a，wire 名 `set_test_block`）。
///
/// `mode` 枚举（VarInt）：0=开始，1=日志，2=失败，3=接受。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTestBlock {
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 模式（VarInt）。
    pub mode: i32,
    /// 消息（String）。
    pub message: String,
}

impl Packet for SetTestBlock {
    fn packet_id(&self) -> i32 {
        0x3a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetTestBlock {
            block_position: buf.get_position()?,
            mode: buf.get_varint()?,
            message: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_varint(self.mode);
        buf.put_string(&self.message);
        Ok(())
    }
}

/// 更新告示牌（serverbound, id 0x3b，wire 名 `update_sign`）。
///
/// 线格式固定为 4 个 String 行（正面 / 背面由 `is_front_text` 决定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSign {
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 是否为正面文本（Bool）。
    pub is_front_text: bool,
    /// 4 行文本（String×4）。
    pub lines: Vec<String>,
}

impl Packet for UpdateSign {
    fn packet_id(&self) -> i32 {
        0x3b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let block_position = buf.get_position()?;
        let is_front_text = buf.get_bool()?;
        let lines = vec![
            buf.get_string()?,
            buf.get_string()?,
            buf.get_string()?,
            buf.get_string()?,
        ];
        Ok(UpdateSign {
            block_position,
            is_front_text,
            lines,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        if self.lines.len() != 4 {
            return Err(ProtocolError::InvalidValue);
        }
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_bool(self.is_front_text);
        for line in &self.lines {
            buf.put_string(line);
        }
        Ok(())
    }
}

/// 手臂动画（serverbound, id 0x3c，wire 名 `animation`）。
///
/// `hand` 枚举（VarInt）：0=主手，1=副手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Animation {
    /// 手（VarInt）。
    pub hand: i32,
}

impl Packet for Animation {
    fn packet_id(&self) -> i32 {
        0x3c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Animation {
            hand: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.hand);
        Ok(())
    }
}

/// 旁观目标（serverbound, id 0x3d，wire 名 `spectate`）。
///
/// 客户端通过热键栏在实体间切换时发送，用于将玩家传送到目标实体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spectate {
    /// 目标实体 UUID。
    pub target: Uuid,
}

impl Packet for Spectate {
    fn packet_id(&self) -> i32 {
        0x3d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Spectate {
            target: buf.get_uuid()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.target);
        Ok(())
    }
}

/// 测试实例方块动作（serverbound, id 0x3e，wire 名 `test_instance_block_action`）。
///
/// `action` 枚举（VarInt）：0=初始化，1=查询，2=设置，3=重置，4=保存，5=导出，6=运行；
/// 数据中 `status`：0=已清除，1=运行中，2=已完成。`error_message` 为 optional 组件
/// （NBT 原始字节，含前导 `TAG_COMPOUND`(0x0a)），本框架不解析其内部结构。
#[derive(Debug, Clone, PartialEq)]
pub struct TestInstanceBlockActionData {
    /// 测试名（optional String）。
    pub test: Option<String>,
    /// 尺寸（3×VarInt）。
    pub size: (i32, i32, i32),
    /// 旋转（VarInt）。
    pub rotation: i32,
    /// 是否忽略实体（Bool）。
    pub ignore_entities: bool,
    /// 状态（VarInt）。
    pub status: i32,
    /// 错误消息组件（optional，原始 NBT 字节含 tag id）。
    pub error_message: Option<Vec<u8>>,
}

/// 测试实例方块动作（serverbound, id 0x3e，wire 名 `test_instance_block_action`）。
#[derive(Debug, Clone, PartialEq)]
pub struct TestInstanceBlockAction {
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 动作（VarInt）。
    pub action: i32,
    /// 数据负载。
    pub data: TestInstanceBlockActionData,
}

impl Packet for TestInstanceBlockAction {
    fn packet_id(&self) -> i32 {
        0x3e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let block_position = buf.get_position()?;
        let action = buf.get_varint()?;
        let test = if buf.get_bool()? {
            Some(buf.get_string()?)
        } else {
            None
        };
        let size = (buf.get_varint()?, buf.get_varint()?, buf.get_varint()?);
        let rotation = buf.get_varint()?;
        let ignore_entities = buf.get_bool()?;
        let status = buf.get_varint()?;
        let error_message = if buf.get_bool()? {
            // 组件以 TAG_COMPOUND(0x0a) 起始，NBT 自定界：解析出实际消费字节数。
            let tag_id = buf.get_u8()?;
            if tag_id != 0x0a {
                return Err(ProtocolError::InvalidValue);
            }
            let rest = buf.get_bytes(buf.remaining())?;
            let (_tag, consumed) =
                nbt::decode_anonymous(&rest).map_err(|_| ProtocolError::UnexpectedEof)?;
            // 还原为 tag id + payload 的原始字节。
            let mut raw = Vec::with_capacity(1 + consumed);
            raw.push(0x0a);
            raw.extend_from_slice(&rest[..consumed]);
            Some(raw)
        } else {
            None
        };
        Ok(TestInstanceBlockAction {
            block_position,
            action,
            data: TestInstanceBlockActionData {
                test,
                size,
                rotation,
                ignore_entities,
                status,
                error_message,
            },
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_varint(self.action);
        match &self.data.test {
            Some(t) => {
                buf.put_bool(true);
                buf.put_string(t);
            }
            None => buf.put_bool(false),
        }
        let (sx, sy, sz) = self.data.size;
        buf.put_varint(sx);
        buf.put_varint(sy);
        buf.put_varint(sz);
        buf.put_varint(self.data.rotation);
        buf.put_bool(self.data.ignore_entities);
        buf.put_varint(self.data.status);
        match &self.data.error_message {
            Some(raw) => {
                buf.put_bool(true);
                buf.put_bytes(raw);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 玩家方块放置（serverbound, id 0x3f，wire 名 `player_block_placement`）。
///
/// `hand`（VarInt）：0=主手，1=副手；`block_face`（VarInt）：0=下，1=上，2=北，
/// 3=南，4=西，5=东。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerBlockPlacement {
    /// 手（VarInt）。
    pub hand: i32,
    /// 方块坐标（Position 打包）。
    pub block_position: (i32, i32, i32),
    /// 方块面（VarInt）。
    pub block_face: i32,
    /// 光标位置 X（Float）。
    pub cursor_position_x: f32,
    /// 光标位置 Y（Float）。
    pub cursor_position_y: f32,
    /// 光标位置 Z（Float）。
    pub cursor_position_z: f32,
    /// 是否在方块内部（Bool）。
    pub inside_block: bool,
    /// 是否命中世界边界（Bool）。
    pub hit_world_border: bool,
    /// 序列号（VarInt）。
    pub sequence: i32,
}

impl Packet for PlayerBlockPlacement {
    fn packet_id(&self) -> i32 {
        0x3f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerBlockPlacement {
            hand: buf.get_varint()?,
            block_position: buf.get_position()?,
            block_face: buf.get_varint()?,
            cursor_position_x: buf.get_f32()?,
            cursor_position_y: buf.get_f32()?,
            cursor_position_z: buf.get_f32()?,
            inside_block: buf.get_bool()?,
            hit_world_border: buf.get_bool()?,
            sequence: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.hand);
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_varint(self.block_face);
        buf.put_f32(self.cursor_position_x);
        buf.put_f32(self.cursor_position_y);
        buf.put_f32(self.cursor_position_z);
        buf.put_bool(self.inside_block);
        buf.put_bool(self.hit_world_border);
        buf.put_varint(self.sequence);
        Ok(())
    }
}

/// 使用物品（serverbound, id 0x40，wire 名 `use_item`）。
///
/// `hand`（VarInt）：0=主手，1=副手。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseItem {
    /// 手（VarInt）。
    pub hand: i32,
    /// 序列号（VarInt）。
    pub sequence: i32,
    /// 偏航角（Float）。
    pub yaw: f32,
    /// 俯仰角（Float）。
    pub pitch: f32,
}

impl Packet for UseItem {
    fn packet_id(&self) -> i32 {
        0x40
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UseItem {
            hand: buf.get_varint()?,
            sequence: buf.get_varint()?,
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.hand);
        buf.put_varint(self.sequence);
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        Ok(())
    }
}

/// 自定义点击动作（serverbound, id 0x41，wire 名 `custom_click_action`）。
///
/// `payload` 为长度前缀（VarInt）的 NBT 原始字节，本框架不解析其内部结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomClickAction {
    /// 动作键（Identifier，String）。
    pub key: String,
    /// 负载（lengthPrefixed NBT 原始字节）。
    pub payload: Vec<u8>,
}

impl Packet for CustomClickAction {
    fn packet_id(&self) -> i32 {
        0x41
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let key = buf.get_string()?;
        let payload = read_byte_array(buf)?;
        Ok(CustomClickAction { key, payload })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.key);
        write_byte_array(buf, &self.payload)
    }
}

/// 手持物品切换确认（clientbound, id 0x67，wire 名 `held_item_change`）。
///
/// 服务端在接受切换后回发给客户端。线格式仅 `slot`(BYTE)。
/// 见 `.specs/implement-item-inventory/`。
#[derive(Debug, Clone, PartialEq)]
pub struct HeldItemChange {
    /// 已生效的手持槽（BYTE，0-8）。
    pub slot: i8,
}

impl Packet for HeldItemChange {
    fn packet_id(&self) -> i32 {
        0x67
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let slot = buf.get_i8()?;
        Ok(Self { slot })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.slot);
        Ok(())
    }
}

// ============================ clientbound ============================

/// 系统聊天（clientbound, id 0x77，wire 名 `system_chat`）：服务器向客户端回发的命令反馈。
///
/// 线格式为 `TAG_COMPOUND`(0x0a) 后接 anonymous NBT（Compound，含 `text` 字符串键），
/// 再接 `overlay`(BOOL) 一字节。消息文本以 NBT Compound 形式承载，编码使用
/// [`crate::protocol::nbt::encode_anonymous`]，解码使用
/// [`crate::protocol::nbt::decode_anonymous`]，见 `.specs/implement-command-framework/`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemChatPacket {
    /// 聊天内容文本（取自 NBT Compound 的 `text` 键）。
    pub message: String,
    /// 是否以覆盖层（action bar）形式展示。
    pub overlay: bool,
}

impl Packet for SystemChatPacket {
    fn packet_id(&self) -> i32 {
        0x77
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        // 前导 TAG_COMPOUND（0x0a），与 encode_anonymous 互补（后者不写 0x0a）。
        let tag_id = buf.get_u8()?;
        if tag_id != 0x0a {
            return Err(ProtocolError::UnexpectedEof);
        }
        // NBT 自界定，overlay 固定占据末尾 1 字节；故 NBT 长度为「剩余 - 1」。
        let rest = buf.remaining();
        if rest < 1 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let nbt_len = rest - 1;
        let nbt_bytes = buf.get_bytes(nbt_len)?;
        let (tag, _consumed) =
            nbt::decode_anonymous(&nbt_bytes).map_err(|_| ProtocolError::UnexpectedEof)?;
        let message = match tag {
            NbtTag::Compound(entries) => entries
                .into_iter()
                .find(|(k, _)| k == "text")
                .and_then(|(_, v)| {
                    if let NbtTag::String(s) = v {
                        Some(s)
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
        let overlay = buf.get_bool()?;
        Ok(Self { message, overlay })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let compound = NbtTag::Compound(vec![(
            "text".to_string(),
            NbtTag::String(self.message.clone()),
        )]);
        let nbt = nbt::encode_anonymous(&compound).map_err(|_| ProtocolError::UnexpectedEof)?;
        buf.put_u8(0x0a);
        buf.put_bytes(&nbt);
        buf.put_bool(self.overlay);
        Ok(())
    }
}

/// 生成实体（clientbound, id 0x01，wire 名 `spawn_entity`）。
///
/// 1.21.11 中玩家生成同样走此包（`SpawnPlayer` 已废弃）。`velocity` 为 3×i16 定点
/// 分量（units/8000）；`pitch`/`yaw`/`head_pitch` 为角度 256 分度制的有符号字节。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnEntity {
    pub entity_id: i32,
    pub object_uuid: Uuid,
    /// 实体类型 id（注册表 `minecraft:entity_type` 的协议序号）。
    pub entity_type: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub velocity: [i16; 3],
    pub pitch: i8,
    pub yaw: i8,
    pub head_pitch: i8,
    pub object_data: i32,
}

impl Packet for SpawnEntity {
    fn packet_id(&self) -> i32 {
        0x01
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SpawnEntity {
            entity_id: buf.get_varint()?,
            object_uuid: buf.get_uuid()?,
            entity_type: buf.get_varint()?,
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            velocity: [buf.get_i16()?, buf.get_i16()?, buf.get_i16()?],
            pitch: buf.get_i8()?,
            yaw: buf.get_i8()?,
            head_pitch: buf.get_i8()?,
            object_data: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_uuid(self.object_uuid);
        buf.put_varint(self.entity_type);
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        for v in self.velocity {
            buf.put_i16(v);
        }
        buf.put_i8(self.pitch);
        buf.put_i8(self.yaw);
        buf.put_i8(self.head_pitch);
        buf.put_varint(self.object_data);
        Ok(())
    }
}

/// 玩家位置与朝向（clientbound, id 0x46，wire 名 `position`）：用于出生 / 传送。
///
/// 1.21.2+ 格式：`teleport_id` 在首位，坐标后跟增量 `dx/dy/dz`（无位移时全 0），
/// 不再有 `dismount_vehicle` 位；`flags` 为相对位移位掩码（u8）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub teleport_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub dx: f64,
    pub dy: f64,
    pub dz: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub flags: u8,
}

impl Packet for Position {
    fn packet_id(&self) -> i32 {
        0x46
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Position {
            teleport_id: buf.get_varint()?,
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            dx: buf.get_f64()?,
            dy: buf.get_f64()?,
            dz: buf.get_f64()?,
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
            flags: buf.get_u8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.teleport_id);
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_f64(self.dx);
        buf.put_f64(self.dy);
        buf.put_f64(self.dz);
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        buf.put_u8(self.flags);
        Ok(())
    }
}

/// 更新生命值（clientbound, id 0x66，wire 名 `update_health`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdateHealth {
    pub health: f32,
    pub food: i32,
    pub food_saturation: f32,
}

impl Packet for UpdateHealth {
    fn packet_id(&self) -> i32 {
        0x66
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateHealth {
            health: buf.get_f32()?,
            food: buf.get_varint()?,
            food_saturation: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.health);
        buf.put_varint(self.food);
        buf.put_f32(self.food_saturation);
        Ok(())
    }
}

/// 玩家信息更新（clientbound, id 0x44，wire 名 `player_info`）。
///
/// 最小实现仅支持 ADD_PLAYER 动作（位掩码 0x01）：单条目，含名字与皮肤属性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfo {
    pub uuid: Uuid,
    pub name: String,
    pub properties: Vec<Property>,
}

impl PlayerInfo {
    /// ADD_PLAYER 动作位（位掩码 bit0）。
    const ACTION_ADD_PLAYER: i32 = 0x01;
}

impl Packet for PlayerInfo {
    fn packet_id(&self) -> i32 {
        0x44
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let _actions = buf.get_varint()?;
        let count = buf.get_varint()?;
        let count_usize = usize::try_from(count).map_err(|_| ProtocolError::UnexpectedEof)?;
        // 最小实现仅取第一个条目；多余条目丢弃（占位，供将来扩展）
        if count_usize == 0 {
            return Err(ProtocolError::UnexpectedEof);
        }
        let uuid = buf.get_uuid()?;
        let name = buf.get_string()?;
        let prop_count = buf.get_varint()?;
        let prop_count_usize =
            usize::try_from(prop_count).map_err(|_| ProtocolError::UnexpectedEof)?;
        let mut properties = Vec::with_capacity(prop_count_usize);
        for _ in 0..prop_count_usize {
            let p_name = buf.get_string()?;
            let value = buf.get_string()?;
            let has_sig = buf.get_bool()?;
            let signature = if has_sig {
                Some(buf.get_string()?)
            } else {
                None
            };
            properties.push(Property {
                name: p_name,
                value,
                signature,
            });
        }
        Ok(PlayerInfo {
            uuid,
            name,
            properties,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(Self::ACTION_ADD_PLAYER);
        buf.put_varint(1);
        buf.put_uuid(self.uuid);
        buf.put_string(&self.name);
        let prop_count =
            i32::try_from(self.properties.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
        buf.put_varint(prop_count);
        for p in &self.properties {
            buf.put_string(&p.name);
            buf.put_string(&p.value);
            match &p.signature {
                Some(s) => {
                    buf.put_bool(true);
                    buf.put_string(s);
                }
                None => buf.put_bool(false),
            }
        }
        Ok(())
    }
}

/// 游戏状态变更（clientbound, id 0x26，wire 名 `game_state_change`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameStateChange {
    pub event: i32,
    pub data: f32,
}
impl Packet for GameStateChange {
    fn packet_id(&self) -> i32 {
        0x26
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(GameStateChange {
            event: buf.get_varint()?,
            data: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.event);
        buf.put_f32(self.data);
        Ok(())
    }
}

/// 玩家移除（clientbound, id 0x43，wire 名 `player_remove`）。
///
/// 玩家退出时向其他在线玩家广播，使其从玩家列表中移除该玩家。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerRemove {
    pub players: Vec<Uuid>,
}

impl Packet for PlayerRemove {
    fn packet_id(&self) -> i32 {
        0x43
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let players = read_varint_array(buf, |b| b.get_uuid())?;
        Ok(PlayerRemove { players })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.players, |b, uuid| {
            b.put_uuid(*uuid);
            Ok(())
        })
    }
}

/// 实体传送（clientbound, id 0x7b，wire 名 `entity_teleport`）：绝对坐标同步。
///
/// 移动距离超过阈值时使用绝对坐标传送；yaw/pitch 为 256 分度制有符号字节。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityTeleport {
    pub entity_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: i8,
    pub pitch: i8,
    pub on_ground: bool,
}

impl Packet for EntityTeleport {
    fn packet_id(&self) -> i32 {
        0x7b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityTeleport {
            entity_id: buf.get_varint()?,
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            yaw: buf.get_i8()?,
            pitch: buf.get_i8()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_i8(self.yaw);
        buf.put_i8(self.pitch);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 实体相对移动（clientbound, id 0x33，wire 名 `rel_entity_move`）：小位移同步。
///
/// `dX/dY/dZ` 为 1/4096 方块单位的定点增量，仅用于位移不超过 ±8 方块的场景。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelEntityMove {
    pub entity_id: i32,
    pub d_x: i16,
    pub d_y: i16,
    pub d_z: i16,
    pub on_ground: bool,
}

impl Packet for RelEntityMove {
    fn packet_id(&self) -> i32 {
        0x33
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(RelEntityMove {
            entity_id: buf.get_varint()?,
            d_x: buf.get_i16()?,
            d_y: buf.get_i16()?,
            d_z: buf.get_i16()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_i16(self.d_x);
        buf.put_i16(self.d_y);
        buf.put_i16(self.d_z);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 元数据值类型（Entity Metadata 协议 id）。
const METADATA_TYPE_BYTE: i32 = 0;
const METADATA_TYPE_VARINT: i32 = 1;
const METADATA_TYPE_FLOAT: i32 = 2;
const METADATA_TYPE_STRING: i32 = 3;
const METADATA_TYPE_BOOLEAN: i32 = 7;
/// 条目循环终止标记（index == 0xFF）。
const METADATA_TERMINATOR: u8 = 0xFF;

/// 实体元数据（clientbound, id 0x61，wire 名 `entity_metadata`）。
///
/// 向客户端同步实体的外观 / 状态元数据。线格式：
/// `entity_id`(VarInt) + 条目循环（`index`(Byte) + `type`(VarInt) + `value`，
/// 直到 `index == 0xFF` 终止）+ `has_metadata`(Bool)。当前 v1 值类型支持
/// Byte / VarInt / Float / String / Bool，对应 `EntityMetadataValue` 各变体。
#[derive(Debug, Clone, PartialEq)]
pub struct EntityMetaData {
    /// 实体 id。
    pub entity_id: i32,
    /// 元数据条目（协议 index → 值）。
    pub entries: Vec<(u8, EntityMetadataValue)>,
    /// 是否还有未随本包发送的元数据。
    pub has_metadata: bool,
}

impl Packet for EntityMetaData {
    fn packet_id(&self) -> i32 {
        0x61
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_id = buf.get_varint()?;
        let mut entries = Vec::new();
        loop {
            let index = buf.get_u8()?;
            if index == METADATA_TERMINATOR {
                break;
            }
            let ty = buf.get_varint()?;
            let value = match ty {
                METADATA_TYPE_BYTE => {
                    let raw = buf.get_i8()?;
                    EntityMetadataValue::Byte(u8::from_ne_bytes(raw.to_ne_bytes()))
                }
                METADATA_TYPE_VARINT => EntityMetadataValue::VarInt(buf.get_varint()?),
                METADATA_TYPE_FLOAT => EntityMetadataValue::Float(buf.get_f32()?),
                METADATA_TYPE_STRING => EntityMetadataValue::String(buf.get_string()?),
                METADATA_TYPE_BOOLEAN => EntityMetadataValue::Bool(buf.get_bool()?),
                _ => return Err(ProtocolError::UnexpectedEof),
            };
            entries.push((index, value));
        }
        let has_metadata = buf.get_bool()?;
        Ok(Self {
            entity_id,
            entries,
            has_metadata,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        for (index, value) in &self.entries {
            buf.put_u8(*index);
            match value {
                EntityMetadataValue::Byte(v) => {
                    buf.put_varint(METADATA_TYPE_BYTE);
                    buf.put_i8(i8::from_ne_bytes([*v]));
                }
                EntityMetadataValue::VarInt(v) => {
                    buf.put_varint(METADATA_TYPE_VARINT);
                    buf.put_varint(*v);
                }
                EntityMetadataValue::Float(v) => {
                    buf.put_varint(METADATA_TYPE_FLOAT);
                    buf.put_f32(*v);
                }
                EntityMetadataValue::String(v) => {
                    buf.put_varint(METADATA_TYPE_STRING);
                    buf.put_string(v);
                }
                EntityMetadataValue::Bool(v) => {
                    buf.put_varint(METADATA_TYPE_BOOLEAN);
                    buf.put_bool(*v);
                }
            }
        }
        buf.put_u8(METADATA_TERMINATOR);
        buf.put_bool(self.has_metadata);
        Ok(())
    }
}

/// 销毁实体（clientbound, id 0x4b，wire 名 `destroy_entities`）。
///
/// 通知客户端移除指定实体。线格式：`entity_ids` 数组（VarInt 长度 +
/// VarInt id × N）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestroyEntities {
    /// 待销毁的实体 id 列表。
    pub entity_ids: Vec<i32>,
}

impl Packet for DestroyEntities {
    fn packet_id(&self) -> i32 {
        0x4b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_ids = read_varint_array(buf, |b| b.get_varint())?;
        Ok(Self { entity_ids })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.entity_ids, |b, id| {
            b.put_varint(*id);
            Ok(())
        })
    }
}

// ============================ 实体同步（T17，clientbound） ============================
// 线格式语义权威为 vendored Java `packet/server/play/`（1.21.11，协议 774），
// 任务清单中的字段猜测与其冲突处以 vendored 为准，差异见各处文档注释。

/// 实体动画（clientbound, id 0x02，wire 名 `entity_animation`）。
///
/// 线格式：`entity_id`(VarInt) + `animation`(UByte)。vendored Java 以 VarInt 编码
/// 枚举（`NetworkBuffer.Enum`），对合法取值 0..=5 与单字节编码字节序完全一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityAnimation {
    /// 动画实体 id。
    pub entity_id: i32,
    /// 动画类型：0=挥主手、1=受击、2=离开床、3=挥副手、4=暴击粒子、5=魔法暴击粒子。
    pub animation: u8,
}

impl Packet for EntityAnimation {
    fn packet_id(&self) -> i32 {
        0x02
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityAnimation {
            entity_id: buf.get_varint()?,
            animation: buf.get_u8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_u8(self.animation);
        Ok(())
    }
}

/// 受击动画（clientbound, id 0x29，wire 名 `hit_animation`）。
///
/// 线格式为 `entity_id`(VarInt) + `yaw`(Float)，与 vendored `HitAnimationPacket`
/// 一致（任务清单误猜为「entity_id2 + damage(i16)」）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitAnimation {
    /// 受击实体 id。
    pub entity_id: i32,
    /// 受击朝向（弧度）。
    pub yaw: f32,
}

impl Packet for HitAnimation {
    fn packet_id(&self) -> i32 {
        0x29
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(HitAnimation {
            entity_id: buf.get_varint()?,
            yaw: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_f32(self.yaw);
        Ok(())
    }
}

/// 实体状态（clientbound, id 0x22，wire 名 `entity_status`）。
///
/// 线格式：`entity_id`(Int 固定 4 字节) + `status`(Byte)，与 vendored `EntityStatusPacket`
/// 及 1.21.x 协议一致（任务清单误猜为 VarInt）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityStatus {
    /// 实体 id（线格式为固定 Int）。
    pub entity_id: i32,
    /// 实体状态值（见协议「Entity statuses」表）。
    pub status: i8,
}

impl Packet for EntityStatus {
    fn packet_id(&self) -> i32 {
        0x22
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityStatus {
            entity_id: buf.get_i32()?,
            status: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i32(self.entity_id);
        buf.put_i8(self.status);
        Ok(())
    }
}

/// 实体位置同步（clientbound, id 0x23，wire 名 `entity_position_sync`）。
///
/// 1.21.6+ 新增包：绝对坐标 + 位移增量 + 朝向。线格式为
/// `entity_id`(VarInt) + `position`(3×Double) + `delta`(3×Double) +
/// `yaw`/`pitch`(Float) + `on_ground`(Bool)，与 vendored `EntityPositionSyncPacket`
/// 一致（任务清单误猜为「i32 协议坐标」）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityPositionSync {
    pub entity_id: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// 位移增量（客户端用于插值）。
    pub d_x: f64,
    pub d_y: f64,
    pub d_z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
}

impl Packet for EntityPositionSync {
    fn packet_id(&self) -> i32 {
        0x23
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityPositionSync {
            entity_id: buf.get_varint()?,
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            d_x: buf.get_f64()?,
            d_y: buf.get_f64()?,
            d_z: buf.get_f64()?,
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_f64(self.d_x);
        buf.put_f64(self.d_y);
        buf.put_f64(self.d_z);
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 实体相对移动（clientbound, id 0x33，wire 名 `entity_position`）。
///
/// 与既有 [`RelEntityMove`] 同构（均为 3×Short 定点增量）；两者分别由不同的
/// 高层同步路径使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityPosition {
    pub entity_id: i32,
    pub d_x: i16,
    pub d_y: i16,
    pub d_z: i16,
    pub on_ground: bool,
}

impl Packet for EntityPosition {
    fn packet_id(&self) -> i32 {
        0x33
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityPosition {
            entity_id: buf.get_varint()?,
            d_x: buf.get_i16()?,
            d_y: buf.get_i16()?,
            d_z: buf.get_i16()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_i16(self.d_x);
        buf.put_i16(self.d_y);
        buf.put_i16(self.d_z);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 实体相对移动 + 旋转（clientbound, id 0x34，wire 名 `entity_position_and_rotation`）。
///
/// `yaw`/`pitch` 为 256 分度制有符号字节（Angle），与既有 [`EntityTeleport`] 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityPositionAndRotation {
    pub entity_id: i32,
    pub d_x: i16,
    pub d_y: i16,
    pub d_z: i16,
    pub yaw: i8,
    pub pitch: i8,
    pub on_ground: bool,
}

impl Packet for EntityPositionAndRotation {
    fn packet_id(&self) -> i32 {
        0x34
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityPositionAndRotation {
            entity_id: buf.get_varint()?,
            d_x: buf.get_i16()?,
            d_y: buf.get_i16()?,
            d_z: buf.get_i16()?,
            yaw: buf.get_i8()?,
            pitch: buf.get_i8()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_i16(self.d_x);
        buf.put_i16(self.d_y);
        buf.put_i16(self.d_z);
        buf.put_i8(self.yaw);
        buf.put_i8(self.pitch);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

/// 实体旋转（clientbound, id 0x36，wire 名 `entity_rotation`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityRotation {
    pub entity_id: i32,
    pub yaw: i8,
    pub pitch: i8,
    pub on_ground: bool,
}

impl Packet for EntityRotation {
    fn packet_id(&self) -> i32 {
        0x36
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityRotation {
            entity_id: buf.get_varint()?,
            yaw: buf.get_i8()?,
            pitch: buf.get_i8()?,
            on_ground: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_i8(self.yaw);
        buf.put_i8(self.pitch);
        buf.put_bool(self.on_ground);
        Ok(())
    }
}

// ---- LP 打包向量（1.21.9+ velocity 线格式，vendored `LP_VECTOR3`）----

/// LP 打包向量各分量可表示的最大绝对值（超出钳制）。
const LP_VECTOR3_ABS_MAX: f64 = 1.717_986_918_3e10;
/// 低于该绝对值视为零向量（单字节 0x00）。
const LP_VECTOR3_ABS_MIN: f64 = 3.051_944_088_384_301e-5;
/// 分量量化上限（[-1, 1] 映射到 15 位无符号）。
const LP_VECTOR3_MAX_QUANTIZED: f64 = 32_766.0;
/// scale 低 2 位掩码。
const LP_VECTOR3_SCALE_BITS: i64 = 0b11;
/// 是否有 continuation（scale > 3）的标志位。
const LP_VECTOR3_CONTINUATION: i64 = 0b100;
/// X 分量在 48 位打包值中的偏移。
const LP_VECTOR3_X_OFFSET: i64 = 3;
/// Y 分量偏移。
const LP_VECTOR3_Y_OFFSET: i64 = 18;
/// Z 分量偏移。
const LP_VECTOR3_Z_OFFSET: i64 = 33;

/// 写出 LP 打包向量（1.21.9+ `entity_velocity` 等 velocity 字段的线格式）。
///
/// 语义与 vendored `LpVector3Type` 对齐：零向量写单字节 `0x00`；否则按
/// `scale = ceil(max 分量)` 归一化后各分量 15 位量化，打包为 48 位值
/// （`flags(2bit) | continuation(1bit) | px<<3 | py<<18 | pz<<33`），
/// 依次写 1+1+4 字节，scale > 3 时再补一个 VarInt（`scale >> 2`）。
fn write_lp_vector3(buf: &mut ByteBuffer, velocity: [f64; 3]) -> Result<(), ProtocolError> {
    let [x, y, z] = velocity;
    // NaN 按 0 处理；超出 ±ABS_MAX 钳制（与 Java `sanitize` 一致）。
    let x = if x.is_nan() {
        0.0
    } else {
        x.clamp(-LP_VECTOR3_ABS_MAX, LP_VECTOR3_ABS_MAX)
    };
    let y = if y.is_nan() {
        0.0
    } else {
        y.clamp(-LP_VECTOR3_ABS_MAX, LP_VECTOR3_ABS_MAX)
    };
    let z = if z.is_nan() {
        0.0
    } else {
        z.clamp(-LP_VECTOR3_ABS_MAX, LP_VECTOR3_ABS_MAX)
    };
    let max = x.abs().max(y.abs()).max(z.abs());
    if max < LP_VECTOR3_ABS_MIN {
        buf.put_u8(0);
        return Ok(());
    }
    // float→int 转换前已由 clamp 保证在 i64 精确表示范围内，`as i64` 无数据丢失。
    let scale = max.ceil() as i64;
    let has_continuation = (scale & LP_VECTOR3_SCALE_BITS) != scale;
    let flags = if has_continuation {
        (scale & LP_VECTOR3_SCALE_BITS) | LP_VECTOR3_CONTINUATION
    } else {
        scale
    };
    // 归一化分量 ∈ [-1, 1]，量化后 ∈ [0, 32766]，round 后 `as i64` 精确。
    let pack = |v: f64| ((v * 0.5 + 0.5) * LP_VECTOR3_MAX_QUANTIZED).round() as i64;
    let px = pack(x / scale as f64) << LP_VECTOR3_X_OFFSET;
    let py = pack(y / scale as f64) << LP_VECTOR3_Y_OFFSET;
    let pz = pack(z / scale as f64) << LP_VECTOR3_Z_OFFSET;
    let packed = flags | px | py | pz;
    // 低 8 位与次 8 位先掩码后收窄，无数据丢失。
    buf.put_u8((packed & 0xFF) as u8);
    buf.put_u8(((packed >> 8) & 0xFF) as u8);
    // 取 48 位打包值的高 32 位（bits 16..=47），与 Java `(int)(packed >> 16)` 一致。
    buf.put_i32((packed >> 16) as i32);
    if has_continuation {
        // scale >> 2 理论上可能超出 i32，先钳制再收窄（实际速度下 scale 极小，不影响）。
        let cont = (scale >> 2).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        buf.put_varint(cont as i32);
    }
    Ok(())
}

/// 读取 LP 打包向量（与 [`write_lp_vector3`] 对称）。
fn read_lp_vector3(buf: &mut ByteBuffer) -> Result<[f64; 3], ProtocolError> {
    let flags = buf.get_u8()?;
    if flags == 0 {
        return Ok([0.0; 3]);
    }
    let p2 = buf.get_u8()?;
    // 同尺寸按位重解释（i32 → u32 不缩窄），再提升为 i64。
    let p3 = u32::from_ne_bytes(buf.get_i32()?.to_ne_bytes()) as i64;
    let value = (p3 << 16) | ((i64::from(p2)) << 8) | i64::from(flags);
    let mut scale = i64::from(flags & 0b11);
    if flags & 0b100 != 0 {
        scale |= (i64::from(buf.get_varint()?) & 0xFFFF_FFFF) << 2;
    }
    // 15 位量化分量 → [-1, 1]。
    let unpack = |v: i64| {
        ((v & 0x7FFF).min(LP_VECTOR3_MAX_QUANTIZED as i64) as f64) * 2.0 / LP_VECTOR3_MAX_QUANTIZED
            - 1.0
    };
    Ok([
        unpack(value >> LP_VECTOR3_X_OFFSET) * scale as f64,
        unpack(value >> LP_VECTOR3_Y_OFFSET) * scale as f64,
        unpack(value >> LP_VECTOR3_Z_OFFSET) * scale as f64,
    ])
}

/// 实体速度（clientbound, id 0x63，wire 名 `entity_velocity`）。
///
/// 1.21.9+ 线格式为 `entity_id`(VarInt) + velocity(LP 打包向量，见
/// [`write_lp_vector3`])，与 vendored `EntityVelocityPacket` 一致（任务清单误猜为
/// 3×Short；Short 格式是 1.21.9 之前的旧格式）。`velocity` 单位为方块/刻。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityVelocity {
    pub entity_id: i32,
    /// 速度向量（方块/刻，按 1/8000 定点语义经 LP 格式量化传输）。
    pub velocity: [f64; 3],
}

impl Packet for EntityVelocity {
    fn packet_id(&self) -> i32 {
        0x63
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_id = buf.get_varint()?;
        let velocity = read_lp_vector3(buf)?;
        Ok(EntityVelocity {
            entity_id,
            velocity,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        write_lp_vector3(buf, self.velocity)
    }
}

/// 绑定实体（clientbound, id 0x62，wire 名 `attach_entity`）：拴绳 / 挂载。
///
/// 线格式两个实体 id 均为固定 Int（与 vendored `AttachEntityPacket` 及 1.21.x
/// 协议一致，任务清单误猜为 VarInt）；`holding_entity_id` 为 -1 表示解除。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachEntity {
    /// 被绑定的实体 id。
    pub attached_entity_id: i32,
    /// 拴绳另一端的实体 id（-1 解除）。
    pub holding_entity_id: i32,
}

impl Packet for AttachEntity {
    fn packet_id(&self) -> i32 {
        0x62
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(AttachEntity {
            attached_entity_id: buf.get_i32()?,
            holding_entity_id: buf.get_i32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i32(self.attached_entity_id);
        buf.put_i32(self.holding_entity_id);
        Ok(())
    }
}

/// 设置乘客（clientbound, id 0x69，wire 名 `set_passengers`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetPassengers {
    /// 载具实体 id。
    pub entity_id: i32,
    /// 乘客实体 id 列表（VarInt 数组）。
    pub passengers: Vec<i32>,
}

impl Packet for SetPassengers {
    fn packet_id(&self) -> i32 {
        0x69
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_id = buf.get_varint()?;
        let passengers = read_varint_array(buf, |b| b.get_varint())?;
        Ok(SetPassengers {
            entity_id,
            passengers,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        write_varint_array(buf, &self.passengers, |b, id| {
            b.put_varint(*id);
            Ok(())
        })
    }
}

/// 实体头部朝向（clientbound, id 0x51，wire 名 `entity_head_look`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityHeadLook {
    pub entity_id: i32,
    /// 头部偏航角（256 分度制有符号字节）。
    pub head_yaw: i8,
}

impl Packet for EntityHeadLook {
    fn packet_id(&self) -> i32 {
        0x51
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityHeadLook {
            entity_id: buf.get_varint()?,
            head_yaw: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_i8(self.head_yaw);
        Ok(())
    }
}

/// 拾取物品（clientbound, id 0x7a，wire 名 `collect_item`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectItem {
    /// 被拾取的物品实体 id。
    pub collected_entity_id: i32,
    /// 拾取者（玩家）实体 id。
    pub collector_entity_id: i32,
    /// 拾取数量。
    pub pickup_count: i32,
}

impl Packet for CollectItem {
    fn packet_id(&self) -> i32 {
        0x7a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(CollectItem {
            collected_entity_id: buf.get_varint()?,
            collector_entity_id: buf.get_varint()?,
            pickup_count: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.collected_entity_id);
        buf.put_varint(self.collector_entity_id);
        buf.put_varint(self.pickup_count);
        Ok(())
    }
}

/// 移除实体效果（clientbound, id 0x4c，wire 名 `remove_entity_effect`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoveEntityEffect {
    pub entity_id: i32,
    /// 药水效果 id（注册表序号）。
    pub effect_id: i32,
}

impl Packet for RemoveEntityEffect {
    fn packet_id(&self) -> i32 {
        0x4c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(RemoveEntityEffect {
            entity_id: buf.get_varint()?,
            effect_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_varint(self.effect_id);
        Ok(())
    }
}

/// 实体效果（clientbound, id 0x82，wire 名 `entity_effect`）。
///
/// 线格式为 `entity_id`(VarInt) + `effect_id`(VarInt) + `amplifier`(VarInt) +
/// `duration`(VarInt) + `flags`(Byte)，与 vendored `EntityEffectPacket`（Potion 网络
/// 类型）及 1.21.x 协议一致（任务清单误猜 amplifier 为 Byte）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityEffect {
    pub entity_id: i32,
    /// 药水效果 id。
    pub effect_id: i32,
    /// 放大器（线格式 VarInt；客户端显示为 amplifier + 1 级）。
    pub amplifier: i32,
    /// 持续时间（秒）。
    pub duration: i32,
    /// 效果标志位（bit0 隐藏粒子、bit1 隐藏环境音效、bit2 显示图标）。
    pub flags: u8,
}

impl Packet for EntityEffect {
    fn packet_id(&self) -> i32 {
        0x82
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntityEffect {
            entity_id: buf.get_varint()?,
            effect_id: buf.get_varint()?,
            amplifier: buf.get_varint()?,
            duration: buf.get_varint()?,
            flags: buf.get_u8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_varint(self.effect_id);
        buf.put_varint(self.amplifier);
        buf.put_varint(self.duration);
        buf.put_u8(self.flags);
        Ok(())
    }
}

/// 属性修饰器（[`EntityAttributes`] 子结构）。
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeModifier {
    /// 修饰器 id（命名空间键，如 `minecraft:some_modifier`）。
    pub modifier_id: String,
    /// 修饰量。
    pub amount: f64,
    /// 操作：0=加值、1=加乘基底、2=加乘总和（线格式 VarInt）。
    pub operation: i32,
}

/// 属性条目（[`EntityAttributes`] 子结构）。
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeProperty {
    /// 属性注册表 id（1.21.9+ 线格式为 VarInt 注册表序号，任务清单误猜为 String 键）。
    pub attribute_id: i32,
    /// 属性基值。
    pub value: f64,
    /// 修饰器列表（VarInt 数组）。
    pub modifiers: Vec<AttributeModifier>,
}

/// 实体属性（clientbound, id 0x81，wire 名 `entity_update_attributes`）。
///
/// 线格式：`entity_id`(VarInt) + `properties`(VarInt 数组，每个：
/// `attribute_id`(VarInt) + `value`(Double) + `modifiers`(VarInt 数组，每个：
/// `modifier_id`(String) + `amount`(Double) + `operation`(VarInt)))，与 vendored
/// `EntityAttributesPacket` 一致（任务清单误猜为「key(String)/uuid(UUID)/operation(Byte)」）。
#[derive(Debug, Clone, PartialEq)]
pub struct EntityAttributes {
    pub entity_id: i32,
    pub properties: Vec<AttributeProperty>,
}

impl Packet for EntityAttributes {
    fn packet_id(&self) -> i32 {
        0x81
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_id = buf.get_varint()?;
        let properties = read_varint_array(buf, |b| {
            let attribute_id = b.get_varint()?;
            let value = b.get_f64()?;
            let modifiers = read_varint_array(b, |b| {
                let modifier_id = b.get_string()?;
                let amount = b.get_f64()?;
                let operation = b.get_varint()?;
                Ok(AttributeModifier {
                    modifier_id,
                    amount,
                    operation,
                })
            })?;
            Ok(AttributeProperty {
                attribute_id,
                value,
                modifiers,
            })
        })?;
        Ok(EntityAttributes {
            entity_id,
            properties,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        write_varint_array(buf, &self.properties, |b, p| {
            b.put_varint(p.attribute_id);
            b.put_f64(p.value);
            write_varint_array(b, &p.modifiers, |b, m| {
                b.put_string(&m.modifier_id);
                b.put_f64(m.amount);
                b.put_varint(m.operation);
                Ok(())
            })
        })
    }
}

/// 设置经验（clientbound, id 0x65，wire 名 `set_experience`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetExperience {
    /// 经验条进度（0.0..=1.0）。
    pub experience_bar: f32,
    /// 等级。
    pub level: i32,
    /// 总经验值。
    pub total_experience: i32,
}

impl Packet for SetExperience {
    fn packet_id(&self) -> i32 {
        0x65
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetExperience {
            experience_bar: buf.get_f32()?,
            level: buf.get_varint()?,
            total_experience: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.experience_bar);
        buf.put_varint(self.level);
        buf.put_varint(self.total_experience);
        Ok(())
    }
}

/// 玩家背包单槽更新（clientbound, id 0x6a，wire 名 `set_player_inventory_slot`）。
///
/// 与通用 [`SetSlotPacket`](0x14) 不同，此包只针对玩家自身背包且无窗口/状态 id：
/// 线格式为 `slot`(VarInt) + `item`(ItemStack)，与 vendored `SetPlayerInventorySlotPacket`
/// 一致（任务清单误猜为「window_id + state_id + slot(Short)」）。
#[derive(Debug, Clone, PartialEq)]
pub struct SetPlayerInventorySlot {
    /// 背包槽位（0..=45，其中 36..=45 为快捷栏）。
    pub slot: i32,
    /// 槽位物品。
    pub item: ItemStack,
}

impl Packet for SetPlayerInventorySlot {
    fn packet_id(&self) -> i32 {
        0x6a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetPlayerInventorySlot {
            slot: buf.get_varint()?,
            item: decode_item_stack(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.slot);
        encode_item_stack(&self.item, buf)
    }
}

/// 伤害事件（clientbound, id 0x19，wire 名 `damage_event`）。
///
/// 线格式：`target_entity_id`(VarInt) + `damage_type_id`(VarInt) +
/// `source_cause_id`(VarInt) + `source_direct_id`(VarInt) +
/// `source_position`(可选：Bool 存在位 + 3×Double)，与 vendored `DamageEventPacket`
/// 一致。`source_cause_id`/`source_direct_id` 为 0 表示无来源；实体来源以 id + 1
/// 编码。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageEvent {
    /// 受伤实体 id。
    pub target_entity_id: i32,
    /// 伤害类型 id（注册表 `minecraft:damage_type` 序号）。
    pub damage_type_id: i32,
    /// 伤害原因实体 id（0 表示无；否则实体 id + 1）。
    pub source_cause_id: i32,
    /// 直接来源实体 id（弹射物等；0 表示无；否则实体 id + 1）。
    pub source_direct_id: i32,
    /// 来源位置（无则 `None`；有则为 (x, y, z)）。
    pub source_position: Option<(f64, f64, f64)>,
}

impl Packet for DamageEvent {
    fn packet_id(&self) -> i32 {
        0x19
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let target_entity_id = buf.get_varint()?;
        let damage_type_id = buf.get_varint()?;
        let source_cause_id = buf.get_varint()?;
        let source_direct_id = buf.get_varint()?;
        let source_position = if buf.get_bool()? {
            Some((buf.get_f64()?, buf.get_f64()?, buf.get_f64()?))
        } else {
            None
        };
        Ok(DamageEvent {
            target_entity_id,
            damage_type_id,
            source_cause_id,
            source_direct_id,
            source_position,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.target_entity_id);
        buf.put_varint(self.damage_type_id);
        buf.put_varint(self.source_cause_id);
        buf.put_varint(self.source_direct_id);
        match self.source_position {
            Some((x, y, z)) => {
                buf.put_bool(true);
                buf.put_f64(x);
                buf.put_f64(y);
                buf.put_f64(z);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

// ============================ 世界/区块/音效（T18，clientbound） ============================

/// 方块更新（clientbound, id 0x08，wire 名 `block_change`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockChange {
    /// 方块坐标（打包 Position）。
    pub block_position: (i32, i32, i32),
    /// 方块状态 id。
    pub block_state: i32,
}

impl Packet for BlockChange {
    fn packet_id(&self) -> i32 {
        0x08
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(BlockChange {
            block_position: buf.get_position()?,
            block_state: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_varint(self.block_state);
        Ok(())
    }
}

/// 多方块更新（clientbound, id 0x52，wire 名 `multi_block_change`）。
///
/// 1.21.5+ 线格式：`chunk_section_position`(Long) + `blocks`(VarInt 数组，每个为
/// 打包 Long：`block_state << 12 | local_x << 8 | local_z << 4 | local_y`，其中
/// local_x/local_y/local_z 为区块段内 0..=15 坐标)，与 vendored
/// `MultiBlockChangePacket`/`CoordConversion.encodeSectionBlockChange` 一致（任务清单
/// 误猜为「Short+Byte+VarInt」三字段记录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiBlockChange {
    /// 区块段位置（打包：chunk_x<<42 | section<<0 | chunk_z<<20）。
    pub chunk_section_position: i64,
    /// 变更记录（每个为打包 Long，见结构体文档）。
    pub blocks: Vec<i64>,
}

impl Packet for MultiBlockChange {
    fn packet_id(&self) -> i32 {
        0x52
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(MultiBlockChange {
            chunk_section_position: buf.get_i64()?,
            blocks: read_varint_array(buf, |b| b.get_i64())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.chunk_section_position);
        write_varint_array(buf, &self.blocks, |b, v| {
            b.put_i64(*v);
            Ok(())
        })
    }
}

/// 方块变更确认（clientbound, id 0x04，wire 名 `acknowledge_block_change`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcknowledgeBlockChange {
    /// 与玩家方块放置请求关联的序列号。
    pub sequence_id: i32,
}

impl Packet for AcknowledgeBlockChange {
    fn packet_id(&self) -> i32 {
        0x04
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(AcknowledgeBlockChange {
            sequence_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.sequence_id);
        Ok(())
    }
}

/// 方块动作（clientbound, id 0x07，wire 名 `block_action`）：活塞 / 门 / 音符盒等。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockAction {
    pub block_position: (i32, i32, i32),
    /// 动作 id（按方块类型解释）。
    pub action_id: i8,
    /// 动作参数（按方块类型解释）。
    pub action_param: i8,
    /// 方块类型 id（用于客户端选择动作含义）。
    pub block_type: i32,
}

impl Packet for BlockAction {
    fn packet_id(&self) -> i32 {
        0x07
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(BlockAction {
            block_position: buf.get_position()?,
            action_id: buf.get_i8()?,
            action_param: buf.get_i8()?,
            block_type: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_i8(self.action_id);
        buf.put_i8(self.action_param);
        buf.put_varint(self.block_type);
        Ok(())
    }
}

/// 方块实体数据（clientbound, id 0x06，wire 名 `block_entity_data`）。
///
/// **简化**：`nbt_data` 以原始字节透传，encode 直接追加、decode 读取 NBT 之后的全部
/// 剩余字节（NBT 是最后一个字段，anonymous Compound 自定界）。后续接入完整 NBT
/// 解析后可替换为结构化字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEntityData {
    pub block_position: (i32, i32, i32),
    /// 方块实体类型 id。
    pub block_entity_type: i32,
    /// anonymous NBT 原始字节（简化实现，未解析）。
    pub nbt_data: Vec<u8>,
}

impl Packet for BlockEntityData {
    fn packet_id(&self) -> i32 {
        0x06
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(BlockEntityData {
            block_position: buf.get_position()?,
            block_entity_type: buf.get_varint()?,
            nbt_data: buf.get_bytes(buf.remaining())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_varint(self.block_entity_type);
        buf.put_bytes(&self.nbt_data);
        Ok(())
    }
}

/// 方块破坏动画（clientbound, id 0x05，wire 名 `block_break_animation`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockBreakAnimation {
    /// 挖掘者实体 id。
    pub entity_id: i32,
    pub block_position: (i32, i32, i32),
    /// 破坏阶段（0..=9；255 表示取消）。
    pub destroy_stage: i8,
}

impl Packet for BlockBreakAnimation {
    fn packet_id(&self) -> i32 {
        0x05
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(BlockBreakAnimation {
            entity_id: buf.get_varint()?,
            block_position: buf.get_position()?,
            destroy_stage: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_i8(self.destroy_stage);
        Ok(())
    }
}

/// 光照更新（clientbound, id 0x2f，wire 名 `update_light`）。
///
/// 与 [`MapChunk`] 的光照区段同构：四个掩码为 VarInt 数组（Long 元素），两组光照为
/// VarInt 数组（每个是 VarInt 长度 + 原始字节）。vendored 以 BitSet 表示掩码，
/// 其线格式即 Long 数组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateLight {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub sky_light_mask: Vec<i64>,
    pub block_light_mask: Vec<i64>,
    pub empty_sky_light_mask: Vec<i64>,
    pub empty_block_light_mask: Vec<i64>,
    pub sky_light: Vec<Vec<u8>>,
    pub block_light: Vec<Vec<u8>>,
}

impl Packet for UpdateLight {
    fn packet_id(&self) -> i32 {
        0x2f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateLight {
            chunk_x: buf.get_varint()?,
            chunk_z: buf.get_varint()?,
            sky_light_mask: read_varint_array(buf, |b| b.get_i64())?,
            block_light_mask: read_varint_array(buf, |b| b.get_i64())?,
            empty_sky_light_mask: read_varint_array(buf, |b| b.get_i64())?,
            empty_block_light_mask: read_varint_array(buf, |b| b.get_i64())?,
            sky_light: read_varint_array(buf, read_byte_array)?,
            block_light: read_varint_array(buf, read_byte_array)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.chunk_x);
        buf.put_varint(self.chunk_z);
        write_varint_array(buf, &self.sky_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.block_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.empty_sky_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.empty_block_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.sky_light, |b, arr| write_byte_array(b, arr))?;
        write_varint_array(buf, &self.block_light, |b, arr| write_byte_array(b, arr))?;
        Ok(())
    }
}

/// 单个区块的生物群系数据（[`ChunkBiomes`] 子结构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBiomeData {
    pub chunk_x: i32,
    pub chunk_z: i32,
    /// 群系数据（压缩后的 3D 群系调色板数组，简化以原始字节透传）。
    pub data: Vec<u8>,
}

/// 区块生物群系（clientbound, id 0x0d，wire 名 `chunk_biomes`）。
///
/// 线格式为 `chunks`(VarInt 数组)，每个条目为 `chunk_z`(Int) + `chunk_x`(Int) +
/// `data`(ByteArray)；vendored `ChunkBiomeData` 序列化器先写 chunk_z 再写 chunk_x
/// （注释「x and z are inverted, not a bug」），此处按 vendored 线序。`data` 按
/// 任务清单简化直接透传字节数组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBiomes {
    pub chunks: Vec<ChunkBiomeData>,
}

impl Packet for ChunkBiomes {
    fn packet_id(&self) -> i32 {
        0x0d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let chunks = read_varint_array(buf, |b| {
            let chunk_z = b.get_i32()?;
            let chunk_x = b.get_i32()?;
            let data = read_byte_array(b)?;
            Ok(ChunkBiomeData {
                chunk_x,
                chunk_z,
                data,
            })
        })?;
        Ok(ChunkBiomes { chunks })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.chunks, |b, c| {
            // 线序：chunk_z 在前（vendored 反转约定）。
            b.put_i32(c.chunk_z);
            b.put_i32(c.chunk_x);
            write_byte_array(b, &c.data)
        })
    }
}

/// 卸载区块（clientbound, id 0x25，wire 名 `unload_chunk`）。
///
/// 线序为 `chunk_z`(Int) + `chunk_x`(Int)：客户端将 8 字节整体读作大端 Long 后
/// 再拆分（vendored `UnloadChunkPacket` 注释「we have to write it backwards」），
/// 故坐标先 z 后 x，与任务清单的「x/z(VarInt)」不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnloadChunk {
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl Packet for UnloadChunk {
    fn packet_id(&self) -> i32 {
        0x25
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let chunk_z = buf.get_i32()?;
        let chunk_x = buf.get_i32()?;
        Ok(UnloadChunk { chunk_x, chunk_z })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        // 线序先 z 后 x，见结构体文档。
        buf.put_i32(self.chunk_z);
        buf.put_i32(self.chunk_x);
        Ok(())
    }
}

/// 世界事件（clientbound, id 0x2d，wire 名 `world_event`）：音效 / 粒子 / 活塞等。
///
/// 线格式为 `event`(Int) + `block_position`(Position) + `data`(Int) +
/// `disable_relative_volume`(Bool)，与 vendored `WorldEventPacket` 一致（任务清单
/// 误猜 event/data 为 VarInt）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldEvent {
    /// 事件 id。
    pub event: i32,
    pub block_position: (i32, i32, i32),
    /// 事件数据（按事件 id 解释）。
    pub data: i32,
    /// 是否禁用相对音量。
    pub disable_relative_volume: bool,
}

impl Packet for WorldEvent {
    fn packet_id(&self) -> i32 {
        0x2d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WorldEvent {
            event: buf.get_i32()?,
            block_position: buf.get_position()?,
            data: buf.get_i32()?,
            disable_relative_volume: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i32(self.event);
        let (x, y, z) = self.block_position;
        buf.put_position(x, y, z);
        buf.put_i32(self.data);
        buf.put_bool(self.disable_relative_volume);
        Ok(())
    }
}

/// 粒子（clientbound, id 0x2e，wire 名 `particle`）。
///
/// 线格式按任务清单（与 vanilla 一致）：`particle_id`(VarInt) +
/// `long_distance`(Bool) + `x/y/z`(Double×3) + `offset_x/y/z`(Float×3) +
/// `max_speed`(Float) + `particle_count`(VarInt)。**简化**：粒子数据
/// （`particle_id` 之后的按类型字段）置空不编解码，且未包含 vendored 的
/// `override_limiter` 首字段，见 Open Issues。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    /// 粒子类型 id（注册表 `minecraft:particle_type` 序号）。
    pub particle_id: i32,
    /// 远距离渲染（忽略距离衰减）。
    pub long_distance: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub offset_x: f32,
    pub offset_y: f32,
    pub offset_z: f32,
    /// 最大速度。
    pub max_speed: f32,
    /// 粒子数量。
    pub particle_count: i32,
}

impl Packet for Particle {
    fn packet_id(&self) -> i32 {
        0x2e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Particle {
            particle_id: buf.get_varint()?,
            long_distance: buf.get_bool()?,
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            offset_x: buf.get_f32()?,
            offset_y: buf.get_f32()?,
            offset_z: buf.get_f32()?,
            max_speed: buf.get_f32()?,
            particle_count: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.particle_id);
        buf.put_bool(self.long_distance);
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_f32(self.offset_x);
        buf.put_f32(self.offset_y);
        buf.put_f32(self.offset_z);
        buf.put_f32(self.max_speed);
        buf.put_varint(self.particle_count);
        Ok(())
    }
}

/// 爆炸记录（[`Explosion`] 子结构）：区块段内局部坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplosionRecord {
    /// 局部坐标打包：高 4 位 x、低 4 位 z（区块段内 0..=15）。
    pub xz: i8,
    /// 局部 y（区块段内 0..=15）。
    pub y: i8,
}

/// 爆炸（clientbound, id 0x24，wire 名 `explosion`）。
///
/// **简化**：按任务清单实现经典线格式 `x/y/z`(Float×3) + `strength`(Float) +
/// `records`(VarInt 数组，每个 `xz`(Byte)+`y`(Byte)) + `player_motion_x/y/z`(Float×3)。
/// vendored 1.21.11 的 `ExplosionPacket` 已改为新格式（Double 中心、粒子 id、声音、
/// 加权方块粒子），本框架暂不实现，见 Open Issues。
#[derive(Debug, Clone, PartialEq)]
pub struct Explosion {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 爆炸强度（半径）。
    pub strength: f32,
    /// 被破坏方块记录。
    pub records: Vec<ExplosionRecord>,
    /// 玩家被爆炸推动的速度。
    pub player_motion_x: f32,
    pub player_motion_y: f32,
    pub player_motion_z: f32,
}

impl Packet for Explosion {
    fn packet_id(&self) -> i32 {
        0x24
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Explosion {
            x: buf.get_f32()?,
            y: buf.get_f32()?,
            z: buf.get_f32()?,
            strength: buf.get_f32()?,
            records: read_varint_array(buf, |b| {
                Ok(ExplosionRecord {
                    xz: b.get_i8()?,
                    y: b.get_i8()?,
                })
            })?,
            player_motion_x: buf.get_f32()?,
            player_motion_y: buf.get_f32()?,
            player_motion_z: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.x);
        buf.put_f32(self.y);
        buf.put_f32(self.z);
        buf.put_f32(self.strength);
        write_varint_array(buf, &self.records, |b, r| {
            b.put_i8(r.xz);
            b.put_i8(r.y);
            Ok(())
        })?;
        buf.put_f32(self.player_motion_x);
        buf.put_f32(self.player_motion_y);
        buf.put_f32(self.player_motion_z);
        Ok(())
    }
}

/// 声音效果（clientbound, id 0x73，wire 名 `sound_effect`）。
///
/// 线格式：`sound_id`(VarInt) + `sound_category`(VarInt) + `x/y/z`(Int，1/8 方块
/// 定点) + `volume`(Float) + `pitch`(Float) + `seed`(Long)。**简化**：vendored 的
/// `SoundEvent.NETWORK_TYPE` 支持自定义声音（VarInt 0 + 名称 + 可选范围），本实现
/// 仅透传内置声音的注册表 id + 1 值，见 Open Issues。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundEffect {
    /// 声音 id（线格式值 = 注册表 id + 1；0 表示自定义声音，本简化不支持）。
    pub sound_id: i32,
    /// 声音分类（0=大师、1=音乐、2=记录、3=天气、4=方块、5=敌对、6=中立、7=玩家、8=环境、9=语音）。
    pub sound_category: i32,
    /// 声音坐标 x（1/8 方块定点）。
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub volume: f32,
    pub pitch: f32,
    /// 随机种子（客户端用于偏移播放）。
    pub seed: i64,
}

impl Packet for SoundEffect {
    fn packet_id(&self) -> i32 {
        0x73
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SoundEffect {
            sound_id: buf.get_varint()?,
            sound_category: buf.get_varint()?,
            x: buf.get_i32()?,
            y: buf.get_i32()?,
            z: buf.get_i32()?,
            volume: buf.get_f32()?,
            pitch: buf.get_f32()?,
            seed: buf.get_i64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.sound_id);
        buf.put_varint(self.sound_category);
        buf.put_i32(self.x);
        buf.put_i32(self.y);
        buf.put_i32(self.z);
        buf.put_f32(self.volume);
        buf.put_f32(self.pitch);
        buf.put_i64(self.seed);
        Ok(())
    }
}

/// 实体声音效果（clientbound, id 0x72，wire 名 `entity_sound_effect`）。
///
/// 与 [`SoundEffect`] 同构，但以 `entity_id`(VarInt) 代替坐标。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntitySoundEffect {
    /// 声音 id（线格式值 = 注册表 id + 1）。
    pub sound_id: i32,
    /// 声音分类。
    pub sound_category: i32,
    /// 发声实体 id。
    pub entity_id: i32,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
}

impl Packet for EntitySoundEffect {
    fn packet_id(&self) -> i32 {
        0x72
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EntitySoundEffect {
            sound_id: buf.get_varint()?,
            sound_category: buf.get_varint()?,
            entity_id: buf.get_varint()?,
            volume: buf.get_f32()?,
            pitch: buf.get_f32()?,
            seed: buf.get_i64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.sound_id);
        buf.put_varint(self.sound_category);
        buf.put_varint(self.entity_id);
        buf.put_f32(self.volume);
        buf.put_f32(self.pitch);
        buf.put_i64(self.seed);
        Ok(())
    }
}

/// 停止声音（clientbound, id 0x75，wire 名 `stop_sound`）。
///
/// 线格式：`flags`(Byte) + [可选 `source`(VarInt)，flags bit0 置位时] +
/// [可选 `sound`(String)，flags bit1 置位时]，与 vendored `StopSoundPacket` 一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopSound {
    /// 位标志：bit0=含 source、bit1=含 sound。
    pub flags: u8,
    /// 声音分类（flags bit0 置位时存在）。
    pub source: Option<i32>,
    /// 声音 id（flags bit1 置位时存在）。
    pub sound: Option<String>,
}

impl Packet for StopSound {
    fn packet_id(&self) -> i32 {
        0x75
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let flags = buf.get_u8()?;
        let source = if flags & 0x01 != 0 {
            Some(buf.get_varint()?)
        } else {
            None
        };
        let sound = if flags & 0x02 != 0 {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(StopSound {
            flags,
            source,
            sound,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_u8(self.flags);
        if self.flags & 0x01 != 0 {
            let source = self.source.ok_or(ProtocolError::UnexpectedEof)?;
            buf.put_varint(source);
        }
        if self.flags & 0x02 != 0 {
            let sound = self.sound.as_deref().ok_or(ProtocolError::UnexpectedEof)?;
            buf.put_string(sound);
        }
        Ok(())
    }
}

/// 单个标签组（[`TagsRegistry`] 子结构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    /// 标签名（如 `minecraft:planks`）。
    pub name: String,
    /// 条目 id（注册表序号）。
    pub entries: Vec<i32>,
}

/// 单个注册表（[`Tags`] 子结构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagsRegistry {
    /// 注册表名（如 `minecraft:block`）。
    pub registry: String,
    /// 标签组列表。
    pub tags: Vec<TagEntry>,
}

/// 标签（clientbound, id 0x84，wire 名 `tags`）。
///
/// 线格式为 `registries`(VarInt 数组)，每个：`registry`(String) + `tags`(VarInt
/// 数组)，每个标签：`name`(String) + `entries`(VarInt 数组)，与 vendored
/// `TagsPacket.Registry` 一致（任务清单简化建议略去 registry 层级，本实现保留）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tags {
    pub registries: Vec<TagsRegistry>,
}

impl Packet for Tags {
    fn packet_id(&self) -> i32 {
        0x84
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let registries = read_varint_array(buf, |b| {
            let registry = b.get_string()?;
            let tags = read_varint_array(b, |b| {
                let name = b.get_string()?;
                let entries = read_varint_array(b, |b| b.get_varint())?;
                Ok(TagEntry { name, entries })
            })?;
            Ok(TagsRegistry { registry, tags })
        })?;
        Ok(Tags { registries })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.registries, |b, r| {
            b.put_string(&r.registry);
            write_varint_array(b, &r.tags, |b, t| {
                b.put_string(&t.name);
                write_varint_array(b, &t.entries, |b, e| {
                    b.put_varint(*e);
                    Ok(())
                })
            })
        })
    }
}

/// 服务器数据（clientbound, id 0x54，wire 名 `server_data`）：MOTD 与图标。
///
/// 线格式：`motd`(TAG_COMPOUND 0x0a + anonymous NBT Compound，含 `text` 键) +
/// `icon`(可选 ByteArray) + `enforces_secure_chat`(Bool)。**简化**：`motd` 仅存
/// 纯文本，编码/解码复用 [`SystemChatPacket`] 的 NBT 方案（框架仅存文本，不保留
/// 富文本结构）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerData {
    /// MOTD 文本（线格式为 NBT Compound JSON，框架仅存文本）。
    pub motd: String,
    /// 服务器图标（base64 PNG 数据，可选）。
    pub icon: Option<Vec<u8>>,
    /// 是否强制安全聊天。
    pub enforces_secure_chat: bool,
}

impl Packet for ServerData {
    fn packet_id(&self) -> i32 {
        0x54
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        // 前导 TAG_COMPOUND（0x0a），与 encode 的 encode_anonymous 互补。
        let tag_id = buf.get_u8()?;
        if tag_id != 0x0a {
            return Err(ProtocolError::UnexpectedEof);
        }
        let rest = buf.get_bytes(buf.remaining())?;
        let (tag, consumed) =
            nbt::decode_anonymous(&rest).map_err(|_| ProtocolError::UnexpectedEof)?;
        let motd = match tag {
            NbtTag::Compound(entries) => entries
                .into_iter()
                .find(|(k, _)| k == "text")
                .and_then(|(_, v)| {
                    if let NbtTag::String(s) = v {
                        Some(s)
                    } else {
                        None
                    }
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
        // NBT 消费 consumed 字节后的剩余部分为 icon + enforces_secure_chat。
        let tail = rest.get(consumed..).ok_or(ProtocolError::UnexpectedEof)?;
        let mut tail_buf = ByteBuffer::new(tail.to_vec());
        let icon = if tail_buf.get_bool()? {
            Some(read_byte_array(&mut tail_buf)?)
        } else {
            None
        };
        let enforces_secure_chat = tail_buf.get_bool()?;
        Ok(ServerData {
            motd,
            icon,
            enforces_secure_chat,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let compound = NbtTag::Compound(vec![(
            "text".to_string(),
            NbtTag::String(self.motd.clone()),
        )]);
        let nbt = nbt::encode_anonymous(&compound).map_err(|_| ProtocolError::UnexpectedEof)?;
        buf.put_u8(0x0a);
        buf.put_bytes(&nbt);
        match &self.icon {
            Some(bytes) => {
                buf.put_bool(true);
                write_byte_array(buf, bytes)?;
            }
            None => buf.put_bool(false),
        }
        buf.put_bool(self.enforces_secure_chat);
        Ok(())
    }
}

/// 时间更新（clientbound, id 0x6f，wire 名 `time_update`）。
///
/// 线格式为 `world_age`(Long) + `time_of_day`(Long) + `tick_day_time`(Bool)，
/// 与 vendored `TimeUpdatePacket` 一致（任务清单只列了前两个 Long）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeUpdate {
    /// 世界年龄（刻）。
    pub world_age: i64,
    /// 时刻（刻，可带负号以固定时间）。
    pub time_of_day: i64,
    /// 是否推进昼夜循环。
    pub tick_day_time: bool,
}

impl Packet for TimeUpdate {
    fn packet_id(&self) -> i32 {
        0x6f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(TimeUpdate {
            world_age: buf.get_i64()?,
            time_of_day: buf.get_i64()?,
            tick_day_time: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.world_age);
        buf.put_i64(self.time_of_day);
        buf.put_bool(self.tick_day_time);
        Ok(())
    }
}

/// 设置刻状态（clientbound, id 0x7d，wire 名 `set_tick_state`）。
///
/// 线格式为 `tick_rate`(Float) + `is_frozen`(Bool)，与 vendored `SetTickStatePacket`
/// 一致（任务清单误猜 tick_rate 为 VarInt）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetTickState {
    /// 每秒刻数。
    pub tick_rate: f32,
    /// 世界是否冻结。
    pub is_frozen: bool,
}

impl Packet for SetTickState {
    fn packet_id(&self) -> i32 {
        0x7d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetTickState {
            tick_rate: buf.get_f32()?,
            is_frozen: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.tick_rate);
        buf.put_bool(self.is_frozen);
        Ok(())
    }
}

/// 步进刻（clientbound, id 0x7e，wire 名 `tick_step`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickStep {
    /// 步进的刻数。
    pub tick_steps: i32,
}

impl Packet for TickStep {
    fn packet_id(&self) -> i32 {
        0x7e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(TickStep {
            tick_steps: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.tick_steps);
        Ok(())
    }
}

/// 更新模拟距离（clientbound, id 0x6d，wire 名 `update_simulation_distance`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateSimulationDistance {
    /// 模拟距离（区块数）。
    pub simulation_distance: i32,
}

impl Packet for UpdateSimulationDistance {
    fn packet_id(&self) -> i32 {
        0x6d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateSimulationDistance {
            simulation_distance: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.simulation_distance);
        Ok(())
    }
}

/// 更新视角位置（clientbound, id 0x5c，wire 名 `update_view_position`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateViewPosition {
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl Packet for UpdateViewPosition {
    fn packet_id(&self) -> i32 {
        0x5c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateViewPosition {
            chunk_x: buf.get_varint()?,
            chunk_z: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.chunk_x);
        buf.put_varint(self.chunk_z);
        Ok(())
    }
}

/// 更新视距（clientbound, id 0x5d，wire 名 `update_view_distance`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateViewDistance {
    /// 渲染距离（区块数）。
    pub view_distance: i32,
}

impl Packet for UpdateViewDistance {
    fn packet_id(&self) -> i32 {
        0x5d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(UpdateViewDistance {
            view_distance: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.view_distance);
        Ok(())
    }
}

// ============================ 库存（clientbound） ============================
// 见 `.specs/implement-item-inventory/`（物品与物品栏任务规格）。

/// 玩家/容器库存全量内容（clientbound, id 0x12, wire 名 `container_set_content`）。
///
/// 发送窗口全部槽位 [`ItemStack`] 与光标携带物品。见 `.specs/implement-item-inventory/`。
#[derive(Debug, Clone, PartialEq)]
pub struct WindowItemsPacket {
    /// 窗口 id（0 = 玩家背包；其余为容器窗口）。
    pub window_id: i32,
    /// 状态 id（用于乐观锁，防止客户端/服务端状态漂移）。
    pub state_id: i32,
    /// 全部槽位物品（按窗口槽位顺序）。
    pub items: Vec<ItemStack>,
    /// 光标携带物品（鼠标拖拽中的物品）。
    pub carried_item: ItemStack,
}

impl Packet for WindowItemsPacket {
    fn packet_id(&self) -> i32 {
        0x12
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let window_id = buf.get_varint()?;
        let state_id = buf.get_varint()?;
        let n = buf.get_varint()?;
        // n 可能为负（非法协议数据），先夹到非负再转 usize；缩窄用 TryFrom。
        let n_usize = usize::try_from(n.max(0)).map_err(|_| ProtocolError::UnexpectedEof)?;
        let mut items = Vec::with_capacity(n_usize);
        for _ in 0..n_usize {
            items.push(decode_item_stack(buf)?);
        }
        let carried_item = decode_item_stack(buf)?;
        Ok(WindowItemsPacket {
            window_id,
            state_id,
            items,
            carried_item,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        // 槽位数 usize → i32 属潜在缩窄，用 TryFrom 保证溢出即报错。
        let count = i32::try_from(self.items.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
        buf.put_varint(self.window_id);
        buf.put_varint(self.state_id);
        buf.put_varint(count);
        for it in &self.items {
            encode_item_stack(it, buf)?;
        }
        encode_item_stack(&self.carried_item, buf)
    }
}

/// 单槽刷新（clientbound, id 0x14, wire 名 `set_slot`）。
///
/// 仅更新窗口中某个槽位，避免每次变动都重发全量内容。
#[derive(Debug, Clone, PartialEq)]
pub struct SetSlotPacket {
    /// 窗口 id（0 = 玩家背包）。
    pub window_id: i32,
    /// 状态 id（乐观锁）。
    pub state_id: i32,
    /// 槽位索引（玩家背包中 -106 等负索引表示特殊槽）。
    pub slot: i16,
    /// 该槽位的新物品。
    pub item: ItemStack,
}

impl Packet for SetSlotPacket {
    fn packet_id(&self) -> i32 {
        0x14
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let window_id = buf.get_varint()?;
        let state_id = buf.get_varint()?;
        let slot = buf.get_i16()?;
        let item = decode_item_stack(buf)?;
        Ok(SetSlotPacket {
            window_id,
            state_id,
            slot,
            item,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        buf.put_varint(self.state_id);
        buf.put_i16(self.slot);
        encode_item_stack(&self.item, buf)
    }
}

/// 装备条目（`EntityEquipmentPacket` 用）：槽位 + 物品。
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentEntry {
    /// 装备槽位（旧协议 id 映射）。
    pub slot: EquipmentSlot,
    /// 该槽位装备的物品。
    pub item: ItemStack,
}

/// 装备槽枚举（`EntityEquipmentPacket` 旧协议 id 映射）。
///
/// 1.21.11 仍沿用此旧 id 顺序；见 `.specs/implement-item-inventory/`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquipmentSlot {
    /// 主手。
    MainHand,
    /// 副手（左手）。
    OffHand,
    /// 靴子。
    Boots,
    /// 护腿。
    Leggings,
    /// 胸甲。
    Chestplate,
    /// 头盔。
    Helmet,
    /// 身体（马铠 / 披风等）。
    Body,
    /// 鞍。
    Saddle,
}

impl EquipmentSlot {
    /// 旧协议 id（`EntityEquipmentPacket` 用）。
    ///
    /// MAIN_HAND=0, OFF_HAND=1, BOOTS=2, LEGGINGS=3, CHESTPLATE=4, HELMET=5, BODY=6, SADDLE=7。
    pub fn legacy_id(self) -> u8 {
        match self {
            EquipmentSlot::MainHand => 0,
            EquipmentSlot::OffHand => 1,
            EquipmentSlot::Boots => 2,
            EquipmentSlot::Leggings => 3,
            EquipmentSlot::Chestplate => 4,
            EquipmentSlot::Helmet => 5,
            EquipmentSlot::Body => 6,
            EquipmentSlot::Saddle => 7,
        }
    }
    /// 由旧协议 id 解析（取低 7 位后传入）。非法 id 返回 `None`。
    pub fn from_legacy_id(id: u8) -> Option<EquipmentSlot> {
        match id {
            0 => Some(EquipmentSlot::MainHand),
            1 => Some(EquipmentSlot::OffHand),
            2 => Some(EquipmentSlot::Boots),
            3 => Some(EquipmentSlot::Leggings),
            4 => Some(EquipmentSlot::Chestplate),
            5 => Some(EquipmentSlot::Helmet),
            6 => Some(EquipmentSlot::Body),
            7 => Some(EquipmentSlot::Saddle),
            _ => None,
        }
    }
}

/// 装备槽（clientbound, id 0x64, wire 名 `set_equipment`）。
///
/// 可一次更新多个装备槽；除末项外，每个槽位字节最高位（0x80）置位表示后续还有条目。
#[derive(Debug, Clone, PartialEq)]
pub struct EntityEquipmentPacket {
    /// 实体 id。
    pub entity_id: i32,
    /// 装备条目列表（解码顺序即发送顺序）。
    pub equipments: Vec<EquipmentEntry>,
}

impl Packet for EntityEquipmentPacket {
    fn packet_id(&self) -> i32 {
        0x64
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_id = buf.get_varint()?;
        let mut equipments = Vec::new();
        loop {
            let b = buf.get_u8()?;
            let slot =
                EquipmentSlot::from_legacy_id(b & 0x7F).ok_or(ProtocolError::UnexpectedEof)?;
            let item = decode_item_stack(buf)?;
            equipments.push(EquipmentEntry { slot, item });
            if b & 0x80 == 0 {
                break;
            }
        }
        Ok(EntityEquipmentPacket {
            entity_id,
            equipments,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        let n = self.equipments.len();
        for (i, e) in self.equipments.iter().enumerate() {
            let last = i + 1 == n;
            let mut b = e.slot.legacy_id();
            if !last {
                b |= 0x80;
            }
            buf.put_u8(b);
            encode_item_stack(&e.item, buf)?;
        }
        Ok(())
    }
}

/// 区块批量开始（clientbound, id 0x0c，wire 名 `chunk_batch_start`），空包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkBatchStart;

impl Packet for ChunkBatchStart {
    fn packet_id(&self) -> i32 {
        0x0c
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ChunkBatchStart)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 区块批量完成（clientbound, id 0x0b，wire 名 `chunk_batch_finished`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBatchFinished {
    pub batch_size: i32,
}

impl Packet for ChunkBatchFinished {
    fn packet_id(&self) -> i32 {
        0x0b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ChunkBatchFinished {
            batch_size: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.batch_size);
        Ok(())
    }
}

/// 高度图条目：类型（varint mapper id）+ 每列一位的 packed 高度数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heightmap {
    pub map_type: i32,
    pub data: Vec<i64>,
}

/// 方块实体（chunk 内）：`xz` 高 4 位为局部 x、低 4 位为局部 z。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBlockEntity {
    pub xz: u8,
    pub y: i16,
    pub block_entity_type: i32,
    /// anonymous NBT 占位（原始字节）。解码暂不支持非空值，见 [`read_opt_nbt`]。
    pub nbt_data: Option<Vec<u8>>,
}

/// 区块数据（clientbound, id 0x2c，wire 名 `map_chunk`）。
///
/// 字段顺序严格对齐线格式：坐标 → 高度图 → 区块字节 → 方块实体 → 四组光照掩码
/// → 天空 / 方块光照。光照数组可为空（服务端尚未实现光照计算时的最小合法发送）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapChunk {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub heightmaps: Vec<Heightmap>,
    pub chunk_data: Vec<u8>,
    pub block_entities: Vec<ChunkBlockEntity>,
    pub sky_light_mask: Vec<i64>,
    pub block_light_mask: Vec<i64>,
    pub empty_sky_light_mask: Vec<i64>,
    pub empty_block_light_mask: Vec<i64>,
    pub sky_light: Vec<Vec<u8>>,
    pub block_light: Vec<Vec<u8>>,
}

impl Packet for MapChunk {
    fn packet_id(&self) -> i32 {
        0x2c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let chunk_x = buf.get_i32()?;
        let chunk_z = buf.get_i32()?;
        let heightmaps = read_varint_array(buf, |b| {
            let map_type = b.get_varint()?;
            let data = read_varint_array(b, |b| b.get_i64())?;
            Ok(Heightmap { map_type, data })
        })?;
        let chunk_data = read_byte_array(buf)?;
        let block_entities = read_varint_array(buf, |b| {
            let xz = b.get_u8()?;
            let y = b.get_i16()?;
            let block_entity_type = b.get_varint()?;
            let nbt_data = read_opt_nbt(b)?;
            Ok(ChunkBlockEntity {
                xz,
                y,
                block_entity_type,
                nbt_data,
            })
        })?;
        let sky_light_mask = read_varint_array(buf, |b| b.get_i64())?;
        let block_light_mask = read_varint_array(buf, |b| b.get_i64())?;
        let empty_sky_light_mask = read_varint_array(buf, |b| b.get_i64())?;
        let empty_block_light_mask = read_varint_array(buf, |b| b.get_i64())?;
        let sky_light = read_varint_array(buf, read_byte_array)?;
        let block_light = read_varint_array(buf, read_byte_array)?;
        Ok(MapChunk {
            chunk_x,
            chunk_z,
            heightmaps,
            chunk_data,
            block_entities,
            sky_light_mask,
            block_light_mask,
            empty_sky_light_mask,
            empty_block_light_mask,
            sky_light,
            block_light,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i32(self.chunk_x);
        buf.put_i32(self.chunk_z);
        write_varint_array(buf, &self.heightmaps, |b, h| {
            b.put_varint(h.map_type);
            write_varint_array(b, &h.data, |b, v| {
                b.put_i64(*v);
                Ok(())
            })
        })?;
        write_byte_array(buf, &self.chunk_data)?;
        write_varint_array(buf, &self.block_entities, |b, e| {
            b.put_u8(e.xz);
            b.put_i16(e.y);
            b.put_varint(e.block_entity_type);
            write_opt_nbt(b, &e.nbt_data);
            Ok(())
        })?;
        write_varint_array(buf, &self.sky_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.block_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.empty_sky_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.empty_block_light_mask, |b, v| {
            b.put_i64(*v);
            Ok(())
        })?;
        write_varint_array(buf, &self.sky_light, |b, arr| write_byte_array(b, arr))?;
        write_varint_array(buf, &self.block_light, |b, arr| write_byte_array(b, arr))?;
        Ok(())
    }
}

/// 死亡锚点（`GlobalPos`）：维度名 + 打包方块坐标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalPos {
    pub dimension: String,
    pub position: i64,
}

/// 登录世界状态（`SpawnInfo`），`Login` 的子结构。
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnInfo {
    /// 维度类型 id（注册表序号）。
    pub dimension: i32,
    /// 世界名。
    pub name: String,
    pub hashed_seed: i64,
    /// 游戏模式 id（0=生存，1=创造，2=冒险，3=旁观）。
    pub gamemode: i8,
    /// 上一游戏模式（255 表示无）。
    pub previous_gamemode: u8,
    pub is_debug: bool,
    pub is_flat: bool,
    /// 死亡地点（无则 `None`）。
    pub death: Option<GlobalPos>,
    pub portal_cooldown: i32,
    pub sea_level: i32,
}

/// 登录（clientbound, id 0x30，wire 名 `login`）：进入 Play 前必须发送。
#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    pub entity_id: i32,
    pub is_hardcore: bool,
    pub world_names: Vec<String>,
    pub max_players: i32,
    pub view_distance: i32,
    pub simulation_distance: i32,
    pub reduced_debug_info: bool,
    pub enable_respawn_screen: bool,
    pub do_limited_crafting: bool,
    pub world_state: SpawnInfo,
    pub enforces_secure_chat: bool,
}

impl Packet for Login {
    fn packet_id(&self) -> i32 {
        0x30
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_id = buf.get_i32()?;
        let is_hardcore = buf.get_bool()?;
        let world_names = read_varint_array(buf, |b| b.get_string())?;
        let max_players = buf.get_varint()?;
        let view_distance = buf.get_varint()?;
        let simulation_distance = buf.get_varint()?;
        let reduced_debug_info = buf.get_bool()?;
        let enable_respawn_screen = buf.get_bool()?;
        let do_limited_crafting = buf.get_bool()?;
        let world_state = decode_spawn_info(buf)?;
        let enforces_secure_chat = buf.get_bool()?;
        Ok(Login {
            entity_id,
            is_hardcore,
            world_names,
            max_players,
            view_distance,
            simulation_distance,
            reduced_debug_info,
            enable_respawn_screen,
            do_limited_crafting,
            world_state,
            enforces_secure_chat,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i32(self.entity_id);
        buf.put_bool(self.is_hardcore);
        write_varint_array(buf, &self.world_names, |b, s| {
            b.put_string(s);
            Ok(())
        })?;
        buf.put_varint(self.max_players);
        buf.put_varint(self.view_distance);
        buf.put_varint(self.simulation_distance);
        buf.put_bool(self.reduced_debug_info);
        buf.put_bool(self.enable_respawn_screen);
        buf.put_bool(self.do_limited_crafting);
        encode_spawn_info(buf, &self.world_state)?;
        buf.put_bool(self.enforces_secure_chat);
        Ok(())
    }
}

fn decode_spawn_info(buf: &mut ByteBuffer) -> Result<SpawnInfo, ProtocolError> {
    let dimension = buf.get_varint()?;
    let name = buf.get_string()?;
    let hashed_seed = buf.get_i64()?;
    let gamemode = buf.get_i8()?;
    let previous_gamemode = buf.get_u8()?;
    let is_debug = buf.get_bool()?;
    let is_flat = buf.get_bool()?;
    let death = if buf.get_bool()? {
        Some(GlobalPos {
            dimension: buf.get_string()?,
            position: buf.get_i64()?,
        })
    } else {
        None
    };
    let portal_cooldown = buf.get_varint()?;
    let sea_level = buf.get_varint()?;
    Ok(SpawnInfo {
        dimension,
        name,
        hashed_seed,
        gamemode,
        previous_gamemode,
        is_debug,
        is_flat,
        death,
        portal_cooldown,
        sea_level,
    })
}

fn encode_spawn_info(buf: &mut ByteBuffer, info: &SpawnInfo) -> Result<(), ProtocolError> {
    buf.put_varint(info.dimension);
    buf.put_string(&info.name);
    buf.put_i64(info.hashed_seed);
    buf.put_i8(info.gamemode);
    buf.put_u8(info.previous_gamemode);
    buf.put_bool(info.is_debug);
    buf.put_bool(info.is_flat);
    match &info.death {
        Some(gp) => {
            buf.put_bool(true);
            buf.put_string(&gp.dimension);
            buf.put_i64(gp.position);
        }
        None => buf.put_bool(false),
    }
    buf.put_varint(info.portal_cooldown);
    buf.put_varint(info.sea_level);
    Ok(())
}

// ============================ 界面/杂项（T19，clientbound） ============================

/// 数据包束（clientbound, id 0x00，wire 名 `bundle_delimiter`）。
///
/// 1.21.11 真实线格式为空包（仅作束边界，束内子包以 VarInt 长度前缀包裹在流中顺序出现）。
/// 本框架按任务清单**简化**为 `packets: Vec<Vec<u8>>`（VarInt 计数 + 每项 VarInt 长度 +
/// 完整包字节，即 packet_id + 包体），自洽往返但非真实协议格式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// 束内完整包字节（packet_id + 包体）。
    pub packets: Vec<Vec<u8>>,
}

impl Packet for Bundle {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let packets = read_varint_array(buf, |b| {
            let len = b.get_varint()?;
            let len_usize = usize::try_from(len).map_err(|_| ProtocolError::UnexpectedEof)?;
            b.get_bytes(len_usize)
        })?;
        Ok(Bundle { packets })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.packets, |b, pkt| {
            let len = i32::try_from(pkt.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
            b.put_varint(len);
            b.put_bytes(pkt);
            Ok(())
        })
    }
}

/// 统计信息（clientbound, id 0x03，wire 名 `statistics`）。
///
/// 每项为 `category_id`(VarInt) + `statistic_id`(VarInt) + `value`(VarInt) 三元组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statistics {
    /// 统计条目列表（类别 id, 统计 id, 数值）。
    pub entries: Vec<(i32, i32, i32)>,
}

impl Packet for Statistics {
    fn packet_id(&self) -> i32 {
        0x03
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entries = read_varint_array(buf, |b| {
            Ok((b.get_varint()?, b.get_varint()?, b.get_varint()?))
        })?;
        Ok(Statistics { entries })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.entries, |b, e| {
            b.put_varint(e.0);
            b.put_varint(e.1);
            b.put_varint(e.2);
            Ok(())
        })
    }
}

/// 清除标题（clientbound, id 0x0e，wire 名 `clear_titles`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClearTitles {
    /// 是否同时重置淡入淡出计时。
    pub reset: bool,
}

impl Packet for ClearTitles {
    fn packet_id(&self) -> i32 {
        0x0e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClearTitles {
            reset: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_bool(self.reset);
        Ok(())
    }
}

/// 命令补全响应（clientbound, id 0x0f，wire 名 `tab_complete`）。
///
/// 与 serverbound `TabComplete`(0x0e) 同名冲突，故命名为 `ClientboundTabComplete`。
/// 每项匹配为 `match`(String) + `has_tooltip`(Bool) + `tooltip`(Option<String>)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientboundTabComplete {
    /// 事务 id（VarInt）。
    pub transaction_id: i32,
    /// 匹配起始偏移（VarInt）。
    pub start: i32,
    /// 匹配长度（VarInt）。
    pub length: i32,
    /// 匹配项列表。
    pub matches: Vec<(String, bool, Option<String>)>,
}

impl Packet for ClientboundTabComplete {
    fn packet_id(&self) -> i32 {
        0x0f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let transaction_id = buf.get_varint()?;
        let start = buf.get_varint()?;
        let length = buf.get_varint()?;
        let matches = read_varint_array(buf, |b| {
            let matched = b.get_string()?;
            let has_tooltip = b.get_bool()?;
            let tooltip = if has_tooltip {
                Some(b.get_string()?)
            } else {
                None
            };
            Ok((matched, has_tooltip, tooltip))
        })?;
        Ok(ClientboundTabComplete {
            transaction_id,
            start,
            length,
            matches,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.transaction_id);
        buf.put_varint(self.start);
        buf.put_varint(self.length);
        write_varint_array(buf, &self.matches, |b, m| {
            b.put_string(&m.0);
            match &m.2 {
                Some(tip) => {
                    b.put_bool(true);
                    b.put_string(tip);
                }
                None => b.put_bool(false),
            }
            Ok(())
        })
    }
}

/// 命令节点（`DeclareCommands` 的树节点，对齐 Java `DeclareCommandsPacket.Node`）。
///
/// `flags` 位布局（对齐 Java `NODE_TYPE`/`IS_EXECUTABLE`/`HAS_REDIRECT`/
/// `HAS_SUGGESTION_TYPE`）：
///
/// - `0x03` 类型掩码：0=`ROOT`，1=`LITERAL`，2=`ARGUMENT`；
/// - `0x04` executable（可执行节点）；
/// - `0x08` redirect：随后写 `redirect` 字段；
/// - `0x10` suggestion：随后写 `suggestions_type` 字段。
///
/// 线格式：`flags`(Byte) + `children`(VarInt 数组) + [`redirect`](Self::redirect)
/// (VarInt，仅 0x08) + [`name`](Self::name)(String，仅类型位非零) +
/// [`parser`](Self::parser)(VarInt) 与 [`properties`](Self::properties)(原始字节，
/// 仅类型位含 0x02) + [`suggestions_type`](Self::suggestions_type)(String，仅 0x10)。
///
/// `properties` 为解析器专属参数的**原始字节**（不透明承载，roundtrip 无损）：
/// `DOUBLE`/`INTEGER`/`FLOAT`/`LONG` 为 min/max 位掩码（flags 字节 0x01=min、
/// 0x02=max，各值按对应宽度大端）+ 值；`STRING` 为 VarInt；`ENTITY`/`SCORE_HOLDER`
/// 为 1 字节；`TIME` 为 Int（4 字节）；`RESOURCE_OR_TAG`/`RESOURCE_OR_TAG_KEY`/
/// `RESOURCE`/`RESOURCE_KEY` 为 String；其余类型为空。未知 parser id 在解码时按
/// [`ProtocolError::InvalidValue`] 拒绝（不 panic）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandNode {
    /// 节点 flags（Byte）。
    pub flags: i8,
    /// 子节点索引（VarInt 数组）。
    pub children: Vec<i32>,
    /// 重定向目标节点索引（仅 flags 0x08 位）。
    pub redirect: Option<i32>,
    /// 节点名（仅 LITERAL/ARGUMENT）。
    pub name: Option<String>,
    /// 参数解析器（仅 ARGUMENT；flags 置位但解析器缺失 / 未知 id 为畸形数据）。
    pub parser: Option<ArgumentParserType>,
    /// 解析器专属参数原始字节（仅 ARGUMENT；未知解析器类型为空）。
    pub properties: Vec<u8>,
    /// 补全类型标识（仅 flags 0x10 位）。
    pub suggestions_type: Option<String>,
}

/// 按解析器类型提取 `properties` 原始字节（对齐 Java `getProperties` 的
/// `extractBytes` 语义：消费解析器专属参数并原样切片，不重编码）。
fn extract_parser_properties(
    buf: &mut ByteBuffer,
    parser: ArgumentParserType,
) -> Result<Vec<u8>, ProtocolError> {
    let start = buf.position();
    match parser {
        ArgumentParserType::DOUBLE
        | ArgumentParserType::INTEGER
        | ArgumentParserType::FLOAT
        | ArgumentParserType::LONG => {
            let flags = buf.get_u8()?;
            if flags & 0x01 != 0 {
                consume_typed_value(buf, parser)?;
            }
            if flags & 0x02 != 0 {
                consume_typed_value(buf, parser)?;
            }
        }
        ArgumentParserType::STRING => {
            buf.get_varint()?;
        }
        ArgumentParserType::ENTITY | ArgumentParserType::SCORE_HOLDER => {
            buf.get_u8()?;
        }
        ArgumentParserType::TIME => {
            buf.get_i32()?;
        }
        ArgumentParserType::RESOURCE_OR_TAG
        | ArgumentParserType::RESOURCE_OR_TAG_KEY
        | ArgumentParserType::RESOURCE
        | ArgumentParserType::RESOURCE_KEY => {
            buf.get_string()?;
        }
        _ => {}
    }
    let end = buf.position();
    let raw = buf
        .as_slice()
        .get(start..end)
        .ok_or(ProtocolError::UnexpectedEof)?;
    Ok(raw.to_vec())
}

/// 消费一个对应宽度的数值参数（min/max 位掩码后的单个值）。
fn consume_typed_value(
    buf: &mut ByteBuffer,
    parser: ArgumentParserType,
) -> Result<(), ProtocolError> {
    match parser {
        ArgumentParserType::DOUBLE => {
            buf.get_f64()?;
        }
        ArgumentParserType::FLOAT => {
            buf.get_f32()?;
        }
        ArgumentParserType::LONG => {
            buf.get_i64()?;
        }
        _ => {
            buf.get_i32()?;
        }
    }
    Ok(())
}

impl CommandNode {
    /// 是否声明重定向（flags 0x08 位）。
    pub fn has_redirect(&self) -> bool {
        self.flags & 0x08 != 0
    }

    /// 是否声明补全类型（flags 0x10 位）。
    pub fn has_suggestion_type(&self) -> bool {
        self.flags & 0x10 != 0
    }

    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.flags);
        write_varint_array(buf, &self.children, |b, c| {
            b.put_varint(*c);
            Ok(())
        })?;
        if self.has_redirect() {
            let redirect = self.redirect.ok_or(ProtocolError::InvalidValue)?;
            buf.put_varint(redirect);
        }
        let type_bits = self.flags & 0x03;
        if type_bits != 0 {
            buf.put_string(self.name.as_deref().unwrap_or(""));
        }
        if type_bits & 0x02 != 0 {
            let parser = self.parser.ok_or(ProtocolError::InvalidValue)?;
            buf.put_varint(parser.id());
            buf.put_bytes(&self.properties);
        }
        if self.has_suggestion_type() {
            buf.put_string(self.suggestions_type.as_deref().unwrap_or(""));
        }
        Ok(())
    }

    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let flags = buf.get_i8()?;
        let children = read_varint_array(buf, |b| b.get_varint())?;
        let redirect = if flags & 0x08 != 0 {
            Some(buf.get_varint()?)
        } else {
            None
        };
        let type_bits = flags & 0x03;
        let name = if type_bits != 0 {
            Some(buf.get_string()?)
        } else {
            None
        };
        let (parser, properties) = if type_bits & 0x02 != 0 {
            let id = buf.get_varint()?;
            let parser = ArgumentParserType::from_id(id).ok_or(ProtocolError::InvalidValue)?;
            let properties = extract_parser_properties(buf, parser)?;
            (Some(parser), properties)
        } else {
            (None, Vec::new())
        };
        let suggestions_type = if flags & 0x10 != 0 {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(CommandNode {
            flags,
            children,
            redirect,
            name,
            parser,
            properties,
            suggestions_type,
        })
    }
}

/// 声明命令（clientbound, id 0x10，wire 名 `commands`）。
///
/// 1.21.11 真实线格式：`nodes`（VarInt 计数，每条为 [`CommandNode`]）+
/// `root_index`(VarInt)。命令树可承载 ROOT/LITERAL/ARGUMENT 节点，含
/// executable / redirect / suggestion 位与解析器专属参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclareCommands {
    /// 命令树节点列表。
    pub nodes: Vec<CommandNode>,
    /// 根节点索引（VarInt）。
    pub root_index: i32,
}

impl Packet for DeclareCommands {
    fn packet_id(&self) -> i32 {
        0x10
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let nodes = read_varint_array(buf, CommandNode::decode)?;
        let root_index = buf.get_varint()?;
        Ok(DeclareCommands { nodes, root_index })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.nodes, |b, n| n.encode(b))?;
        buf.put_varint(self.root_index);
        Ok(())
    }
}

/// 关闭窗口（clientbound, id 0x11，wire 名 `close_window`）。
///
/// 任务清单：`window_id`(Byte)。注意 Java 实际以 VarInt 编码窗口 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseWindow {
    /// 窗口 id（Byte）。
    pub window_id: i8,
}

impl Packet for CloseWindow {
    fn packet_id(&self) -> i32 {
        0x11
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(CloseWindow {
            window_id: buf.get_i8()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.window_id);
        Ok(())
    }
}

/// 窗口属性（clientbound, id 0x13，wire 名 `window_property`）。
///
/// 任务清单：`window_id`(Byte)+`property`(Short)+`value`(Short)。Java 实际以 VarInt
/// 编码窗口 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowProperty {
    /// 窗口 id（Byte）。
    pub window_id: i8,
    /// 属性 id（Short）。
    pub property: i16,
    /// 属性值（Short）。
    pub value: i16,
}

impl Packet for WindowProperty {
    fn packet_id(&self) -> i32 {
        0x13
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WindowProperty {
            window_id: buf.get_i8()?,
            property: buf.get_i16()?,
            value: buf.get_i16()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.window_id);
        buf.put_i16(self.property);
        buf.put_i16(self.value);
        Ok(())
    }
}

/// Cookie 请求（clientbound, id 0x15，wire 名 `cookie_request`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieRequest {
    /// Cookie 键（String）。
    pub key: String,
}

impl Packet for CookieRequest {
    fn packet_id(&self) -> i32 {
        0x15
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(CookieRequest {
            key: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.key);
        Ok(())
    }
}

/// 设置冷却（clientbound, id 0x16，wire 名 `set_cooldown`）。
///
/// 1.21.11 真实线格式（对齐 Java `SetCooldownPacket`）：
/// `cooldown_group`(String)+`cooldown_ticks`(VarInt)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCooldown {
    /// 冷却组标识（String，如 `minecraft:shield`）。
    pub cooldown_group: String,
    /// 冷却刻数（VarInt）。
    pub cooldown_ticks: i32,
}

impl Packet for SetCooldown {
    fn packet_id(&self) -> i32 {
        0x16
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetCooldown {
            cooldown_group: buf.get_string()?,
            cooldown_ticks: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.cooldown_group);
        buf.put_varint(self.cooldown_ticks);
        Ok(())
    }
}

/// 自定义聊天补全（clientbound, id 0x17，wire 名 `custom_chat_completion`）。
///
/// `action`(VarInt)：0=ADD，1=REMOVE，2=SET（任务清单简化）。`entries` 为 VarInt 计数的
/// 字符串数组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomChatCompletion {
    /// 动作（VarInt）。
    pub action: i32,
    /// 补全条目（String 数组）。
    pub entries: Vec<String>,
}

impl Packet for CustomChatCompletion {
    fn packet_id(&self) -> i32 {
        0x17
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(CustomChatCompletion {
            action: buf.get_varint()?,
            entries: read_varint_array(buf, |b| b.get_string())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.action);
        write_varint_array(buf, &self.entries, |b, s| {
            b.put_string(s);
            Ok(())
        })
    }
}

/// 插件消息（clientbound, id 0x18，wire 名 `plugin_message`）。
///
/// 与 `configuration::PluginMessage`（mod.rs 已导出）同名冲突，故命名为
/// `ClientboundPluginMessage`。`data` 为通道后剩余全部字节（RAW_BYTES）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientboundPluginMessage {
    /// 通道标识（String）。
    pub channel: String,
    /// 消息体字节（RAW_BYTES，剩余全部）。
    pub data: Vec<u8>,
}

impl Packet for ClientboundPluginMessage {
    fn packet_id(&self) -> i32 {
        0x18
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientboundPluginMessage {
            channel: buf.get_string()?,
            data: buf.get_bytes(buf.remaining())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.channel);
        buf.put_bytes(&self.data);
        Ok(())
    }
}

/// 调试方块值（clientbound, id 0x1a，wire 名 `debug_block_value`）。
///
/// 任务清单**简化**为 `payload: Vec<u8>` 原始字节（VarInt 长度 + 字节），不解析实际
/// 的 BLOCK_POSITION + Update 结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugBlockValue {
    /// 原始负载字节（简化）。
    pub payload: Vec<u8>,
}

impl Packet for DebugBlockValue {
    fn packet_id(&self) -> i32 {
        0x1a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DebugBlockValue {
            payload: read_byte_array(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_byte_array(buf, &self.payload)
    }
}

/// 调试区块值（clientbound, id 0x1b，wire 名 `debug_chunk_value`）。
///
/// 任务清单**简化**为 `payload: Vec<u8>` 原始字节（同 [`DebugBlockValue`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugChunkValue {
    /// 原始负载字节（简化）。
    pub payload: Vec<u8>,
}

impl Packet for DebugChunkValue {
    fn packet_id(&self) -> i32 {
        0x1b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DebugChunkValue {
            payload: read_byte_array(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_byte_array(buf, &self.payload)
    }
}

/// 调试实体值（clientbound, id 0x1c，wire 名 `debug_entity_value`）。
///
/// 任务清单**简化**为 `payload: Vec<u8>` 原始字节（同 [`DebugBlockValue`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugEntityValue {
    /// 原始负载字节（简化）。
    pub payload: Vec<u8>,
}

impl Packet for DebugEntityValue {
    fn packet_id(&self) -> i32 {
        0x1c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DebugEntityValue {
            payload: read_byte_array(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_byte_array(buf, &self.payload)
    }
}

/// 调试事件（clientbound, id 0x1d，wire 名 `debug_event`）。
///
/// 任务清单**简化**：`event`(VarInt)+`payload: Vec<u8>` 原始字节。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugEvent {
    /// 事件类型（VarInt）。
    pub event: i32,
    /// 事件负载字节（简化）。
    pub payload: Vec<u8>,
}

impl Packet for DebugEvent {
    fn packet_id(&self) -> i32 {
        0x1d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DebugEvent {
            event: buf.get_varint()?,
            payload: read_byte_array(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.event);
        write_byte_array(buf, &self.payload)
    }
}

/// 调试采样（clientbound, id 0x1e，wire 名 `debug_sample`）。
///
/// `sample_type`(VarInt)+`data`(VarInt 计数 + Long×N)。任务清单将 Java 的
/// `LONG_ARRAY + Enum(type)` 序调整为 sample_type 在前。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugSample {
    /// 采样类型（VarInt）。
    pub sample_type: i32,
    /// 采样数据（Long 数组）。
    pub data: Vec<i64>,
}

impl Packet for DebugSample {
    fn packet_id(&self) -> i32 {
        0x1e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DebugSample {
            sample_type: buf.get_varint()?,
            data: read_varint_array(buf, |b| b.get_i64())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.sample_type);
        write_varint_array(buf, &self.data, |b, v| {
            b.put_i64(*v);
            Ok(())
        })
    }
}

/// 删除聊天消息（clientbound, id 0x1f，wire 名 `delete_chat`）。
///
/// 任务清单：`message_id`(VarInt)。Java 实际承载 `MessageSignature`
/// （VarInt 长度 + 签名字节），本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteChat {
    /// 消息 id（VarInt）。
    pub message_id: i32,
}

impl Packet for DeleteChat {
    fn packet_id(&self) -> i32 {
        0x1f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DeleteChat {
            message_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.message_id);
        Ok(())
    }
}

/// 断开连接（clientbound, id 0x20，wire 名 `disconnect`）。
///
/// 任务清单**简化**：`reason` 以 String 承载（真实线格式为 NBT JSON 组件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disconnect {
    /// 断开原因文本（简化 String，真实为 NBT JSON）。
    pub reason: String,
}

impl Packet for Disconnect {
    fn packet_id(&self) -> i32 {
        0x20
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Disconnect {
            reason: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.reason);
        Ok(())
    }
}

/// 伪装聊天（clientbound, id 0x21，wire 名 `disguised_chat`）。
///
/// `message` / `sender_name` / `target_name` 均**简化**为 String（真实为 NBT JSON 组件）；
/// `target_name` 以 Bool 存在位前缀。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisguisedChat {
    /// 消息文本（简化 String）。
    pub message: String,
    /// 聊天类型（VarInt）。
    pub chat_type: i32,
    /// 发送者名（简化 String）。
    pub sender_name: String,
    /// 目标名（Option，Bool 存在位 + String）。
    pub target_name: Option<String>,
}

impl Packet for DisguisedChat {
    fn packet_id(&self) -> i32 {
        0x21
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let message = buf.get_string()?;
        let chat_type = buf.get_varint()?;
        let sender_name = buf.get_string()?;
        let target_name = if buf.get_bool()? {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(DisguisedChat {
            message,
            chat_type,
            sender_name,
            target_name,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.message);
        buf.put_varint(self.chat_type);
        buf.put_string(&self.sender_name);
        match &self.target_name {
            Some(t) => {
                buf.put_bool(true);
                buf.put_string(t);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 游戏测试高亮坐标（clientbound, id 0x27，wire 名 `game_test_highlight_pos`）。
///
/// 任务清单：`position`(Position 协议坐标)+`solid`(Bool)+`red/green/blue/alpha`(VarInt)。
/// Java 实际为 absolute + relative 两个 BLOCK_POSITION，本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameTestHighlightPos {
    /// 方块坐标（Position 打包）。
    pub position: (i32, i32, i32),
    /// 是否为实心（Bool）。
    pub solid: bool,
    /// 红色分量（VarInt）。
    pub red: i32,
    /// 绿色分量（VarInt）。
    pub green: i32,
    /// 蓝色分量（VarInt）。
    pub blue: i32,
    /// 透明度（VarInt）。
    pub alpha: i32,
}

impl Packet for GameTestHighlightPos {
    fn packet_id(&self) -> i32 {
        0x27
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(GameTestHighlightPos {
            position: buf.get_position()?,
            solid: buf.get_bool()?,
            red: buf.get_varint()?,
            green: buf.get_varint()?,
            blue: buf.get_varint()?,
            alpha: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.position;
        buf.put_position(x, y, z);
        buf.put_bool(self.solid);
        buf.put_varint(self.red);
        buf.put_varint(self.green);
        buf.put_varint(self.blue);
        buf.put_varint(self.alpha);
        Ok(())
    }
}

/// 打开马匹窗口（clientbound, id 0x28，wire 名 `open_horse_window`）。
///
/// 任务清单：`window_id`(Byte)+`slot_count`(VarInt)+`entity_id`(VarInt)。Java 实际以
/// VarInt 编码窗口 id。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenHorseWindow {
    /// 窗口 id（Byte）。
    pub window_id: i8,
    /// 槽位数量（VarInt）。
    pub slot_count: i32,
    /// 实体 id（VarInt）。
    pub entity_id: i32,
}

impl Packet for OpenHorseWindow {
    fn packet_id(&self) -> i32 {
        0x28
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(OpenHorseWindow {
            window_id: buf.get_i8()?,
            slot_count: buf.get_varint()?,
            entity_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.window_id);
        buf.put_varint(self.slot_count);
        buf.put_varint(self.entity_id);
        Ok(())
    }
}

/// 初始化世界边界（clientbound, id 0x2a，wire 名 `world_border_initialize`）。
///
/// `speed`(VarLong)。注意：Java 实际字段序为
/// portal_teleport_boundary → warning_time → warning_blocks，任务清单为
/// warning_blocks → warning_time，本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InitializeWorldBorder {
    /// 中心 x（Double）。
    pub x: f64,
    /// 中心 z（Double）。
    pub z: f64,
    /// 旧直径（Double）。
    pub old_diameter: f64,
    /// 新直径（Double）。
    pub new_diameter: f64,
    /// 变化速度（VarLong）。
    pub speed: i64,
    /// 传送边界（VarInt）。
    pub portal_teleport_boundary: i32,
    /// 警告方块数（VarInt）。
    pub warning_blocks: i32,
    /// 警告时长（VarInt）。
    pub warning_time: i32,
}

impl Packet for InitializeWorldBorder {
    fn packet_id(&self) -> i32 {
        0x2a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(InitializeWorldBorder {
            x: buf.get_f64()?,
            z: buf.get_f64()?,
            old_diameter: buf.get_f64()?,
            new_diameter: buf.get_f64()?,
            speed: buf.get_varlong()?,
            portal_teleport_boundary: buf.get_varint()?,
            warning_blocks: buf.get_varint()?,
            warning_time: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.x);
        buf.put_f64(self.z);
        buf.put_f64(self.old_diameter);
        buf.put_f64(self.new_diameter);
        buf.put_varlong(self.speed);
        buf.put_varint(self.portal_teleport_boundary);
        buf.put_varint(self.warning_blocks);
        buf.put_varint(self.warning_time);
        Ok(())
    }
}

/// 心跳（clientbound, id 0x2b，wire 名 `keep_alive`）。
///
/// 与 serverbound `KeepAlive`(0x1b) 同名冲突，故命名为 `ClientboundKeepAlive`。
/// 任务清单：`keep_alive_id`(VarLong)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientboundKeepAlive {
    /// 心跳 id（VarLong）。
    pub keep_alive_id: i64,
}

impl Packet for ClientboundKeepAlive {
    fn packet_id(&self) -> i32 {
        0x2b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientboundKeepAlive {
            keep_alive_id: buf.get_varlong()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varlong(self.keep_alive_id);
        Ok(())
    }
}

/// 地图数据（clientbound, id 0x31，wire 名 `map_data`）。
///
/// 任务清单**简化**：`icons` 恒为空数组（仅写 VarInt 0），`data` 为 VarInt 长度的
/// 字节数组。Java 实际按 `tracking_position` 条件性写 icons，并按颜色内容条件性写
/// columns/rows/x/z/data。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapData {
    /// 地图 id（VarInt）。
    pub map_id: i32,
    /// 缩放级别（Byte）。
    pub scale: i8,
    /// 是否锁定（Bool）。
    pub locked: bool,
    /// 列数（Byte）。
    pub columns: i8,
    /// 行数（Byte）。
    pub rows: i8,
    /// 像素数据（VarInt 长度 + Byte 数组）。
    pub data: Vec<u8>,
}

impl Packet for MapData {
    fn packet_id(&self) -> i32 {
        0x31
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let map_id = buf.get_varint()?;
        let scale = buf.get_i8()?;
        let locked = buf.get_bool()?;
        // 简化：icons 恒为空数组，仅消费计数。
        let _icons_count = buf.get_varint()?;
        let columns = buf.get_i8()?;
        let rows = buf.get_i8()?;
        let data = read_byte_array(buf)?;
        Ok(MapData {
            map_id,
            scale,
            locked,
            columns,
            rows,
            data,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.map_id);
        buf.put_i8(self.scale);
        buf.put_bool(self.locked);
        // 简化：icons 恒为空数组。
        buf.put_varint(0);
        buf.put_i8(self.columns);
        buf.put_i8(self.rows);
        write_byte_array(buf, &self.data)
    }
}

/// 交易列表（clientbound, id 0x32，wire 名 `trade_list`）。
///
/// 任务清单**简化**：`trades` 为 `Vec<u8>` 原始字节（VarInt 长度 + 字节），不解析
/// 单个交易结构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeList {
    /// 窗口 id（VarInt）。
    pub window_id: i32,
    /// 交易条目原始字节（简化）。
    pub trades: Vec<u8>,
    /// 村民等级（VarInt）。
    pub villager_level: i32,
    /// 经验值（VarInt）。
    pub experience: i32,
    /// 是否常规村民（Bool）。
    pub is_regular_villager: bool,
    /// 能否补货（Bool）。
    pub can_restock: bool,
}

impl Packet for TradeList {
    fn packet_id(&self) -> i32 {
        0x32
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(TradeList {
            window_id: buf.get_varint()?,
            trades: read_byte_array(buf)?,
            villager_level: buf.get_varint()?,
            experience: buf.get_varint()?,
            is_regular_villager: buf.get_bool()?,
            can_restock: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        write_byte_array(buf, &self.trades)?;
        buf.put_varint(self.villager_level);
        buf.put_varint(self.experience);
        buf.put_bool(self.is_regular_villager);
        buf.put_bool(self.can_restock);
        Ok(())
    }
}

/// 矿车移动（clientbound, id 0x35，wire 名 `move_minecart`）。
///
/// 任务清单：`entity_id`(VarInt)+`x/y/z`(Double)。Java 1.21.11 实际为
/// `entity_id` + `lerp_steps` 列表，本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveMinecart {
    /// 实体 id（VarInt）。
    pub entity_id: i32,
    /// x 坐标（Double）。
    pub x: f64,
    /// y 坐标（Double）。
    pub y: f64,
    /// z 坐标（Double）。
    pub z: f64,
}

impl Packet for MoveMinecart {
    fn packet_id(&self) -> i32 {
        0x35
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(MoveMinecart {
            entity_id: buf.get_varint()?,
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        Ok(())
    }
}

/// 载具移动（clientbound, id 0x37，wire 名 `vehicle_move`）。
///
/// 与 serverbound `VehicleMove`(0x21) 同名冲突，故命名为 `ClientboundVehicleMove`。
/// 注意 clientbound 侧无 `on_ground` 字段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientboundVehicleMove {
    /// x 坐标（Double）。
    pub x: f64,
    /// y 坐标（Double）。
    pub y: f64,
    /// z 坐标（Double）。
    pub z: f64,
    /// 偏航角（Float）。
    pub yaw: f32,
    /// 俯仰角（Float）。
    pub pitch: f32,
}

impl Packet for ClientboundVehicleMove {
    fn packet_id(&self) -> i32 {
        0x37
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientboundVehicleMove {
            x: buf.get_f64()?,
            y: buf.get_f64()?,
            z: buf.get_f64()?,
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.x);
        buf.put_f64(self.y);
        buf.put_f64(self.z);
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        Ok(())
    }
}

/// 打开书本（clientbound, id 0x38，wire 名 `open_book`）。
///
/// `hand`(VarInt)：0=主手，1=副手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenBook {
    /// 手持（VarInt）。
    pub hand: i32,
}

impl Packet for OpenBook {
    fn packet_id(&self) -> i32 {
        0x38
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(OpenBook {
            hand: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.hand);
        Ok(())
    }
}

/// 打开窗口（clientbound, id 0x39，wire 名 `open_window`）。
///
/// `title`**简化**为 String（真实为 NBT JSON 组件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    /// 窗口 id（VarInt）。
    pub window_id: i32,
    /// 窗口类型（VarInt）。
    pub window_type: i32,
    /// 标题（简化 String）。
    pub title: String,
}

impl Packet for OpenWindow {
    fn packet_id(&self) -> i32 {
        0x39
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(OpenWindow {
            window_id: buf.get_varint()?,
            window_type: buf.get_varint()?,
            title: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        buf.put_varint(self.window_type);
        buf.put_string(&self.title);
        Ok(())
    }
}

/// 打开告示牌编辑器（clientbound, id 0x3a，wire 名 `open_sign_editor`）。
///
/// 任务清单：`position`(Position 协议坐标)。Java 实际后随 `is_front_text`(Bool)，
/// 本实现随任务清单省略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenSignEditor {
    /// 方块坐标（Position 打包）。
    pub position: (i32, i32, i32),
}

impl Packet for OpenSignEditor {
    fn packet_id(&self) -> i32 {
        0x3a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(OpenSignEditor {
            position: buf.get_position()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.position;
        buf.put_position(x, y, z);
        Ok(())
    }
}

/// Ping（clientbound, id 0x3b，wire 名 `ping`）。
///
/// 与 `status::{Ping,PingResponse}` 同名冲突，故命名为 `ClientboundPing` /
/// `ClientboundPingResponse`。任务清单：`id`(VarInt)。客户端以此测量延迟。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientboundPing {
    /// 编号（VarInt）。
    pub id: i32,
}

impl Packet for ClientboundPing {
    fn packet_id(&self) -> i32 {
        0x3b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientboundPing {
            id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.id);
        Ok(())
    }
}

/// Ping 响应（clientbound, id 0x3c，wire 名 `ping_response`）。
///
/// 与 `status::PingResponse` 同名冲突，故命名为 `ClientboundPingResponse`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientboundPingResponse {
    /// 编号（VarInt）。
    pub id: i32,
}

impl Packet for ClientboundPingResponse {
    fn packet_id(&self) -> i32 {
        0x3c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientboundPingResponse {
            id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.id);
        Ok(())
    }
}

/// 放置幽灵配方（clientbound, id 0x3d，wire 名 `place_ghost_recipe`）。
///
/// 任务清单：`window_id`(Byte)+`recipe_id`(VarInt)。Java 实际以 VarInt 编码窗口 id，
/// 且 recipe 为 `RecipeDisplay` 结构，本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceGhostRecipe {
    /// 窗口 id（Byte）。
    pub window_id: i8,
    /// 配方 id（VarInt）。
    pub recipe_id: i32,
}

impl Packet for PlaceGhostRecipe {
    fn packet_id(&self) -> i32 {
        0x3d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlaceGhostRecipe {
            window_id: buf.get_i8()?,
            recipe_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.window_id);
        buf.put_varint(self.recipe_id);
        Ok(())
    }
}

/// 玩家能力（clientbound, id 0x3e，wire 名 `player_abilities`）。
///
/// 与 serverbound `PlayerAbilities`(0x27) 同名冲突，故命名为
/// `ClientboundPlayerAbilities`。任务清单第三字段为 `field_of_view_modifier`，
/// Java 实际命名为 `walking_speed`（线格式同构：两个 Float）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientboundPlayerAbilities {
    /// 能力标志位（Byte）。
    pub flags: i8,
    /// 飞行速度（Float）。
    pub flying_speed: f32,
    /// 视场角修正（Float；Java 语义为行走速度）。
    pub field_of_view_modifier: f32,
}

impl Packet for ClientboundPlayerAbilities {
    fn packet_id(&self) -> i32 {
        0x3e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientboundPlayerAbilities {
            flags: buf.get_i8()?,
            flying_speed: buf.get_f32()?,
            field_of_view_modifier: buf.get_f32()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.flags);
        buf.put_f32(self.flying_speed);
        buf.put_f32(self.field_of_view_modifier);
        Ok(())
    }
}

/// 玩家聊天消息体（`SignedMessageBody.Packed` 的最小承载）。
///
/// 线格式按任务契约：`timestamp`(Long)+`salt`(Long)+`content`(String)。
/// Java 源码中该记录还有 `last_seen`（前驱消息回执），本最小承载省略
/// （框架仅转发消息内容，不追踪回执）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedMessageBodyPacked {
    /// 消息时间戳（毫秒，Long）。
    pub timestamp: i64,
    /// 盐（Long）。
    pub salt: i64,
    /// 消息内容（String）。
    pub content: String,
}

impl SignedMessageBodyPacked {
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i64(self.timestamp);
        buf.put_i64(self.salt);
        buf.put_string(&self.content);
        Ok(())
    }

    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SignedMessageBodyPacked {
            timestamp: buf.get_i64()?,
            salt: buf.get_i64()?,
            content: buf.get_string()?,
        })
    }
}

/// 消息过滤掩码（对齐 Java `FilterMask`）。
///
/// 线格式：类型 byte（0=`PassThrough`，1=`Filtered`，2=`FullyFiltered`）；
/// `Filtered` 随后写 VarInt 计数的 `u64` 位掩码数组（各元素 8 字节大端）。
///
/// **注意**：Java 枚举序位为 `PASS_THROUGH`/`FULLY_FILTERED`/`PARTIALLY_FILTERED`，
/// 本实现按任务契约取 `PassThrough`/`Filtered`/`FullyFiltered` 的 0/1/2 序位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterMask {
    /// 全部放行（类型 0）。
    PassThrough,
    /// 部分过滤（类型 1）：位掩码数组。
    Filtered(Vec<u64>),
    /// 全部过滤（类型 2）。
    FullyFiltered,
}

impl FilterMask {
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match self {
            FilterMask::PassThrough => {
                buf.put_u8(0);
                Ok(())
            }
            FilterMask::Filtered(mask) => {
                buf.put_u8(1);
                write_varint_array(buf, mask, |b, v| {
                    // u64 → i64 用位模式转换（非 `as` 缩窄），保证 ARGB 类位模式无损。
                    b.put_i64(i64::from_be_bytes(v.to_be_bytes()));
                    Ok(())
                })
            }
            FilterMask::FullyFiltered => {
                buf.put_u8(2);
                Ok(())
            }
        }
    }

    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        match buf.get_u8()? {
            0 => Ok(FilterMask::PassThrough),
            1 => {
                let mask =
                    read_varint_array(buf, |b| Ok(u64::from_be_bytes(b.get_i64()?.to_be_bytes())))?;
                Ok(FilterMask::Filtered(mask))
            }
            2 => Ok(FilterMask::FullyFiltered),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

/// 玩家聊天消息（clientbound, id 0x3f，wire 名 `player_chat_message`）。
///
/// 1.21.11 真实线格式（对齐 Java `PlayerChatMessagePacket`）：
/// `global_index`(VarInt)+`sender`(Uuid)+`index`(VarInt)+
/// `signature`(可选 ByteArray)+[`SignedMessageBodyPacked`]+
/// `unsigned_content`(可选 Component)+[`FilterMask`]+`msg_type_id`(VarInt)+
/// `msg_type_name`(Component)+`msg_type_target`(可选 Component)。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerChatMessage {
    /// 全局消息索引（VarInt）。
    pub global_index: i32,
    /// 发送者（Uuid）。
    pub sender: Uuid,
    /// 消息索引（VarInt）。
    pub index: i32,
    /// 消息签名（可选，原始字节）。
    pub signature: Option<Vec<u8>>,
    /// 签名消息体（最小承载）。
    pub message_body: SignedMessageBodyPacked,
    /// 未签名内容（可选 Component）。
    pub unsigned_content: Option<Component>,
    /// 过滤掩码。
    pub filter_mask: FilterMask,
    /// 消息类型 id（VarInt）。
    pub msg_type_id: i32,
    /// 消息类型名（Component）。
    pub msg_type_name: Component,
    /// 消息类型目标（可选 Component）。
    pub msg_type_target: Option<Component>,
}

impl Packet for PlayerChatMessage {
    fn packet_id(&self) -> i32 {
        0x3f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let global_index = buf.get_varint()?;
        let sender = buf.get_uuid()?;
        let index = buf.get_varint()?;
        let signature = if buf.get_bool()? {
            Some(read_byte_array(buf)?)
        } else {
            None
        };
        let message_body = SignedMessageBodyPacked::decode(buf)?;
        let unsigned_content = if buf.get_bool()? {
            Some(decode_component(buf)?)
        } else {
            None
        };
        let filter_mask = FilterMask::decode(buf)?;
        let msg_type_id = buf.get_varint()?;
        let msg_type_name = decode_component(buf)?;
        let msg_type_target = if buf.get_bool()? {
            Some(decode_component(buf)?)
        } else {
            None
        };
        Ok(PlayerChatMessage {
            global_index,
            sender,
            index,
            signature,
            message_body,
            unsigned_content,
            filter_mask,
            msg_type_id,
            msg_type_name,
            msg_type_target,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.global_index);
        buf.put_uuid(self.sender);
        buf.put_varint(self.index);
        match &self.signature {
            Some(sig) => {
                buf.put_bool(true);
                write_byte_array(buf, sig)?;
            }
            None => buf.put_bool(false),
        }
        self.message_body.encode(buf)?;
        match &self.unsigned_content {
            Some(c) => {
                buf.put_bool(true);
                encode_component(buf, c)?;
            }
            None => buf.put_bool(false),
        }
        self.filter_mask.encode(buf)?;
        buf.put_varint(self.msg_type_id);
        encode_component(buf, &self.msg_type_name)?;
        match &self.msg_type_target {
            Some(c) => {
                buf.put_bool(true);
                encode_component(buf, c)?;
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 战斗结束（clientbound, id 0x40，wire 名 `end_combat_event`）。
///
/// 任务清单：`duration`(VarInt)+`killer_id`(VarInt)。Java 实际仅 `duration`(VarInt)，
/// 本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndCombatEvent {
    /// 战斗时长（VarInt）。
    pub duration: i32,
    /// 击杀者实体 id（VarInt）。
    pub killer_id: i32,
}

impl Packet for EndCombatEvent {
    fn packet_id(&self) -> i32 {
        0x40
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EndCombatEvent {
            duration: buf.get_varint()?,
            killer_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.duration);
        buf.put_varint(self.killer_id);
        Ok(())
    }
}

/// 进入战斗（clientbound, id 0x41，wire 名 `enter_combat_event`），空包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnterCombatEvent;

impl Packet for EnterCombatEvent {
    fn packet_id(&self) -> i32 {
        0x41
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(EnterCombatEvent)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 死亡战斗（clientbound, id 0x42，wire 名 `death_combat_event`）。
///
/// 1.21.11 真实线格式（对齐 Java `DeathCombatEventPacket`）：
/// `player_id`(VarInt)+`message`(Component，NBT 承载)。
#[derive(Debug, Clone, PartialEq)]
pub struct DeathCombatEvent {
    /// 玩家 id（VarInt）。
    pub player_id: i32,
    /// 死亡消息（Component）。
    pub message: Component,
}

impl Packet for DeathCombatEvent {
    fn packet_id(&self) -> i32 {
        0x42
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DeathCombatEvent {
            player_id: buf.get_varint()?,
            message: decode_component(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.player_id);
        encode_component(buf, &self.message)?;
        Ok(())
    }
}

/// 面向目标（clientbound, id 0x45，wire 名 `face_player`）。
///
/// 任务清单：`feet_or_eyes`(VarInt)+`target_x/y/z`(Double)+`is_entity`(Bool)+
/// `entity_id`(Option<VarInt>)。Java 实际在实体模式还含 `entity_face_position`，
/// 本实现随任务清单省略。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FacePlayer {
    /// 脚 / 眼（VarInt，0=脚，1=眼）。
    pub feet_or_eyes: i32,
    /// 目标 x（Double）。
    pub target_x: f64,
    /// 目标 y（Double）。
    pub target_y: f64,
    /// 目标 z（Double）。
    pub target_z: f64,
    /// 是否面向实体（Bool）。
    pub is_entity: bool,
    /// 目标实体 id（Option<VarInt>）。
    pub entity_id: Option<i32>,
}

impl Packet for FacePlayer {
    fn packet_id(&self) -> i32 {
        0x45
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let feet_or_eyes = buf.get_varint()?;
        let target_x = buf.get_f64()?;
        let target_y = buf.get_f64()?;
        let target_z = buf.get_f64()?;
        let is_entity = buf.get_bool()?;
        let entity_id = if is_entity {
            Some(buf.get_varint()?)
        } else {
            None
        };
        Ok(FacePlayer {
            feet_or_eyes,
            target_x,
            target_y,
            target_z,
            is_entity,
            entity_id,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.feet_or_eyes);
        buf.put_f64(self.target_x);
        buf.put_f64(self.target_y);
        buf.put_f64(self.target_z);
        buf.put_bool(self.is_entity);
        if let Some(id) = self.entity_id {
            buf.put_varint(id);
        }
        Ok(())
    }
}

/// 玩家旋转（clientbound, id 0x47，wire 名 `player_rotation`）。
///
/// 任务清单：`yaw`/`pitch`(Float)+`flags`(Byte)+`teleport_id`(VarInt)。Java 实际为
/// `yaw`+`relative_yaw`(Bool)+`pitch`+`relative_pitch`(Bool)，本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerRotation {
    /// 偏航角（Float）。
    pub yaw: f32,
    /// 俯仰角（Float）。
    pub pitch: f32,
    /// 标志位（Byte）。
    pub flags: i8,
    /// 传送 id（VarInt）。
    pub teleport_id: i32,
}

impl Packet for PlayerRotation {
    fn packet_id(&self) -> i32 {
        0x47
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerRotation {
            yaw: buf.get_f32()?,
            pitch: buf.get_f32()?,
            flags: buf.get_i8()?,
            teleport_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f32(self.yaw);
        buf.put_f32(self.pitch);
        buf.put_i8(self.flags);
        buf.put_varint(self.teleport_id);
        Ok(())
    }
}

/// 重生（clientbound, id 0x50，wire 名 `respawn`）。
///
/// 任务清单：`dimension`(String)+`world_name`(String)+`hashed_seed`(Long)+
/// `game_mode`(Byte)+`previous_game_mode`(Byte)+`is_debug`(Bool)+`is_flat`(Bool)+
/// `copy_metadata`(Bool)+`death`(可选，简化恒 None)。Java 1.21.11 实际：
/// `dimension_type`(VarInt)、`copy_data`(Byte) 且含 `portal_cooldown`/`sea_level`，
/// 本实现随任务清单。
#[derive(Debug, Clone, PartialEq)]
pub struct Respawn {
    /// 维度名（简化 String）。
    pub dimension: String,
    /// 世界名（String）。
    pub world_name: String,
    /// 哈希种子（Long）。
    pub hashed_seed: i64,
    /// 游戏模式（Byte）。
    pub game_mode: i8,
    /// 上一游戏模式（Byte，255 表示无）。
    pub previous_game_mode: u8,
    /// 是否为调试世界（Bool）。
    pub is_debug: bool,
    /// 是否为超平坦世界（Bool）。
    pub is_flat: bool,
    /// 是否复制元数据（Bool）。
    pub copy_metadata: bool,
    /// 死亡地点（可选，任务清单简化恒为 None）。
    pub death: Option<GlobalPos>,
}

impl Packet for Respawn {
    fn packet_id(&self) -> i32 {
        0x50
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let dimension = buf.get_string()?;
        let world_name = buf.get_string()?;
        let hashed_seed = buf.get_i64()?;
        let game_mode = buf.get_i8()?;
        let previous_game_mode = buf.get_u8()?;
        let is_debug = buf.get_bool()?;
        let is_flat = buf.get_bool()?;
        let copy_metadata = buf.get_bool()?;
        let death = if buf.get_bool()? {
            Some(GlobalPos {
                dimension: buf.get_string()?,
                position: buf.get_i64()?,
            })
        } else {
            None
        };
        Ok(Respawn {
            dimension,
            world_name,
            hashed_seed,
            game_mode,
            previous_game_mode,
            is_debug,
            is_flat,
            copy_metadata,
            death,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.dimension);
        buf.put_string(&self.world_name);
        buf.put_i64(self.hashed_seed);
        buf.put_i8(self.game_mode);
        buf.put_u8(self.previous_game_mode);
        buf.put_bool(self.is_debug);
        buf.put_bool(self.is_flat);
        buf.put_bool(self.copy_metadata);
        match &self.death {
            Some(gp) => {
                buf.put_bool(true);
                buf.put_string(&gp.dimension);
                buf.put_i64(gp.position);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 选择进度标签页（clientbound, id 0x53，wire 名 `select_advancement_tab`）。
///
/// `tab_id`(Option<String>)：Bool 存在位 + String；`None` 表示关闭进度界面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectAdvancementTab {
    /// 标签页标识（Option<String>）。
    pub tab_id: Option<String>,
}

impl Packet for SelectAdvancementTab {
    fn packet_id(&self) -> i32 {
        0x53
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let tab_id = if buf.get_bool()? {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(SelectAdvancementTab { tab_id })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match &self.tab_id {
            Some(t) => {
                buf.put_bool(true);
                buf.put_string(t);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 动作栏（clientbound, id 0x55，wire 名 `action_bar`）。
///
/// `text`**简化**为 String（真实为 NBT JSON 组件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionBar {
    /// 动作栏文本（简化 String）。
    pub text: String,
}

impl Packet for ActionBar {
    fn packet_id(&self) -> i32 {
        0x55
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ActionBar {
            text: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.text);
        Ok(())
    }
}

/// 世界边界中心（clientbound, id 0x56，wire 名 `world_border_center`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBorderCenter {
    /// 中心 x（Double）。
    pub x: f64,
    /// 中心 z（Double）。
    pub z: f64,
}

impl Packet for WorldBorderCenter {
    fn packet_id(&self) -> i32 {
        0x56
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WorldBorderCenter {
            x: buf.get_f64()?,
            z: buf.get_f64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.x);
        buf.put_f64(self.z);
        Ok(())
    }
}

/// 世界边界尺寸插值（clientbound, id 0x57，wire 名 `world_border_lerp_size`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBorderLerpSize {
    /// 旧直径（Double）。
    pub old_diameter: f64,
    /// 新直径（Double）。
    pub new_diameter: f64,
    /// 变化速度（VarLong）。
    pub speed: i64,
}

impl Packet for WorldBorderLerpSize {
    fn packet_id(&self) -> i32 {
        0x57
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WorldBorderLerpSize {
            old_diameter: buf.get_f64()?,
            new_diameter: buf.get_f64()?,
            speed: buf.get_varlong()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.old_diameter);
        buf.put_f64(self.new_diameter);
        buf.put_varlong(self.speed);
        Ok(())
    }
}

/// 世界边界尺寸（clientbound, id 0x58，wire 名 `world_border_size`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldBorderSize {
    /// 直径（Double）。
    pub diameter: f64,
}

impl Packet for WorldBorderSize {
    fn packet_id(&self) -> i32 {
        0x58
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WorldBorderSize {
            diameter: buf.get_f64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_f64(self.diameter);
        Ok(())
    }
}

/// 世界边界警告延时（clientbound, id 0x59，wire 名 `world_border_warning_delay`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldBorderWarningDelay {
    /// 警告时长（VarInt）。
    pub warning_time: i32,
}

impl Packet for WorldBorderWarningDelay {
    fn packet_id(&self) -> i32 {
        0x59
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WorldBorderWarningDelay {
            warning_time: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.warning_time);
        Ok(())
    }
}

/// 世界边界警告范围（clientbound, id 0x5a，wire 名 `world_border_warning_reach`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldBorderWarningReach {
    /// 警告方块数（VarInt）。
    pub warning_blocks: i32,
}

impl Packet for WorldBorderWarningReach {
    fn packet_id(&self) -> i32 {
        0x5a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(WorldBorderWarningReach {
            warning_blocks: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.warning_blocks);
        Ok(())
    }
}

/// 摄像机（clientbound, id 0x5b，wire 名 `camera`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Camera {
    /// 摄像机目标实体 id（VarInt）。
    pub camera_id: i32,
}

impl Packet for Camera {
    fn packet_id(&self) -> i32 {
        0x5b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Camera {
            camera_id: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.camera_id);
        Ok(())
    }
}

/// 设置光标物品（clientbound, id 0x5e，wire 名 `set_cursor_item`）。
///
/// 任务清单：`window_id`(VarInt)+`slot`(Short)+`item`(ItemStack)。Java 1.21.11 实际
/// 仅 `itemStack`（窗口 id / 槽位已移除），本实现随任务清单。
#[derive(Debug, Clone, PartialEq)]
pub struct SetCursorItem {
    /// 窗口 id（VarInt）。
    pub window_id: i32,
    /// 槽位（Short）。
    pub slot: i16,
    /// 光标物品（ItemStack）。
    pub item: ItemStack,
}

impl Packet for SetCursorItem {
    fn packet_id(&self) -> i32 {
        0x5e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetCursorItem {
            window_id: buf.get_varint()?,
            slot: buf.get_i16()?,
            item: decode_item_stack(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.window_id);
        buf.put_i16(self.slot);
        encode_item_stack(&self.item, buf)
    }
}

/// 出生点坐标（clientbound, id 0x5f，wire 名 `spawn_position`）。
///
/// 任务清单：`position`(Position 协议坐标)。Java 实际含 dimension + yaw/pitch，
/// 本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnPosition {
    /// 方块坐标（Position 打包）。
    pub position: (i32, i32, i32),
}

impl Packet for SpawnPosition {
    fn packet_id(&self) -> i32 {
        0x5f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SpawnPosition {
            position: buf.get_position()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        let (x, y, z) = self.position;
        buf.put_position(x, y, z);
        Ok(())
    }
}

/// 玩家列表头尾（clientbound, id 0x78，wire 名 `player_list_header_and_footer`）。
///
/// `header`/`footer`**简化**为 String（真实为 NBT JSON 组件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerListHeaderAndFooter {
    /// 列表头部（简化 String）。
    pub header: String,
    /// 列表尾部（简化 String）。
    pub footer: String,
}

impl Packet for PlayerListHeaderAndFooter {
    fn packet_id(&self) -> i32 {
        0x78
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(PlayerListHeaderAndFooter {
            header: buf.get_string()?,
            footer: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.header);
        buf.put_string(&self.footer);
        Ok(())
    }
}

/// NBT 查询响应（clientbound, id 0x79，wire 名 `nbt_query_response`）。
///
/// `nbt`**简化**为 `Option<Vec<u8>>`：`None` 写单字节 0x00（TAG_End），`Some(bytes)`
/// 写原始 NBT 字节（自定界，恒为最后一个字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NbtQueryResponse {
    /// 事务 id（VarInt）。
    pub transaction_id: i32,
    /// NBT 原始字节（简化；None 表示 TAG_End）。
    pub nbt: Option<Vec<u8>>,
}

impl Packet for NbtQueryResponse {
    fn packet_id(&self) -> i32 {
        0x79
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let transaction_id = buf.get_varint()?;
        let rest = buf.get_bytes(buf.remaining())?;
        let nbt = if rest.len() == 1 && rest.first() == Some(&0x00) {
            None
        } else {
            Some(rest)
        };
        Ok(NbtQueryResponse {
            transaction_id,
            nbt,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.transaction_id);
        match &self.nbt {
            Some(bytes) => buf.put_bytes(bytes),
            None => buf.put_u8(0x00),
        }
        Ok(())
    }
}

/// 标题副标题（clientbound, id 0x6e，wire 名 `set_subtitle`）。
///
/// `text`**简化**为 String（真实为 NBT JSON 组件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTitleSubTitle {
    /// 副标题文本（简化 String）。
    pub text: String,
}

impl Packet for SetTitleSubTitle {
    fn packet_id(&self) -> i32 {
        0x6e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetTitleSubTitle {
            text: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.text);
        Ok(())
    }
}

/// 标题文本（clientbound, id 0x70，wire 名 `set_title`）。
///
/// `text`**简化**为 String（真实为 NBT JSON 组件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetTitleText {
    /// 标题文本（简化 String）。
    pub text: String,
}

impl Packet for SetTitleText {
    fn packet_id(&self) -> i32 {
        0x70
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetTitleText {
            text: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.text);
        Ok(())
    }
}

/// 标题计时（clientbound, id 0x71，wire 名 `set_title_time`）。
///
/// 三个字段均为 VarInt（任务清单；Java 实际用 INT）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetTitleTime {
    /// 淡入时长（VarInt）。
    pub fade_in: i32,
    /// 停留时长（VarInt）。
    pub stay: i32,
    /// 淡出时长（VarInt）。
    pub fade_out: i32,
}

impl Packet for SetTitleTime {
    fn packet_id(&self) -> i32 {
        0x71
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(SetTitleTime {
            fade_in: buf.get_varint()?,
            stay: buf.get_varint()?,
            fade_out: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.fade_in);
        buf.put_varint(self.stay);
        buf.put_varint(self.fade_out);
        Ok(())
    }
}

/// 开始配置（clientbound, id 0x74，wire 名 `start_configuration`），空包。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StartConfiguration;

impl Packet for StartConfiguration {
    fn packet_id(&self) -> i32 {
        0x74
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(StartConfiguration)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// Cookie 存储（clientbound, id 0x76，wire 名 `cookie_store`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieStore {
    /// Cookie 键（String）。
    pub key: String,
    /// Cookie 值（Byte 数组，VarInt 长度）。
    pub payload: Vec<u8>,
}

impl Packet for CookieStore {
    fn packet_id(&self) -> i32 {
        0x76
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(CookieStore {
            key: buf.get_string()?,
            payload: read_byte_array(buf)?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.key);
        write_byte_array(buf, &self.payload)
    }
}

/// 转移服务器（clientbound, id 0x7f，wire 名 `transfer`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// 目标主机（String）。
    pub host: String,
    /// 目标端口（VarInt）。
    pub port: i32,
}

impl Packet for Transfer {
    fn packet_id(&self) -> i32 {
        0x7f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Transfer {
            host: buf.get_string()?,
            port: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.host);
        buf.put_varint(self.port);
        Ok(())
    }
}

/// 弹射物力度（clientbound, id 0x85，wire 名 `projectile_power`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectilePower {
    /// 实体 id（VarInt）。
    pub entity_id: i32,
    /// 加速度力度（Double）。
    pub power: f64,
}

impl Packet for ProjectilePower {
    fn packet_id(&self) -> i32 {
        0x85
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ProjectilePower {
            entity_id: buf.get_varint()?,
            power: buf.get_f64()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.entity_id);
        buf.put_f64(self.power);
        Ok(())
    }
}

/// 自定义举报详情（clientbound, id 0x86，wire 名 `custom_report_details`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomReportDetails {
    /// 详情键值对列表（String, String）。
    pub details: Vec<(String, String)>,
}

impl Packet for CustomReportDetails {
    fn packet_id(&self) -> i32 {
        0x86
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let details = read_varint_array(buf, |b| Ok((b.get_string()?, b.get_string()?)))?;
        Ok(CustomReportDetails { details })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.details, |b, d| {
            b.put_string(&d.0);
            b.put_string(&d.1);
            Ok(())
        })
    }
}

/// 服务器链接（clientbound, id 0x87，wire 名 `server_links`）。
///
/// 每项为 `is_builtin`(Bool)+`url`(String)，**简化**（真实协议为 builtin 枚举 id 或
/// 自定义 URL 二选一）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLinks {
    /// 链接列表（是否内置, URL）。
    pub links: Vec<(bool, String)>,
}

impl Packet for ServerLinks {
    fn packet_id(&self) -> i32 {
        0x87
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let links = read_varint_array(buf, |b| Ok((b.get_bool()?, b.get_string()?)))?;
        Ok(ServerLinks { links })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.links, |b, l| {
            b.put_bool(l.0);
            b.put_string(&l.1);
            Ok(())
        })
    }
}

/// 跟踪路标（clientbound, id 0x88，wire 名 `tracked_waypoint`）。
///
/// 任务清单：`waypoint_id`(VarInt)+`tracking`(Bool)+`name`(Option<String>)+
/// `position`(Option<(i32,i32,i32)>)。Java 实际为 `operation` + `waypoint` 结构，
/// 本实现随任务清单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedWaypoint {
    /// 路标 id（VarInt）。
    pub waypoint_id: i32,
    /// 是否跟踪（Bool）。
    pub tracking: bool,
    /// 路标名（Option<String>）。
    pub name: Option<String>,
    /// 方块坐标（Option，Bool 存在位 + Position 打包）。
    pub position: Option<(i32, i32, i32)>,
}

impl Packet for TrackedWaypoint {
    fn packet_id(&self) -> i32 {
        0x88
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let waypoint_id = buf.get_varint()?;
        let tracking = buf.get_bool()?;
        let name = if buf.get_bool()? {
            Some(buf.get_string()?)
        } else {
            None
        };
        let position = if buf.get_bool()? {
            Some(buf.get_position()?)
        } else {
            None
        };
        Ok(TrackedWaypoint {
            waypoint_id,
            tracking,
            name,
            position,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.waypoint_id);
        buf.put_bool(self.tracking);
        match &self.name {
            Some(n) => {
                buf.put_bool(true);
                buf.put_string(n);
            }
            None => buf.put_bool(false),
        }
        match self.position {
            Some((x, y, z)) => {
                buf.put_bool(true);
                buf.put_position(x, y, z);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 测试实例方块状态（clientbound, id 0x7c，wire 名 `test_instance_block_status`）。
///
/// 任务清单：`status`(VarInt)+`position`(Position)+`error_message`(Option<String>)。
/// Java 实际为 `status`(COMPONENT)+`size`(VECTOR3I)，本实现随任务清单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestInstanceBlockStatus {
    /// 状态（VarInt）。
    pub status: i32,
    /// 方块坐标（Position 打包）。
    pub position: (i32, i32, i32),
    /// 错误消息（Option<String>）。
    pub error_message: Option<String>,
}

impl Packet for TestInstanceBlockStatus {
    fn packet_id(&self) -> i32 {
        0x7c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let status = buf.get_varint()?;
        let position = buf.get_position()?;
        let error_message = if buf.get_bool()? {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(TestInstanceBlockStatus {
            status,
            position,
            error_message,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_varint(self.status);
        let (x, y, z) = self.position;
        buf.put_position(x, y, z);
        match &self.error_message {
            Some(e) => {
                buf.put_bool(true);
                buf.put_string(e);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 服务器难度（clientbound, id 0x0a，wire 名 `server_difficulty`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerDifficulty {
    /// 难度（Byte，0=和平，1=简单，2=普通，3=困难）。
    pub difficulty: i8,
    /// 是否锁定（Bool）。
    pub locked: bool,
}

impl Packet for ServerDifficulty {
    fn packet_id(&self) -> i32 {
        0x0a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ServerDifficulty {
            difficulty: buf.get_i8()?,
            locked: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.difficulty);
        buf.put_bool(self.locked);
        Ok(())
    }
}

/// 资源包推送（clientbound, id 0x4f，wire 名 `resource_pack_push`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePackPush {
    /// 资源包 id（UUID）。
    pub uuid: Uuid,
    /// 下载 URL（String）。
    pub url: String,
    /// SHA-1 哈希（String）。
    pub hash: String,
    /// 是否强制（Bool）。
    pub required: bool,
    /// 提示文本（Option<String>，Bool 存在位 + String）。
    pub prompt: Option<String>,
}

impl Packet for ResourcePackPush {
    fn packet_id(&self) -> i32 {
        0x4f
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ResourcePackPush {
            uuid: buf.get_uuid()?,
            url: buf.get_string()?,
            hash: buf.get_string()?,
            required: buf.get_bool()?,
            prompt: if buf.get_bool()? {
                Some(buf.get_string()?)
            } else {
                None
            },
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.uuid);
        buf.put_string(&self.url);
        buf.put_string(&self.hash);
        buf.put_bool(self.required);
        match &self.prompt {
            Some(p) => {
                buf.put_bool(true);
                buf.put_string(p);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 资源包弹出（clientbound, id 0x4e，wire 名 `resource_pack_pop`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePackPop {
    /// 资源包 id（Option<Uuid>）。
    pub uuid: Option<Uuid>,
}

impl Packet for ResourcePackPop {
    fn packet_id(&self) -> i32 {
        0x4e
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let uuid = if buf.get_bool()? {
            Some(buf.get_uuid()?)
        } else {
            None
        };
        Ok(ResourcePackPop { uuid })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match self.uuid {
            Some(id) => {
                buf.put_bool(true);
                buf.put_uuid(id);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

// ============================ 计分板/进度/配方/对话框/BossBar（T20，clientbound） ============================

/// 计分板目标（clientbound, id 0x68，wire 名 `scoreboard_objective`）。
///
/// 任务清单：`objective_name`(String)+`action`(VarInt)+`display_name`(String 简化)+
/// `objective_type`(VarInt)。字段名 `objective_type` 对应任务清单中的 `type`
/// （Rust 关键字）。Java 实际 `mode` 为 Byte 且仅 mode 0/2 含后续字段，本实现随任务清单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreboardObjective {
    /// 目标名（String）。
    pub objective_name: String,
    /// 动作（VarInt，0=创建，1=移除，2=更新）。
    pub action: i32,
    /// 显示名（简化 String，真实为 NBT JSON 组件）。
    pub display_name: String,
    /// 显示类型（VarInt，0=整数，1=心形）。
    pub objective_type: i32,
}

impl Packet for ScoreboardObjective {
    fn packet_id(&self) -> i32 {
        0x68
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ScoreboardObjective {
            objective_name: buf.get_string()?,
            action: buf.get_varint()?,
            display_name: buf.get_string()?,
            objective_type: buf.get_varint()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.objective_name);
        buf.put_varint(self.action);
        buf.put_string(&self.display_name);
        buf.put_varint(self.objective_type);
        Ok(())
    }
}

/// 队伍（clientbound, id 0x6b，wire 名 `teams`）。
///
/// 任务清单**简化**：固定为
/// `team_name`+`action`+`display_name`+`prefix`+`suffix`+`color`+`members`。
/// 真实协议按 action 分派（0 创建 / 2 更新含全部展示字段，1/3/4 仅成员数组），
/// 本实现自洽往返但不随 action 变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Teams {
    /// 队伍名（String）。
    pub team_name: String,
    /// 动作（VarInt）。
    pub action: i32,
    /// 显示名（简化 String）。
    pub display_name: String,
    /// 前缀（简化 String）。
    pub prefix: String,
    /// 后缀（简化 String）。
    pub suffix: String,
    /// 颜色（VarInt）。
    pub color: i32,
    /// 成员列表（String 数组）。
    pub members: Vec<String>,
}

impl Packet for Teams {
    fn packet_id(&self) -> i32 {
        0x6b
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(Teams {
            team_name: buf.get_string()?,
            action: buf.get_varint()?,
            display_name: buf.get_string()?,
            prefix: buf.get_string()?,
            suffix: buf.get_string()?,
            color: buf.get_varint()?,
            members: read_varint_array(buf, |b| b.get_string())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.team_name);
        buf.put_varint(self.action);
        buf.put_string(&self.display_name);
        buf.put_string(&self.prefix);
        buf.put_string(&self.suffix);
        buf.put_varint(self.color);
        write_varint_array(buf, &self.members, |b, m| {
            b.put_string(m);
            Ok(())
        })
    }
}

/// 更新分数（clientbound, id 0x6c，wire 名 `update_score`）。
///
/// 任务清单：`entity_name`(String)+`action`(VarInt)+`objective_name`(String)+
/// `value`(Option<VarInt>，Bool 存在位 + VarInt)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateScore {
    /// 实体名（String）。
    pub entity_name: String,
    /// 动作（VarInt）。
    pub action: i32,
    /// 目标名（String）。
    pub objective_name: String,
    /// 分数（Option<VarInt>）。
    pub value: Option<i32>,
}

impl Packet for UpdateScore {
    fn packet_id(&self) -> i32 {
        0x6c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_name = buf.get_string()?;
        let action = buf.get_varint()?;
        let objective_name = buf.get_string()?;
        let value = if buf.get_bool()? {
            Some(buf.get_varint()?)
        } else {
            None
        };
        Ok(UpdateScore {
            entity_name,
            action,
            objective_name,
            value,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.entity_name);
        buf.put_varint(self.action);
        buf.put_string(&self.objective_name);
        match self.value {
            Some(v) => {
                buf.put_bool(true);
                buf.put_varint(v);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 重置分数（clientbound, id 0x4d，wire 名 `reset_score`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetScore {
    /// 实体名（String）。
    pub entity_name: String,
    /// 是否指定目标（Bool）。
    pub has_objective: bool,
    /// 目标名（Option<String>）。
    pub objective_name: Option<String>,
}

impl Packet for ResetScore {
    fn packet_id(&self) -> i32 {
        0x4d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entity_name = buf.get_string()?;
        let has_objective = buf.get_bool()?;
        let objective_name = if has_objective {
            Some(buf.get_string()?)
        } else {
            None
        };
        Ok(ResetScore {
            entity_name,
            has_objective,
            objective_name,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.entity_name);
        // has_objective 由 objective_name 是否存在推导（避免与字段冗余双写）。
        match &self.objective_name {
            Some(o) => {
                buf.put_bool(true);
                buf.put_string(o);
            }
            None => buf.put_bool(false),
        }
        Ok(())
    }
}

/// 显示计分板（clientbound, id 0x60，wire 名 `display_scoreboard`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayScoreboard {
    /// 显示位置（Byte，0=列表，1=侧栏，2=下方）。
    pub position: i8,
    /// 目标名（String）。
    pub objective_name: String,
}

impl Packet for DisplayScoreboard {
    fn packet_id(&self) -> i32 {
        0x60
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(DisplayScoreboard {
            position: buf.get_i8()?,
            objective_name: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_i8(self.position);
        buf.put_string(&self.objective_name);
        Ok(())
    }
}

/// BossBar 动作（`BossBar` 的分派负载）。
#[derive(Debug, Clone, PartialEq)]
pub enum BossBarAction {
    /// 添加（action 0）：标题 + 血量 + 颜色 + 分节 + 标志位。
    Add {
        /// 标题（简化 String）。
        title: String,
        /// 血量比例（Float）。
        health: f32,
        /// 颜色（VarInt）。
        color: i32,
        /// 分节（VarInt）。
        division: i32,
        /// 标志位（Byte）。
        flags: u8,
    },
    /// 移除（action 1）：无负载。
    Remove,
    /// 更新血量（action 2）。
    UpdateHealth(f32),
    /// 更新标题（action 3）。
    UpdateTitle(String),
    /// 更新样式（action 4）。
    UpdateStyle {
        /// 颜色（VarInt）。
        color: i32,
        /// 分节（VarInt）。
        division: i32,
    },
    /// 更新标志位（action 5）。
    UpdateFlags(u8),
}

/// BossBar（clientbound, id 0x09，wire 名 `boss_bar`）。
///
/// 线格式：`uuid`(UUID)+`action`(VarInt)，负载随 action 分派（见 [`BossBarAction`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    /// BossBar 唯一标识（UUID）。
    pub uuid: Uuid,
    /// 动作与负载。
    pub action: BossBarAction,
}

impl BossBarAction {
    /// 动作 id（VarInt）。
    fn action_id(&self) -> i32 {
        match self {
            BossBarAction::Add { .. } => 0,
            BossBarAction::Remove => 1,
            BossBarAction::UpdateHealth(_) => 2,
            BossBarAction::UpdateTitle(_) => 3,
            BossBarAction::UpdateStyle { .. } => 4,
            BossBarAction::UpdateFlags(_) => 5,
        }
    }
}

impl Packet for BossBar {
    fn packet_id(&self) -> i32 {
        0x09
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let uuid = buf.get_uuid()?;
        let action_id = buf.get_varint()?;
        let action = match action_id {
            0 => BossBarAction::Add {
                title: buf.get_string()?,
                health: buf.get_f32()?,
                color: buf.get_varint()?,
                division: buf.get_varint()?,
                flags: buf.get_u8()?,
            },
            1 => BossBarAction::Remove,
            2 => BossBarAction::UpdateHealth(buf.get_f32()?),
            3 => BossBarAction::UpdateTitle(buf.get_string()?),
            4 => BossBarAction::UpdateStyle {
                color: buf.get_varint()?,
                division: buf.get_varint()?,
            },
            5 => BossBarAction::UpdateFlags(buf.get_u8()?),
            _ => return Err(ProtocolError::InvalidValue),
        };
        Ok(BossBar { uuid, action })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.uuid);
        buf.put_varint(self.action.action_id());
        match &self.action {
            BossBarAction::Add {
                title,
                health,
                color,
                division,
                flags,
            } => {
                buf.put_string(title);
                buf.put_f32(*health);
                buf.put_varint(*color);
                buf.put_varint(*division);
                buf.put_u8(*flags);
            }
            BossBarAction::Remove => {}
            BossBarAction::UpdateHealth(health) => buf.put_f32(*health),
            BossBarAction::UpdateTitle(title) => buf.put_string(title),
            BossBarAction::UpdateStyle { color, division } => {
                buf.put_varint(*color);
                buf.put_varint(*division);
            }
            BossBarAction::UpdateFlags(flags) => buf.put_u8(*flags),
        }
        Ok(())
    }
}

/// 进度（clientbound, id 0x80，wire 名 `advancements`）。
///
/// 任务清单**简化**：每条进度为 `(advancement_id, parent_id, criteria)` 三元组，
/// 省略 display_data / requirements / sends_telemetry。`removed` 为待移除进度 id 列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advancements {
    /// 是否清空进度（Bool）。
    pub clear: bool,
    /// 进度列表（id, 父 id, 完成条件）。
    pub advancements: Vec<(String, Option<String>, Vec<String>)>,
    /// 待移除进度 id（String 数组）。
    pub removed: Vec<String>,
}

impl Packet for Advancements {
    fn packet_id(&self) -> i32 {
        0x80
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let clear = buf.get_bool()?;
        let advancements = read_varint_array(buf, |b| {
            let advancement_id = b.get_string()?;
            let parent_id = if b.get_bool()? {
                Some(b.get_string()?)
            } else {
                None
            };
            let criteria = read_varint_array(b, |b2| b2.get_string())?;
            Ok((advancement_id, parent_id, criteria))
        })?;
        let removed = read_varint_array(buf, |b| b.get_string())?;
        Ok(Advancements {
            clear,
            advancements,
            removed,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_bool(self.clear);
        write_varint_array(buf, &self.advancements, |b, a| {
            b.put_string(&a.0);
            match &a.1 {
                Some(p) => {
                    b.put_bool(true);
                    b.put_string(p);
                }
                None => b.put_bool(false),
            }
            write_varint_array(b, &a.2, |b2, c| {
                b2.put_string(c);
                Ok(())
            })
        })?;
        write_varint_array(buf, &self.removed, |b, r| {
            b.put_string(r);
            Ok(())
        })
    }
}

/// 声明配方（clientbound, id 0x83，wire 名 `declare_recipes`）。
///
/// 1.21.11 真实线格式（对齐 Java `DeclareRecipesPacket`）：
/// `item_properties`（VarInt 计数，每条为 [`RecipeProperty`] + VarInt 计数的
/// material id 列表）+ `stonecutter_recipes`（VarInt 计数，每条为
/// [`StonecutterRecipe`]）。编码 / 解码复用 T5 已实现的
/// [`RecipeProperty::encode`]/[`RecipeProperty::decode`] 与
/// [`StonecutterRecipe::encode`]/[`StonecutterRecipe::decode`] 线格式方法。
#[derive(Debug, Clone, PartialEq)]
pub struct DeclareRecipes {
    /// 配方属性 → material id 列表。
    pub item_properties: Vec<(RecipeProperty, Vec<u32>)>,
    /// 切石机配方列表。
    pub stonecutter_recipes: Vec<StonecutterRecipe>,
}

impl Packet for DeclareRecipes {
    fn packet_id(&self) -> i32 {
        0x83
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let item_properties = read_varint_array(buf, |b| {
            let property = RecipeProperty::decode(b)?;
            let materials = read_varint_array(b, |bb| {
                u32::try_from(bb.get_varint()?).map_err(|_| ProtocolError::InvalidValue)
            })?;
            Ok((property, materials))
        })?;
        let stonecutter_recipes = read_varint_array(buf, StonecutterRecipe::decode)?;
        Ok(DeclareRecipes {
            item_properties,
            stonecutter_recipes,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.item_properties, |b, (property, materials)| {
            property.encode(b);
            write_varint_array(b, materials, |bb, m| {
                let m = i32::try_from(*m).map_err(|_| ProtocolError::InvalidValue)?;
                bb.put_varint(m);
                Ok(())
            })
        })?;
        write_varint_array(buf, &self.stonecutter_recipes, |b, sr| sr.encode(b))
    }
}

/// 配方书添加（clientbound, id 0x48，wire 名 `recipe_book_add`）。
///
/// 1.21.11 真实线格式（对齐 Java `RecipeBookAddPacket` 的简化子集）：
/// `entries`（VarInt 计数，每条为 `display_id`(VarInt)+[`RecipeDisplay`]）+
/// `replace`(Bool)。Java 中每条还有 `group`/`category`/`crafting_requirements`/
/// `flags` 字段，本实现按任务契约省略。
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeBookAdd {
    /// 配方书条目：(display_id, 配方显示)。
    pub entries: Vec<(i32, RecipeDisplay)>,
    /// 是否替换（Bool）。
    pub replace: bool,
}

impl Packet for RecipeBookAdd {
    fn packet_id(&self) -> i32 {
        0x48
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let entries = read_varint_array(buf, |b| {
            let display_id = b.get_varint()?;
            let display = RecipeDisplay::decode(b)?;
            Ok((display_id, display))
        })?;
        let replace = buf.get_bool()?;
        Ok(RecipeBookAdd { entries, replace })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.entries, |b, (display_id, display)| {
            b.put_varint(*display_id);
            display.encode(b)
        })?;
        buf.put_bool(self.replace);
        Ok(())
    }
}

/// 配方书移除（clientbound, id 0x49，wire 名 `recipe_book_remove`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeBookRemove {
    /// 配方 id 列表（String 数组）。
    pub recipe_ids: Vec<String>,
}

impl Packet for RecipeBookRemove {
    fn packet_id(&self) -> i32 {
        0x49
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(RecipeBookRemove {
            recipe_ids: read_varint_array(buf, |b| b.get_string())?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.recipe_ids, |b, r| {
            b.put_string(r);
            Ok(())
        })
    }
}

/// 配方书设置（clientbound, id 0x4a，wire 名 `recipe_book_settings`）。
///
/// 1.21.11 真实线格式（对齐 Java `RecipeBookSettingsPacket`）：8 个 Bool，
/// 顺序为 crafting / smelting / blast_furnace / smoker 四种配方书各自的
/// 展开（open）与过滤（filter）状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecipeBookSettings {
    /// 合成配方书是否展开。
    pub crafting_open: bool,
    /// 合成配方书过滤是否启用。
    pub crafting_filter: bool,
    /// 熔炉配方书是否展开。
    pub smelting_open: bool,
    /// 熔炉配方书过滤是否启用。
    pub smelting_filter: bool,
    /// 高炉配方书是否展开。
    pub blast_furnace_open: bool,
    /// 高炉配方书过滤是否启用。
    pub blast_furnace_filter: bool,
    /// 烟熏炉配方书是否展开。
    pub smoker_open: bool,
    /// 烟熏炉配方书过滤是否启用。
    pub smoker_filter: bool,
}

impl Packet for RecipeBookSettings {
    fn packet_id(&self) -> i32 {
        0x4a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(RecipeBookSettings {
            crafting_open: buf.get_bool()?,
            crafting_filter: buf.get_bool()?,
            smelting_open: buf.get_bool()?,
            smelting_filter: buf.get_bool()?,
            blast_furnace_open: buf.get_bool()?,
            blast_furnace_filter: buf.get_bool()?,
            smoker_open: buf.get_bool()?,
            smoker_filter: buf.get_bool()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_bool(self.crafting_open);
        buf.put_bool(self.crafting_filter);
        buf.put_bool(self.smelting_open);
        buf.put_bool(self.smelting_filter);
        buf.put_bool(self.blast_furnace_open);
        buf.put_bool(self.blast_furnace_filter);
        buf.put_bool(self.smoker_open);
        buf.put_bool(self.smoker_filter);
        Ok(())
    }
}

/// 显示对话框（clientbound, id 0x8a，wire 名 `show_dialog`）。
///
/// `display_name` 以 [`Component`]（NBT）承载。每项动作为
/// `action_type`(VarInt)+`text`(String)+`tooltip`(可选 String)。
#[derive(Debug, Clone, PartialEq)]
pub struct ShowDialog {
    /// 对话框 id（UUID）。
    pub dialog_id: Uuid,
    /// 显示名（Component，NBT 承载）。
    pub display_name: Component,
    /// 对话框类型（VarInt）。
    pub dialog_type: i32,
    /// 动作列表（类型, 文本, 提示）。
    pub actions: Vec<(i32, String, Option<String>)>,
}

impl Packet for ShowDialog {
    fn packet_id(&self) -> i32 {
        0x8a
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let dialog_id = buf.get_uuid()?;
        let display_name = decode_component(buf)?;
        let dialog_type = buf.get_varint()?;
        let actions = read_varint_array(buf, |b| {
            let action_type = b.get_varint()?;
            let text = b.get_string()?;
            let tooltip = if b.get_bool()? {
                Some(b.get_string()?)
            } else {
                None
            };
            Ok((action_type, text, tooltip))
        })?;
        Ok(ShowDialog {
            dialog_id,
            display_name,
            dialog_type,
            actions,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.dialog_id);
        encode_component(buf, &self.display_name)?;
        buf.put_varint(self.dialog_type);
        write_varint_array(buf, &self.actions, |b, a| {
            b.put_varint(a.0);
            b.put_string(&a.1);
            match &a.2 {
                Some(t) => {
                    b.put_bool(true);
                    b.put_string(t);
                }
                None => b.put_bool(false),
            }
            Ok(())
        })
    }
}

/// 清除对话框（clientbound, id 0x89，wire 名 `clear_dialog`）。
///
/// 任务清单：`dialog_id`(UUID)。注意 Java 实现为空包，本实现随任务清单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClearDialog {
    /// 对话框 id（UUID）。
    pub dialog_id: Uuid,
}

impl Packet for ClearDialog {
    fn packet_id(&self) -> i32 {
        0x89
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClearDialog {
            dialog_id: buf.get_uuid()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_uuid(self.dialog_id);
        Ok(())
    }
}

/// 读取 optional anonymous NBT 占位。
///
/// anonymous NBT 是自定界结构（以 `TAG_End` 结尾），不解析 NBT 便无法得知其字节
/// 长度。本框架的 NBT 接入由后续任务负责，此处仅支持 `None`（线格式单字节 0x00）；
/// 遇到 `Some` 时返回 [`ProtocolError::UnexpectedEof`]，避免读游标错位。
fn read_opt_nbt(buf: &mut ByteBuffer) -> Result<Option<Vec<u8>>, ProtocolError> {
    if !buf.get_bool()? {
        return Ok(None);
    }
    Err(ProtocolError::UnexpectedEof)
}

/// 写出 optional anonymous NBT 占位：布尔存在位 + 原始字节。
fn write_opt_nbt(buf: &mut ByteBuffer, data: &Option<Vec<u8>>) {
    match data {
        Some(bytes) => {
            buf.put_bool(true);
            buf.put_bytes(bytes);
        }
        None => buf.put_bool(false),
    }
}

/// 编码文本组件为线格式：`TAG_COMPOUND`(0x0a) + anonymous NBT payload
/// （复用 [`crate::protocol::nbt::encode_anonymous`]，对齐 Java
/// `ComponentNetworkBufferTypeImpl` 的 `0x0a + Compound` 布局）。
fn encode_component(buf: &mut ByteBuffer, component: &Component) -> Result<(), ProtocolError> {
    let nbt =
        nbt::encode_anonymous(&component.to_nbt()).map_err(|_| ProtocolError::UnexpectedEof)?;
    buf.put_u8(0x0a);
    buf.put_bytes(&nbt);
    Ok(())
}

/// 解码文本组件：消费前导 `TAG_COMPOUND`(0x0a)，再按 anonymous NBT 自界定长度
/// 推进游标并还原为 [`Component`]。非 Compound 前导 / NBT 非法 / 无法还原为组件
/// 一律返回 [`ProtocolError`]（不 panic）。
fn decode_component(buf: &mut ByteBuffer) -> Result<Component, ProtocolError> {
    let tag_id = buf.get_u8()?;
    if tag_id != 0x0a {
        return Err(ProtocolError::InvalidValue);
    }
    let start = buf.position();
    let rest = {
        let slice = buf.as_slice();
        slice.get(start..).ok_or(ProtocolError::UnexpectedEof)?
    };
    let (tag, consumed) = nbt::decode_anonymous(rest).map_err(|_| ProtocolError::UnexpectedEof)?;
    buf.get_bytes(consumed)?;
    Component::from_nbt(&tag).map_err(|_| ProtocolError::InvalidValue)
}

/// 读取 `ByteArray`（varint 长度 + 原始字节）。
fn read_byte_array(buf: &mut ByteBuffer) -> Result<Vec<u8>, ProtocolError> {
    let len = buf.get_varint()?;
    let len_usize = usize::try_from(len).map_err(|_| ProtocolError::UnexpectedEof)?;
    buf.get_bytes(len_usize)
}

/// 写入 `ByteArray`（varint 长度 + 原始字节）。
fn write_byte_array(buf: &mut ByteBuffer, data: &[u8]) -> Result<(), ProtocolError> {
    let len = i32::try_from(data.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
    buf.put_varint(len);
    buf.put_bytes(data);
    Ok(())
}

/// 读取 varint 计数的 `T` 数组。
fn read_varint_array<T, F>(buf: &mut ByteBuffer, mut item: F) -> Result<Vec<T>, ProtocolError>
where
    F: FnMut(&mut ByteBuffer) -> Result<T, ProtocolError>,
{
    let count = buf.get_varint()?;
    let count_usize = usize::try_from(count).map_err(|_| ProtocolError::UnexpectedEof)?;
    let mut items = Vec::with_capacity(count_usize);
    for _ in 0..count_usize {
        items.push(item(buf)?);
    }
    Ok(items)
}

/// 写入 varint 计数的 `T` 数组。
fn write_varint_array<T, F>(
    buf: &mut ByteBuffer,
    items: &[T],
    mut item: F,
) -> Result<(), ProtocolError>
where
    F: FnMut(&mut ByteBuffer, &T) -> Result<(), ProtocolError>,
{
    let count = i32::try_from(items.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
    buf.put_varint(count);
    for v in items {
        item(buf, v)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::resource::recipe::{Ingredient, SlotDisplay};

    /// encode → decode 往返后应与原值一致。
    fn roundtrip<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let mut buf = ByteBuffer::with_capacity(128);
        p.encode(&mut buf).unwrap();
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = P::decode(&mut buf).unwrap();
        assert_eq!(*p, decoded);
    }

    /// 将包编码为纯包体字节（不含 packet_id），供手动断言负载布局。
    fn payload_of<P: Packet>(p: &P) -> Vec<u8> {
        let mut buf = ByteBuffer::with_capacity(64);
        p.encode(&mut buf).unwrap();
        buf.into_inner()
    }

    /// 关键包 ID 与 774 真实映射一致。
    #[test]
    fn packet_ids_match_774() {
        // 全量 774 映射表：serverbound 66 项（0x00..=0x41）、clientbound 139 项
        // （序位 = 包 ID，与 vendored PacketRegistry.java 一致）。新增包实现后
        // 须同步扩展本表。
        let serverbound: &[(i32, &str)] = &[
            (0x00, "teleport_confirm"),
            (0x01, "query_block_nbt"),
            (0x02, "select_bundle_item"),
            (0x03, "change_difficulty"),
            (0x04, "change_game_mode"),
            (0x05, "chat_ack"),
            (0x06, "chat_command"),
            (0x07, "chat_command_signed"),
            (0x08, "chat_message"),
            (0x09, "chat_session_update"),
            (0x0a, "chunk_batch_received"),
            (0x0b, "client_status"),
            (0x0c, "tick_end"),
            (0x0d, "client_settings"),
            (0x0e, "tab_complete"),
            (0x0f, "configuration_ack"),
            (0x10, "click_window_button"),
            (0x11, "click_window"),
            (0x12, "close_window"),
            (0x13, "window_slot_state"),
            (0x14, "cookie_response"),
            (0x15, "plugin_message"),
            (0x16, "debug_subscription_request"),
            (0x17, "edit_book"),
            (0x18, "query_entity_nbt"),
            (0x19, "interact_entity"),
            (0x1a, "generate_structure"),
            (0x1b, "keep_alive"),
            (0x1c, "lock_difficulty"),
            (0x1d, "position"),
            (0x1e, "position_look"),
            (0x1f, "look"),
            (0x20, "player_position_status"),
            (0x21, "vehicle_move"),
            (0x22, "steer_boat"),
            (0x23, "pick_item_from_block"),
            (0x24, "pick_item_from_entity"),
            (0x25, "ping_request"),
            (0x26, "place_recipe"),
            (0x27, "player_abilities"),
            (0x28, "player_action"),
            (0x29, "entity_action"),
            (0x2a, "input"),
            (0x2b, "player_loaded"),
            (0x2c, "pong"),
            (0x2d, "set_recipe_book_state"),
            (0x2e, "recipe_book_seen_recipe"),
            (0x2f, "name_item"),
            (0x30, "resource_pack_status"),
            (0x31, "advancement_tab"),
            (0x32, "select_trade"),
            (0x33, "set_beacon_effect"),
            (0x34, "held_item_change"),
            (0x35, "update_command_block"),
            (0x36, "update_command_block_minecart"),
            (0x37, "creative_inventory_action"),
            (0x38, "update_jigsaw_block"),
            (0x39, "update_structure_block"),
            (0x3a, "set_test_block"),
            (0x3b, "update_sign"),
            (0x3c, "animation"),
            (0x3d, "spectate"),
            (0x3e, "test_instance_block_action"),
            (0x3f, "player_block_placement"),
            (0x40, "use_item"),
            (0x41, "custom_click_action"),
        ];
        let clientbound: &[(i32, &str)] = &[
            (0x00, "bundle"),
            (0x01, "spawn_entity"),
            (0x02, "entity_animation"),
            (0x03, "statistics"),
            (0x04, "acknowledge_block_change"),
            (0x05, "block_break_animation"),
            (0x06, "block_entity_data"),
            (0x07, "block_action"),
            (0x08, "block_change"),
            (0x09, "boss_bar"),
            (0x0a, "server_difficulty"),
            (0x0b, "chunk_batch_finished"),
            (0x0c, "chunk_batch_start"),
            (0x0d, "chunk_biomes"),
            (0x0e, "clear_titles"),
            (0x0f, "tab_complete"),
            (0x10, "declare_commands"),
            (0x11, "close_window"),
            (0x12, "container_set_content"),
            (0x13, "window_property"),
            (0x14, "set_slot"),
            (0x15, "cookie_request"),
            (0x16, "set_cooldown"),
            (0x17, "custom_chat_completion"),
            (0x18, "plugin_message"),
            (0x19, "damage_event"),
            (0x1a, "debug_block_value"),
            (0x1b, "debug_chunk_value"),
            (0x1c, "debug_entity_value"),
            (0x1d, "debug_event"),
            (0x1e, "debug_sample"),
            (0x1f, "delete_chat"),
            (0x20, "disconnect"),
            (0x21, "disguised_chat"),
            (0x22, "entity_status"),
            (0x23, "entity_position_sync"),
            (0x24, "explosion"),
            (0x25, "unload_chunk"),
            (0x26, "game_state_change"),
            (0x27, "game_test_highlight_pos"),
            (0x28, "open_horse_window"),
            (0x29, "hit_animation"),
            (0x2a, "initialize_world_border"),
            (0x2b, "keep_alive"),
            (0x2c, "map_chunk"),
            (0x2d, "world_event"),
            (0x2e, "particle"),
            (0x2f, "update_light"),
            (0x30, "join_game"),
            (0x31, "map_data"),
            (0x32, "trade_list"),
            (0x33, "entity_position"),
            (0x34, "entity_position_and_rotation"),
            (0x35, "move_minecart"),
            (0x36, "entity_rotation"),
            (0x37, "vehicle_move"),
            (0x38, "open_book"),
            (0x39, "open_window"),
            (0x3a, "open_sign_editor"),
            (0x3b, "ping"),
            (0x3c, "ping_response"),
            (0x3d, "place_ghost_recipe"),
            (0x3e, "player_abilities"),
            (0x3f, "player_chat_message"),
            (0x40, "end_combat_event"),
            (0x41, "enter_combat_event"),
            (0x42, "death_combat_event"),
            (0x43, "player_info_remove"),
            (0x44, "player_info_update"),
            (0x45, "face_player"),
            (0x46, "player_position_and_look"),
            (0x47, "player_rotation"),
            (0x48, "recipe_book_add"),
            (0x49, "recipe_book_remove"),
            (0x4a, "recipe_book_settings"),
            (0x4b, "destroy_entities"),
            (0x4c, "remove_entity_effect"),
            (0x4d, "reset_score"),
            (0x4e, "resource_pack_pop"),
            (0x4f, "resource_pack_push"),
            (0x50, "respawn"),
            (0x51, "entity_head_look"),
            (0x52, "multi_block_change"),
            (0x53, "select_advancement_tab"),
            (0x54, "server_data"),
            (0x55, "action_bar"),
            (0x56, "world_border_center"),
            (0x57, "world_border_lerp_size"),
            (0x58, "world_border_size"),
            (0x59, "world_border_warning_delay"),
            (0x5a, "world_border_warning_reach"),
            (0x5b, "camera"),
            (0x5c, "update_view_position"),
            (0x5d, "update_view_distance"),
            (0x5e, "set_cursor_item"),
            (0x5f, "spawn_position"),
            (0x60, "display_scoreboard"),
            (0x61, "entity_metadata"),
            (0x62, "attach_entity"),
            (0x63, "entity_velocity"),
            (0x64, "set_equipment"),
            (0x65, "set_experience"),
            (0x66, "update_health"),
            (0x67, "held_item_change"),
            (0x68, "scoreboard_objective"),
            (0x69, "set_passengers"),
            (0x6a, "set_player_inventory_slot"),
            (0x6b, "teams"),
            (0x6c, "update_score"),
            (0x6d, "update_simulation_distance"),
            (0x6e, "set_title_sub_title"),
            (0x6f, "time_update"),
            (0x70, "set_title_text"),
            (0x71, "set_title_time"),
            (0x72, "entity_sound_effect"),
            (0x73, "sound_effect"),
            (0x74, "start_configuration"),
            (0x75, "stop_sound"),
            (0x76, "cookie_store"),
            (0x77, "system_chat"),
            (0x78, "player_list_header_and_footer"),
            (0x79, "nbt_query_response"),
            (0x7a, "collect_item"),
            (0x7b, "entity_teleport"),
            (0x7c, "test_instance_block_status"),
            (0x7d, "set_tick_state"),
            (0x7e, "tick_step"),
            (0x7f, "transfer"),
            (0x80, "advancements"),
            (0x81, "entity_attributes"),
            (0x82, "entity_effect"),
            (0x83, "declare_recipes"),
            (0x84, "tags"),
            (0x85, "projectile_power"),
            (0x86, "custom_report_details"),
            (0x87, "server_links"),
            (0x88, "tracked_waypoint"),
            (0x89, "clear_dialog"),
            (0x8a, "show_dialog"),
        ];
        assert_eq!(
            serverbound.len(),
            66,
            "serverbound 应覆盖全部 66 项（0x00..=0x41）"
        );
        assert_eq!(clientbound.len(), 139, "clientbound 应覆盖全部 139 项");
        // 关键包 packet_id() 抽查：防止表与实际实现脱节。
        assert_eq!(TeleportConfirm { teleport_id: 0 }.packet_id(), 0x00);
        assert_eq!(Status { action: 0 }.packet_id(), 0x0b);
        assert_eq!(
            InteractEntity {
                target_id: 0,
                interact_type: InteractType::Attack,
                sneaking: false
            }
            .packet_id(),
            0x19
        );
        assert_eq!(
            PlayerAction {
                status: 0,
                block_position: (0, 0, 0),
                block_face: 0,
                sequence: 0
            }
            .packet_id(),
            0x28
        );
        assert_eq!(
            UseItem {
                hand: 0,
                sequence: 0,
                yaw: 0.0,
                pitch: 0.0
            }
            .packet_id(),
            0x40
        );
        assert_eq!(
            EntityAnimation {
                entity_id: 0,
                animation: 0
            }
            .packet_id(),
            0x02
        );
        assert_eq!(
            BlockChange {
                block_position: (0, 0, 0),
                block_state: 0
            }
            .packet_id(),
            0x08
        );
        assert_eq!(
            SoundEffect {
                sound_id: 0,
                sound_category: 0,
                x: 0,
                y: 0,
                z: 0,
                volume: 0.0,
                pitch: 0.0,
                seed: 0
            }
            .packet_id(),
            0x73
        );
        assert_eq!(
            SystemChatPacket {
                message: String::new(),
                overlay: false
            }
            .packet_id(),
            0x77
        );
        assert_eq!(
            ScoreboardObjective {
                objective_name: String::new(),
                action: 0,
                display_name: String::new(),
                objective_type: 0
            }
            .packet_id(),
            0x68
        );
        // T6 复杂包真实化：8 个重写包 id 与 774 映射一致（0x10/0x83/0x3f/0x42/0x8a/0x16/0x48/0x4a）。
        assert_eq!(
            DeclareCommands {
                nodes: vec![],
                root_index: 0
            }
            .packet_id(),
            0x10
        );
        assert_eq!(
            SetCooldown {
                cooldown_group: String::new(),
                cooldown_ticks: 0
            }
            .packet_id(),
            0x16
        );
        assert_eq!(
            PlayerChatMessage {
                global_index: 0,
                sender: Uuid::nil(),
                index: 0,
                signature: None,
                message_body: SignedMessageBodyPacked {
                    timestamp: 0,
                    salt: 0,
                    content: String::new(),
                },
                unsigned_content: None,
                filter_mask: FilterMask::PassThrough,
                msg_type_id: 0,
                msg_type_name: Component::Empty,
                msg_type_target: None,
            }
            .packet_id(),
            0x3f
        );
        assert_eq!(
            DeathCombatEvent {
                player_id: 0,
                message: Component::Empty,
            }
            .packet_id(),
            0x42
        );
        assert_eq!(
            RecipeBookAdd {
                entries: vec![],
                replace: false,
            }
            .packet_id(),
            0x48
        );
        assert_eq!(
            RecipeBookSettings {
                crafting_open: false,
                crafting_filter: false,
                smelting_open: false,
                smelting_filter: false,
                blast_furnace_open: false,
                blast_furnace_filter: false,
                smoker_open: false,
                smoker_filter: false,
            }
            .packet_id(),
            0x4a
        );
        assert_eq!(
            DeclareRecipes {
                item_properties: vec![],
                stonecutter_recipes: vec![],
            }
            .packet_id(),
            0x83
        );
        assert_eq!(
            ShowDialog {
                dialog_id: Uuid::nil(),
                display_name: Component::Empty,
                dialog_type: 0,
                actions: vec![],
            }
            .packet_id(),
            0x8a
        );
    }

    #[test]
    fn teleport_confirm_roundtrip() {
        roundtrip(&TeleportConfirm { teleport_id: 7 });
    }

    #[test]
    fn chunk_batch_received_roundtrip() {
        roundtrip(&ChunkBatchReceived {
            chunks_per_tick: 20.0,
        });
    }

    #[test]
    fn status_roundtrip() {
        roundtrip(&Status { action: 0 });
        roundtrip(&Status { action: 1 });
    }

    #[test]
    fn client_command_chat_roundtrip() {
        // 0x06 仅编码 / 解码 message，尾部字段由更高层忽略。
        roundtrip(&ClientCommandChatPacket {
            message: "/gamemode creative".to_string(),
        });
        roundtrip(&ClientCommandChatPacket {
            message: "help".to_string(),
        });
    }

    #[test]
    fn client_signed_command_chat_roundtrip() {
        // 0x07 同构，仅取 message。
        roundtrip(&ClientSignedCommandChatPacket {
            message: "say 你好世界".to_string(),
        });
    }

    #[test]
    fn system_chat_text_roundtrip() {
        // 0x77 文本以 NBT Compound（0x0a + encode_anonymous 无根名头）承载。
        let p = SystemChatPacket {
            message: "命令已执行".to_string(),
            overlay: false,
        };
        let payload = payload_of(&p);
        // 首字节必须为 TAG_COMPOUND（0x0a）。
        assert_eq!(payload.first(), Some(&0x0a));
        // 末字节为 overlay（BOOL = 0）。
        assert_eq!(payload.last(), Some(&0u8));
        // 中间段等于 encode_anonymous 输出的 Compound payload。
        let nbt = nbt::encode_anonymous(&NbtTag::Compound(vec![(
            "text".to_string(),
            NbtTag::String("命令已执行".to_string()),
        )]))
        .unwrap();
        assert_eq!(payload.get(1..payload.len() - 1), Some(nbt.as_slice()));
        // 解码应还原 message 与 overlay。
        let decoded = SystemChatPacket::decode(&mut ByteBuffer::new(payload.clone())).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn system_chat_overlay_true_roundtrip() {
        let p = SystemChatPacket {
            message: "actionbar".to_string(),
            overlay: true,
        };
        let payload = payload_of(&p);
        assert_eq!(payload.last(), Some(&1u8));
        let decoded = SystemChatPacket::decode(&mut ByteBuffer::new(payload.clone())).unwrap();
        assert_eq!(decoded, p);
    }

    #[test]
    fn keep_alive_roundtrip() {
        roundtrip(&KeepAlive {
            keep_alive_id: 0x1234_5678_9abc_def0,
        });
    }

    #[test]
    fn player_position_roundtrip() {
        roundtrip(&PlayerPosition {
            x: 1.5,
            y: 64.0,
            z: -10.25,
            grounded: true,
        });
    }

    #[test]
    fn player_position_and_rotation_roundtrip() {
        roundtrip(&PlayerPositionAndRotation {
            x: 100.0,
            y: 63.5,
            z: -200.0,
            yaw: 45.0,
            pitch: -30.0,
            grounded: false,
        });
    }

    #[test]
    fn look_roundtrip() {
        roundtrip(&Look {
            yaw: 90.0,
            pitch: -10.0,
            on_ground: true,
        });
    }

    #[test]
    fn player_position_status_roundtrip() {
        roundtrip(&PlayerPositionStatus {
            flags: PlayerPositionStatus::FLAG_ON_GROUND,
        });
        roundtrip(&PlayerPositionStatus {
            flags: PlayerPositionStatus::FLAG_ON_GROUND
                | PlayerPositionStatus::FLAG_HORIZONTAL_COLLISION,
        });
    }

    #[test]
    fn player_loaded_roundtrip() {
        assert_eq!(PlayerLoaded.packet_id(), 0x2b);
        roundtrip(&PlayerLoaded);
    }

    // ---------- serverbound 全面补全（implement-framework-capabilities R7）----------

    #[test]
    fn serverbound_query_and_world_roundtrip() {
        roundtrip(&QueryBlockNbt {
            transaction_id: 1,
            block_position: (10, 64, -2),
        });
        roundtrip(&SelectBundleItem {
            slot: 5,
            selected_index: 3,
        });
        roundtrip(&ChangeDifficulty {
            difficulty: 1,
            locked: false,
        });
        roundtrip(&ChangeGameMode { gamemode: 1 });
        roundtrip(&LockDifficulty { locked: true });
        roundtrip(&GenerateStructure {
            block_position: (1, 2, 3),
            level: 2,
            keep_jigsaws: true,
        });
        roundtrip(&PickItemFromBlock {
            pos: (0, 64, 0),
            include_data: false,
        });
        roundtrip(&PickItemFromEntity {
            entity_id: 7,
            include_data: true,
        });
    }

    #[test]
    fn serverbound_chat_roundtrip() {
        roundtrip(&ChatAck { offset: 3 });
        roundtrip(&ChatMessage {
            message: "hi".to_string(),
            timestamp: 1,
            salt: 2,
            signature: None,
            ack_offset: 0,
            ack_list: [0, 0, 0],
            checksum: 0,
        });
        roundtrip(&ChatMessage {
            message: "signed".to_string(),
            timestamp: 3,
            salt: 4,
            signature: Some(vec![0u8; 256]),
            ack_offset: 5,
            ack_list: [1, 2, 3],
            checksum: -1,
        });
        roundtrip(&ChatSessionUpdate {
            session_id: Uuid::nil(),
            expires_at: 100,
            public_key: vec![1, 2],
            signature: vec![3, 4],
        });
        roundtrip(&NameItem {
            item_name: "diamond".to_string(),
        });
    }

    #[test]
    fn serverbound_settings_and_interact_roundtrip() {
        roundtrip(&TickEnd);
        roundtrip(&ConfigurationAck);
        roundtrip(&Settings {
            locale: "zh_cn".to_string(),
            view_distance: 10,
            chat_mode: 0,
            chat_colors: true,
            displayed_skin_parts: 0x7f,
            main_hand: 1,
            enable_text_filtering: false,
            allow_server_listings: true,
            particle_setting: 0,
        });
        roundtrip(&TabComplete {
            transaction_id: 1,
            text: "/warp".to_string(),
        });
        roundtrip(&ClickWindowButton {
            window_id: 0,
            button_id: 1,
        });
        roundtrip(&WindowSlotState {
            slot: 3,
            window_id: 0,
            new_state: true,
        });
        roundtrip(&InteractEntity {
            target_id: 9,
            interact_type: InteractType::Interact { hand: 0 },
            sneaking: false,
        });
        roundtrip(&InteractEntity {
            target_id: 9,
            interact_type: InteractType::Attack,
            sneaking: true,
        });
        roundtrip(&EntityAction {
            player_id: 1,
            action: 1,
            horse_jump_boost: 0,
        });
        roundtrip(&Animation { hand: 0 });
    }

    #[test]
    fn serverbound_movement_and_abilities_roundtrip() {
        roundtrip(&VehicleMove {
            x: 1.5,
            y: 64.0,
            z: -2.5,
            yaw: 10.0,
            pitch: -5.0,
            on_ground: true,
        });
        roundtrip(&SteerBoat {
            left_paddle: true,
            right_paddle: false,
        });
        roundtrip(&PingRequest { number: 42 });
        roundtrip(&Pong { id: 42 });
        roundtrip(&PlayerAbilities { flags: 0x01 });
        roundtrip(&Input { flags: 0 });
        roundtrip(&Input { flags: 0x40 });
        roundtrip(&PlayerAction {
            status: 1,
            block_position: (8, 64, 8),
            block_face: 1,
            sequence: 5,
        });
        roundtrip(&Spectate {
            target: Uuid::nil(),
        });
    }

    #[test]
    fn serverbound_inventory_and_blocks_roundtrip() {
        roundtrip(&CookieResponse {
            key: "mc:brand".to_string(),
            value: None,
        });
        roundtrip(&CookieResponse {
            key: "k".to_string(),
            value: Some(vec![0xde, 0xad]),
        });
        roundtrip(&ClientPluginMessage {
            channel: "minecraft:brand".to_string(),
            data: vec![1, 2, 3],
        });
        roundtrip(&DebugSubscriptionRequest {
            subscriptions: vec![1, 2],
        });
        roundtrip(&EditBook {
            slot: 0,
            pages: vec!["p1".to_string()],
            title: Some("t".to_string()),
        });
        roundtrip(&QueryEntityNbt {
            transaction_id: 1,
            entity_id: 2,
        });
        roundtrip(&PlaceRecipe {
            window_id: 0,
            recipe_display_id: 1,
            make_all: false,
        });
        roundtrip(&CreativeInventoryAction {
            slot: 1,
            item: ItemStack::AIR,
        });
        roundtrip(&UpdateCommandBlock {
            block_position: (1, 2, 3),
            command: "say hi".to_string(),
            mode: 0,
            flags: 0,
        });
        roundtrip(&UpdateCommandBlockMinecart {
            entity_id: 4,
            command: "say hi".to_string(),
            track_output: true,
        });
        roundtrip(&UpdateSign {
            block_position: (1, 2, 3),
            is_front_text: true,
            lines: vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ],
        });
        roundtrip(&SetTestBlock {
            block_position: (0, 0, 0),
            mode: 0,
            message: "m".to_string(),
        });
    }

    #[test]
    fn serverbound_misc_and_placement_roundtrip() {
        roundtrip(&SetRecipeBookState {
            book_type: 0,
            book_open: true,
            filter_active: false,
        });
        roundtrip(&RecipeBookSeenRecipe { recipe_id: 3 });
        roundtrip(&ResourcePackStatus {
            id: Uuid::nil(),
            status: 0,
        });
        roundtrip(&AdvancementTab {
            action: 0,
            tab_identifier: Some("minecraft:story".to_string()),
        });
        roundtrip(&AdvancementTab {
            action: 1,
            tab_identifier: None,
        });
        roundtrip(&SelectTrade { selected_slot: 2 });
        roundtrip(&SetBeaconEffect {
            primary_effect: Some(1),
            secondary_effect: None,
        });
        roundtrip(&UpdateJigsawBlock {
            location: (1, 2, 3),
            name: "n".to_string(),
            target: "t".to_string(),
            pool: "p".to_string(),
            final_state: "f".to_string(),
            joint_type: "j".to_string(),
            selection_priority: 0,
            placement_priority: 1,
        });
        roundtrip(&UpdateStructureBlock {
            location: (1, 2, 3),
            action: 0,
            mode: 0,
            name: "n".to_string(),
            offset: (0, 0, 0),
            size: (1, 1, 1),
            mirror: 0,
            rotation: 0,
            metadata: "".to_string(),
            integrity: 1.0,
            seed: 0,
            flags: 0,
        });
        roundtrip(&TestInstanceBlockAction {
            block_position: (1, 2, 3),
            action: 0,
            data: TestInstanceBlockActionData {
                test: None,
                size: (1, 1, 1),
                rotation: 0,
                ignore_entities: false,
                status: 0,
                error_message: None,
            },
        });
        roundtrip(&PlayerBlockPlacement {
            hand: 0,
            block_position: (1, 2, 3),
            block_face: 1,
            cursor_position_x: 0.5,
            cursor_position_y: 0.5,
            cursor_position_z: 0.5,
            inside_block: false,
            hit_world_border: false,
            sequence: 1,
        });
        roundtrip(&UseItem {
            hand: 0,
            sequence: 1,
            yaw: 0.0,
            pitch: 0.0,
        });
        roundtrip(&CustomClickAction {
            key: "k".to_string(),
            payload: vec![9, 9],
        });
    }

    // ---------- clientbound 全面补全（implement-framework-capabilities R7）----------

    #[test]
    fn clientbound_entity_animation_and_status_roundtrip() {
        roundtrip(&EntityAnimation {
            entity_id: 1,
            animation: 0,
        });
        roundtrip(&HitAnimation {
            entity_id: 1,
            yaw: 1.5,
        });
        roundtrip(&EntityStatus {
            entity_id: 1,
            status: 2,
        });
        roundtrip(&EntityPositionSync {
            entity_id: 1,
            x: 10.0,
            y: 64.0,
            z: -2.0,
            d_x: 0.0,
            d_y: 0.0,
            d_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        });
        roundtrip(&EntityPosition {
            entity_id: 1,
            d_x: 10,
            d_y: 0,
            d_z: -2,
            on_ground: true,
        });
        roundtrip(&EntityPositionAndRotation {
            entity_id: 1,
            d_x: 1,
            d_y: 2,
            d_z: 3,
            yaw: 10,
            pitch: 20,
            on_ground: false,
        });
        roundtrip(&EntityRotation {
            entity_id: 1,
            yaw: 10,
            pitch: 20,
            on_ground: false,
        });
    }

    #[test]
    fn clientbound_entity_meta_and_links_roundtrip() {
        roundtrip(&EntityVelocity {
            entity_id: 1,
            // LP_VECTOR3 按 scale=ceil(max) 归一化并 15 位量化：scale=1 且分量为
            // ±1/0 时恰在量化网格上，roundtrip 无损失。
            velocity: [1.0, 0.0, -1.0],
        });
        roundtrip(&AttachEntity {
            attached_entity_id: 1,
            holding_entity_id: 2,
        });
        roundtrip(&SetPassengers {
            entity_id: 1,
            passengers: vec![2, 3],
        });
        roundtrip(&EntityHeadLook {
            entity_id: 1,
            head_yaw: 30,
        });
        roundtrip(&CollectItem {
            collected_entity_id: 1,
            collector_entity_id: 2,
            pickup_count: 3,
        });
        roundtrip(&RemoveEntityEffect {
            entity_id: 1,
            effect_id: 5,
        });
        roundtrip(&EntityEffect {
            entity_id: 1,
            effect_id: 5,
            amplifier: 1,
            duration: 100,
            flags: 0,
        });
    }

    #[test]
    fn clientbound_attributes_and_xp_roundtrip() {
        roundtrip(&EntityAttributes {
            entity_id: 1,
            properties: vec![AttributeProperty {
                attribute_id: 0,
                value: 1.5,
                modifiers: vec![AttributeModifier {
                    modifier_id: "minecraft:test".to_string(),
                    amount: 2.0,
                    operation: 0,
                }],
            }],
        });
        roundtrip(&SetExperience {
            experience_bar: 0.5,
            level: 10,
            total_experience: 100,
        });
        roundtrip(&SetPlayerInventorySlot {
            slot: 36,
            item: ItemStack::AIR,
        });
        roundtrip(&DamageEvent {
            target_entity_id: 1,
            damage_type_id: 2,
            source_cause_id: 3,
            source_direct_id: 4,
            source_position: Some((1.0, 64.0, 2.0)),
        });
        roundtrip(&DamageEvent {
            target_entity_id: 1,
            damage_type_id: 2,
            source_cause_id: 3,
            source_direct_id: 4,
            source_position: None,
        });
    }

    #[test]
    fn clientbound_block_world_roundtrip() {
        roundtrip(&BlockChange {
            block_position: (1, 2, 3),
            block_state: 1,
        });
        roundtrip(&MultiBlockChange {
            chunk_section_position: 1234,
            blocks: vec![1, 2, 3],
        });
        roundtrip(&AcknowledgeBlockChange { sequence_id: 7 });
        roundtrip(&BlockAction {
            block_position: (1, 2, 3),
            action_id: 1,
            action_param: 2,
            block_type: 3,
        });
        roundtrip(&BlockEntityData {
            block_position: (1, 2, 3),
            block_entity_type: 1,
            nbt_data: vec![0x0a, 0x00, 0x00],
        });
        roundtrip(&BlockBreakAnimation {
            entity_id: 1,
            block_position: (1, 2, 3),
            destroy_stage: 5,
        });
        roundtrip(&UnloadChunk {
            chunk_x: 3,
            chunk_z: -4,
        });
        roundtrip(&ChunkBiomes {
            chunks: vec![ChunkBiomeData {
                chunk_x: 0,
                chunk_z: 0,
                data: vec![1, 2, 3],
            }],
        });
    }

    #[test]
    fn clientbound_light_and_events_roundtrip() {
        roundtrip(&UpdateLight {
            chunk_x: 0,
            chunk_z: 0,
            sky_light_mask: vec![1],
            block_light_mask: vec![0],
            empty_sky_light_mask: vec![],
            empty_block_light_mask: vec![],
            sky_light: vec![vec![0u8; 8]],
            block_light: vec![],
        });
        roundtrip(&WorldEvent {
            event: 1023,
            block_position: (1, 2, 3),
            data: 0,
            disable_relative_volume: false,
        });
        roundtrip(&Particle {
            particle_id: 1,
            long_distance: false,
            x: 1.0,
            y: 2.0,
            z: 3.0,
            offset_x: 0.1,
            offset_y: 0.2,
            offset_z: 0.3,
            max_speed: 1.0,
            particle_count: 10,
        });
        roundtrip(&Explosion {
            x: 1.0,
            y: 64.0,
            z: 2.0,
            strength: 4.0,
            records: vec![ExplosionRecord { xz: 1, y: 2 }],
            player_motion_x: 0.1,
            player_motion_y: 0.2,
            player_motion_z: 0.3,
        });
        roundtrip(&Explosion {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            strength: 0.0,
            records: vec![],
            player_motion_x: 0.0,
            player_motion_y: 0.0,
            player_motion_z: 0.0,
        });
    }

    #[test]
    fn clientbound_sound_and_tags_roundtrip() {
        roundtrip(&SoundEffect {
            sound_id: 1,
            sound_category: 2,
            x: 100,
            y: 64,
            z: 200,
            volume: 1.0,
            pitch: 1.2,
            seed: 42,
        });
        roundtrip(&EntitySoundEffect {
            sound_id: 1,
            sound_category: 2,
            entity_id: 3,
            volume: 0.5,
            pitch: 2.0,
            seed: 7,
        });
        roundtrip(&StopSound {
            flags: 3,
            source: Some(2),
            sound: Some("minecraft:ambient".to_string()),
        });
        roundtrip(&Tags {
            registries: vec![TagsRegistry {
                registry: "minecraft:block".to_string(),
                tags: vec![TagEntry {
                    name: "minecraft:stone".to_string(),
                    entries: vec![1, 2],
                }],
            }],
        });
        roundtrip(&ServerData {
            motd: r#"{"text":"Hi"}"#.to_string(),
            icon: None,
            enforces_secure_chat: false,
        });
    }

    #[test]
    fn clientbound_time_and_view_roundtrip() {
        roundtrip(&TimeUpdate {
            world_age: 1000,
            time_of_day: 6000,
            tick_day_time: false,
        });
        roundtrip(&SetTickState {
            tick_rate: 20.0,
            is_frozen: false,
        });
        roundtrip(&TickStep { tick_steps: 5 });
        roundtrip(&UpdateSimulationDistance {
            simulation_distance: 10,
        });
        roundtrip(&UpdateViewPosition {
            chunk_x: 1,
            chunk_z: 2,
        });
        roundtrip(&UpdateViewDistance { view_distance: 10 });
    }

    #[test]
    fn spawn_entity_roundtrip() {
        roundtrip(&SpawnEntity {
            entity_id: 10,
            object_uuid: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            entity_type: 74, // minecraft:player 的协议序号（占位值）
            x: 1.0,
            y: 65.0,
            z: 2.0,
            velocity: [0, 0, 0],
            pitch: 0,
            yaw: 40, // 256 分度制
            head_pitch: 0,
            object_data: 0,
        });
    }

    #[test]
    fn position_roundtrip() {
        roundtrip(&Position {
            teleport_id: 7,
            x: 0.0,
            y: 64.0,
            z: 0.0,
            dx: 0.0,
            dy: 0.0,
            dz: 0.0,
            yaw: 90.0,
            pitch: 0.0,
            flags: 0b0000_0100, // Y_ROT（Y 为相对值）
        });
    }

    #[test]
    fn update_health_roundtrip() {
        roundtrip(&UpdateHealth {
            health: 20.0,
            food: 20,
            food_saturation: 5.0,
        });
    }

    #[test]
    fn player_info_roundtrip() {
        roundtrip(&PlayerInfo {
            uuid: Uuid::from_u128(0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff),
            name: "Steve".to_string(),
            properties: vec![
                Property {
                    name: "textures".to_string(),
                    value: "eyJ0ZXh0dXJlcyI6e319".to_string(),
                    signature: Some("c2lnbmF0dXJl".to_string()),
                },
                Property {
                    name: "preferredLanguage".to_string(),
                    value: "zh_cn".to_string(),
                    signature: None,
                },
            ],
        });
    }

    #[test]
    fn game_state_change_roundtrip() {
        roundtrip(&GameStateChange {
            event: 3, // CHANGE_GAMEMODE
            data: 1.0,
        });
    }

    #[test]
    fn chunk_batch_start_roundtrip() {
        assert_eq!(ChunkBatchStart.packet_id(), 0x0c);
        roundtrip(&ChunkBatchStart);
    }

    #[test]
    fn chunk_batch_finished_roundtrip() {
        roundtrip(&ChunkBatchFinished { batch_size: 512 });
    }

    #[test]
    fn map_chunk_roundtrip() {
        roundtrip(&MapChunk {
            chunk_x: 1,
            chunk_z: -2,
            heightmaps: vec![Heightmap {
                map_type: 4, // MOTION_BLOCKING
                data: vec![0x2a2a_2a2a_2a2a_2a2a, 0x1111_1111_1111_1111],
            }],
            chunk_data: vec![0x00, 0x01, 0x02, 0xff],
            block_entities: vec![ChunkBlockEntity {
                xz: 0x37, // 局部坐标打包：高 4 位 x=3，低 4 位 z=7
                y: 60,
                block_entity_type: 8,
                nbt_data: None,
            }],
            sky_light_mask: vec![0x01, 0x02],
            block_light_mask: vec![0x03],
            empty_sky_light_mask: vec![],
            empty_block_light_mask: vec![],
            sky_light: vec![vec![0xAB; 2048]],
            block_light: vec![vec![0xCD; 2048]],
        });
    }

    #[test]
    fn login_roundtrip_without_death() {
        roundtrip(&Login {
            entity_id: 1,
            is_hardcore: false,
            world_names: vec!["minecraft:overworld".to_string()],
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            world_state: SpawnInfo {
                dimension: 0,
                name: "minecraft:overworld".to_string(),
                hashed_seed: 0x1234_5678_9abc_def0,
                gamemode: 0,
                previous_gamemode: 255,
                is_debug: false,
                is_flat: false,
                death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            enforces_secure_chat: true,
        });
    }

    #[test]
    fn login_roundtrip_with_death() {
        roundtrip(&Login {
            entity_id: 2,
            is_hardcore: true,
            world_names: vec![],
            max_players: 1,
            view_distance: 8,
            simulation_distance: 8,
            reduced_debug_info: true,
            enable_respawn_screen: false,
            do_limited_crafting: true,
            world_state: SpawnInfo {
                dimension: 1,
                name: "minecraft:the_nether".to_string(),
                hashed_seed: 0,
                gamemode: 1,
                previous_gamemode: 0,
                is_debug: false,
                is_flat: true,
                death: Some(GlobalPos {
                    dimension: "minecraft:overworld".to_string(),
                    position: (12i64 << 38) | (20i64 << 12) | 64, // 打包坐标 (12, 64, 20)
                }),
                portal_cooldown: 300,
                sea_level: 63,
            },
            enforces_secure_chat: false,
        });
    }

    /// NBT 占位：含 NBT 的方块实体解码返回 Err（游标无法推进，由后续任务接线）。
    #[test]
    fn map_chunk_nbt_placeholder_rejects_some() {
        let mut buf = ByteBuffer::with_capacity(16);
        buf.put_bool(true); // has NBT
        buf.put_bytes(&[0x0a, 0x00]);
        let mut buf = ByteBuffer::new(buf.into_inner());
        assert_eq!(read_opt_nbt(&mut buf), Err(ProtocolError::UnexpectedEof));
    }

    #[test]
    fn player_remove_roundtrip() {
        assert_eq!(
            PlayerRemove {
                players: vec![Uuid::nil(), Uuid::from_u128(1)]
            }
            .packet_id(),
            0x43
        );
        roundtrip(&PlayerRemove {
            players: vec![
                Uuid::nil(),
                Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            ],
        });
    }

    #[test]
    fn entity_teleport_roundtrip() {
        assert_eq!(
            EntityTeleport {
                entity_id: 1,
                x: 0.0,
                y: 64.0,
                z: 0.0,
                yaw: 0,
                pitch: 0,
                on_ground: true,
            }
            .packet_id(),
            0x7b
        );
        roundtrip(&EntityTeleport {
            entity_id: 42,
            x: 100.5,
            y: 65.0,
            z: -200.25,
            yaw: 90,
            pitch: -30,
            on_ground: true,
        });
    }

    #[test]
    fn rel_entity_move_roundtrip() {
        assert_eq!(
            RelEntityMove {
                entity_id: 1,
                d_x: 0,
                d_y: 0,
                d_z: 0,
                on_ground: false,
            }
            .packet_id(),
            0x33
        );
        roundtrip(&RelEntityMove {
            entity_id: 7,
            d_x: 100,
            d_y: -50,
            d_z: 200,
            on_ground: true,
        });
    }

    #[test]
    fn entity_metadata_roundtrip() {
        let p = EntityMetaData {
            entity_id: 7,
            entries: vec![
                (8, EntityMetadataValue::Byte(0x02)), // flags
                (
                    2,
                    EntityMetadataValue::String("{\"text\":\"Hi\"}".to_string()), // custom_name
                ),
                (9, EntityMetadataValue::Float(20.0)), // health
                (0, EntityMetadataValue::VarInt(5)),
                (3, EntityMetadataValue::Bool(true)),
            ],
            has_metadata: false,
        };
        assert_eq!(p.packet_id(), 0x61);
        roundtrip(&p);
    }

    #[test]
    fn entity_metadata_byte_wire_layout() {
        // entity_id(VarInt 0x01) | index(0x08) | type(0x00) | value(0x02)
        // | terminator(0xff) | has_metadata(0x00)
        let p = EntityMetaData {
            entity_id: 1,
            entries: vec![(8, EntityMetadataValue::Byte(0x02))],
            has_metadata: false,
        };
        assert_eq!(payload_of(&p), vec![0x01, 0x08, 0x00, 0x02, 0xff, 0x00]);
    }

    #[test]
    fn entity_metadata_string_wire_layout() {
        // entity_id(0x00) | index(0x02) | type(0x03) | len(0x02) | 'h' 'i'
        // | terminator(0xff) | has_metadata(0x01)
        let p = EntityMetaData {
            entity_id: 0,
            entries: vec![(2, EntityMetadataValue::String("hi".to_string()))],
            has_metadata: true,
        };
        assert_eq!(
            payload_of(&p),
            vec![0x00, 0x02, 0x03, 0x02, b'h', b'i', 0xff, 0x01]
        );
    }

    #[test]
    fn destroy_entities_roundtrip() {
        assert_eq!(
            DestroyEntities {
                entity_ids: vec![1, 2, 3]
            }
            .packet_id(),
            0x4b
        );
        roundtrip(&DestroyEntities {
            entity_ids: vec![1, 2, 3],
        });
        roundtrip(&DestroyEntities { entity_ids: vec![] });
    }

    #[test]
    fn destroy_entities_wire_layout() {
        // count(0x02) | id(0x01) | id(0x2a)
        assert_eq!(
            payload_of(&DestroyEntities {
                entity_ids: vec![1, 42]
            }),
            vec![0x02, 0x01, 0x2a]
        );
    }

    // ---- 库存 clientbound 包（见 `.specs/implement-item-inventory/`）----

    #[test]
    fn window_items_packet_id() {
        assert_eq!(
            WindowItemsPacket {
                window_id: 0,
                state_id: 0,
                items: vec![],
                carried_item: ItemStack::AIR,
            }
            .packet_id(),
            0x12
        );
    }

    #[test]
    fn set_slot_packet_id() {
        assert_eq!(
            SetSlotPacket {
                window_id: 0,
                state_id: 0,
                slot: 0,
                item: ItemStack::AIR,
            }
            .packet_id(),
            0x14
        );
    }

    #[test]
    fn entity_equipment_packet_id() {
        assert_eq!(
            EntityEquipmentPacket {
                entity_id: 0,
                equipments: vec![],
            }
            .packet_id(),
            0x64
        );
    }

    #[test]
    fn window_items_forty_six_air_roundtrip() {
        // 46 个 AIR 槽 + 光标 AIR，window_id/state_id 均为 0。
        let items = vec![ItemStack::AIR; 46];
        let packet = WindowItemsPacket {
            window_id: 0,
            state_id: 0,
            items,
            carried_item: ItemStack::AIR,
        };
        let mut buf = ByteBuffer::with_capacity(256);
        packet.encode(&mut buf).unwrap();
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = WindowItemsPacket::decode(&mut buf).unwrap();
        assert_eq!(decoded.items.len(), 46);
        assert!(decoded.items.iter().all(|it| it.is_air()));
        assert!(decoded.carried_item.is_air());
        assert_eq!(decoded.window_id, 0);
        assert_eq!(decoded.state_id, 0);
        // 回环一致。
        assert_eq!(decoded, packet);
    }

    #[test]
    fn set_slot_body_prefix_and_roundtrip() {
        let packet = SetSlotPacket {
            window_id: 0,
            state_id: 0,
            slot: 36,
            item: ItemStack::new(264, 1),
        };
        let mut buf = ByteBuffer::with_capacity(16);
        packet.encode(&mut buf).unwrap();
        let bytes = buf.as_slice();
        // VarInt 0, VarInt 0, i16 36（= 00 24）。
        assert_eq!(&bytes[0..4], &[0x00, 0x00, 0x00, 0x24]);
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = SetSlotPacket::decode(&mut buf).unwrap();
        assert_eq!(decoded, packet);
    }

    #[test]
    fn entity_equipment_slot_bytes_and_roundtrip() {
        // 顺序 HELMET → CHESTPLATE，共 2 项；均为 AIR。
        // 线格式：entity_id(VarInt 0x00) | 槽位字节 | 物品 | 槽位字节 | 物品。
        // 首非末项 HELMET(5) 高位置位 => 0x85；末项 CHESTPLATE(4) 无高位 => 0x04。
        // 因 AIR 物品编码为单字节 0x00，槽位字节实际位于索引 1 与 3。
        let packet = EntityEquipmentPacket {
            entity_id: 0,
            equipments: vec![
                EquipmentEntry {
                    slot: EquipmentSlot::Helmet,
                    item: ItemStack::AIR,
                },
                EquipmentEntry {
                    slot: EquipmentSlot::Chestplate,
                    item: ItemStack::AIR,
                },
            ],
        };
        let mut buf = ByteBuffer::with_capacity(16);
        packet.encode(&mut buf).unwrap();
        let bytes = buf.as_slice();
        assert_eq!(bytes[1], 0x85);
        assert_eq!(bytes[3], 0x04);
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = EntityEquipmentPacket::decode(&mut buf).unwrap();
        assert_eq!(decoded.equipments.len(), 2);
        assert_eq!(decoded.equipments[0].slot, EquipmentSlot::Helmet);
        assert_eq!(decoded.equipments[1].slot, EquipmentSlot::Chestplate);
        assert_eq!(decoded, packet);
    }

    #[test]
    fn window_items_non_air_roundtrip() {
        let packet = WindowItemsPacket {
            window_id: 2,
            state_id: 5,
            items: vec![
                ItemStack::new(264, 1),
                ItemStack::AIR,
                ItemStack::new(1, 64),
            ],
            carried_item: ItemStack::new(264, 3),
        };
        roundtrip(&packet);
    }

    #[test]
    fn set_slot_non_air_roundtrip() {
        roundtrip(&SetSlotPacket {
            window_id: 1,
            state_id: 2,
            slot: -1,
            item: ItemStack::new(264, 16),
        });
    }

    #[test]
    fn entity_equipment_non_air_roundtrip() {
        roundtrip(&EntityEquipmentPacket {
            entity_id: 42,
            equipments: vec![
                EquipmentEntry {
                    slot: EquipmentSlot::MainHand,
                    item: ItemStack::new(264, 1),
                },
                EquipmentEntry {
                    slot: EquipmentSlot::OffHand,
                    item: ItemStack::new(1, 32),
                },
                EquipmentEntry {
                    slot: EquipmentSlot::Boots,
                    item: ItemStack::AIR,
                },
            ],
        });
    }

    #[test]
    fn equipment_slot_legacy_roundtrip() {
        // legacy_id / from_legacy_id 互为逆映射。
        for slot in [
            EquipmentSlot::MainHand,
            EquipmentSlot::OffHand,
            EquipmentSlot::Boots,
            EquipmentSlot::Leggings,
            EquipmentSlot::Chestplate,
            EquipmentSlot::Helmet,
            EquipmentSlot::Body,
            EquipmentSlot::Saddle,
        ] {
            assert_eq!(EquipmentSlot::from_legacy_id(slot.legacy_id()), Some(slot));
        }
        assert_eq!(EquipmentSlot::from_legacy_id(0x7F & 0x7F), None);
    }

    // ---- 库存点击 serverbound 包（见 `.specs/implement-item-click/`）----

    #[test]
    fn click_container_roundtrip_with_items() {
        // 含 changed_slots（钻石 + AIR）与携带钻石的往返一致。
        roundtrip(&ClickContainer {
            window_id: 0,
            state_id: 0,
            slot: 12,
            button: 0, // 左键
            mode: ClickMode::Pickup.as_i32(),
            changed_slots: vec![
                (0, ItemStack::new(264, 32)), // 钻石
                (5, ItemStack::AIR),
            ],
            carried_item: ItemStack::new(264, 1),
        });
    }

    #[test]
    fn click_container_roundtrip_air_carried() {
        // THROW 模式（slot=-999）且光标为 AIR 的往返一致。
        roundtrip(&ClickContainer {
            window_id: 0,
            state_id: 0,
            slot: -999,
            button: 0,
            mode: ClickMode::Throw.as_i32(),
            changed_slots: vec![],
            carried_item: ItemStack::AIR,
        });
    }

    #[test]
    fn click_mode_inverse() {
        // from_i32 / as_i32 互为逆映射；越界与负值返回 None。
        for m in [
            ClickMode::Pickup,
            ClickMode::QuickMove,
            ClickMode::Swap,
            ClickMode::Clone,
            ClickMode::Throw,
            ClickMode::QuickCraft,
            ClickMode::PickupAll,
        ] {
            assert_eq!(ClickMode::from_i32(m.as_i32()), Some(m));
        }
        assert_eq!(ClickMode::from_i32(99), None);
        assert_eq!(ClickMode::from_i32(-1), None);
    }

    #[test]
    fn click_container_decode_truncated() {
        // 只写前三个字段，剩余字段缺失 => 解码返回 Err（UnexpectedEof）。
        let mut buf = ByteBuffer::with_capacity(16);
        buf.put_varint(0); // window_id
        buf.put_varint(0); // state_id
        buf.put_i16(12); // slot；button / mode / changed_slots / carried_item 缺失
        let mut buf = ByteBuffer::new(buf.into_inner());
        assert!(ClickContainer::decode(&mut buf).is_err());
    }

    // ---- 界面/杂项 clientbound 包（T19）----

    #[test]
    fn clientbound_bundle_statistics_roundtrip() {
        roundtrip(&Bundle {
            packets: vec![vec![0x00, 0x01], vec![0x00]],
        });
        roundtrip(&Bundle { packets: vec![] });
        roundtrip(&Statistics {
            entries: vec![(0, 1, 2), (3, 4, 5)],
        });
        roundtrip(&Statistics { entries: vec![] });
    }

    #[test]
    fn clientbound_titles_commands_roundtrip() {
        roundtrip(&ClearTitles { reset: true });
        roundtrip(&ClearTitles { reset: false });
        roundtrip(&DeclareCommands {
            nodes: vec![
                CommandNode {
                    flags: 0x00,
                    children: vec![1, 2],
                    redirect: None,
                    name: None,
                    parser: None,
                    properties: vec![],
                    suggestions_type: None,
                },
                CommandNode {
                    flags: 0x0d, // LITERAL | executable | redirect
                    children: vec![2],
                    redirect: Some(0),
                    name: Some("sub".to_string()),
                    parser: None,
                    properties: vec![],
                    suggestions_type: None,
                },
                CommandNode {
                    flags: 0x16, // ARGUMENT | executable | suggestion
                    children: vec![],
                    redirect: None,
                    name: Some("player".to_string()),
                    parser: Some(ArgumentParserType::STRING),
                    properties: vec![0x00], // string type: single word
                    suggestions_type: Some("minecraft:ask_server".to_string()),
                },
            ],
            root_index: 0,
        });
        roundtrip(&DeclareCommands {
            nodes: vec![],
            root_index: 0,
        });
        roundtrip(&SetTitleSubTitle {
            text: r#"{"text":"sub"}"#.to_string(),
        });
        roundtrip(&SetTitleText {
            text: r#"{"text":"title"}"#.to_string(),
        });
        roundtrip(&SetTitleTime {
            fade_in: 10,
            stay: 60,
            fade_out: 20,
        });
        roundtrip(&ActionBar {
            text: r#"{"text":"bar"}"#.to_string(),
        });
    }

    #[test]
    fn clientbound_window_property_roundtrip() {
        roundtrip(&CloseWindow { window_id: 1 });
        roundtrip(&WindowProperty {
            window_id: 1,
            property: 2,
            value: 3,
        });
        roundtrip(&OpenHorseWindow {
            window_id: 1,
            slot_count: 15,
            entity_id: 42,
        });
        roundtrip(&OpenWindow {
            window_id: 1,
            window_type: 0,
            title: r#"{"text":"Chest"}"#.to_string(),
        });
        roundtrip(&OpenSignEditor {
            position: (10, 64, -5),
        });
        roundtrip(&PlaceGhostRecipe {
            window_id: 1,
            recipe_id: 7,
        });
        roundtrip(&SetCursorItem {
            window_id: 0,
            slot: 0,
            item: ItemStack::new(264, 1),
        });
    }

    #[test]
    fn clientbound_cookie_cooldown_completion_roundtrip() {
        roundtrip(&CookieRequest {
            key: "minecraft:brand".to_string(),
        });
        roundtrip(&SetCooldown {
            cooldown_group: "minecraft:shield".to_string(),
            cooldown_ticks: 20,
        });
        roundtrip(&SetCooldown {
            cooldown_group: String::new(),
            cooldown_ticks: 0,
        });
        roundtrip(&CustomChatCompletion {
            action: 0,
            entries: vec!["a".to_string(), "b".to_string()],
        });
        roundtrip(&CookieStore {
            key: "minecraft:brand".to_string(),
            payload: vec![1, 2, 3],
        });
        roundtrip(&DeleteChat { message_id: 5 });
    }

    #[test]
    fn clientbound_chat_disconnect_roundtrip() {
        roundtrip(&Disconnect {
            reason: r#"{"text":"bye"}"#.to_string(),
        });
        roundtrip(&DisguisedChat {
            message: r#"{"text":"hi"}"#.to_string(),
            chat_type: 1,
            sender_name: "Steve".to_string(),
            target_name: Some("Alex".to_string()),
        });
        roundtrip(&DisguisedChat {
            message: "hi".to_string(),
            chat_type: 2,
            sender_name: "Steve".to_string(),
            target_name: None,
        });
        // 带签名与未签名内容、目标名（msg_type_target）的完整包。
        roundtrip(&PlayerChatMessage {
            global_index: 7,
            sender: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            index: 3,
            signature: Some(vec![0xde, 0xad, 0xbe, 0xef]),
            message_body: SignedMessageBodyPacked {
                timestamp: 1234567890,
                salt: 42,
                content: "hello".to_string(),
            },
            unsigned_content: Some(Component::text("unsigned")),
            filter_mask: FilterMask::Filtered(vec![0x0102_0304_0506_0708]),
            msg_type_id: 1,
            msg_type_name: Component::Translatable {
                key: "chat.type.text".to_string(),
                fallback: None,
                args: vec![],
            },
            msg_type_target: Some(Component::text("@s")),
        });
        // 无签名、无未签名内容、无目标名（全部可选字段缺席）。
        roundtrip(&PlayerChatMessage {
            global_index: 0,
            sender: Uuid::nil(),
            index: 0,
            signature: None,
            message_body: SignedMessageBodyPacked {
                timestamp: 0,
                salt: 0,
                content: "plain".to_string(),
            },
            unsigned_content: None,
            filter_mask: FilterMask::PassThrough,
            msg_type_id: 0,
            msg_type_name: Component::text("chat"),
            msg_type_target: None,
        });
        roundtrip(&PlayerListHeaderAndFooter {
            header: r#"{"text":"head"}"#.to_string(),
            footer: r#"{"text":"foot"}"#.to_string(),
        });
        roundtrip(&DeathCombatEvent {
            player_id: 42,
            message: Component::text("died"),
        });
        roundtrip(&DeathCombatEvent {
            player_id: 0,
            message: Component::Translatable {
                key: "death.attack.generic".to_string(),
                fallback: Some("%s died".to_string()),
                args: vec![Component::text("Steve")],
            },
        });
    }

    #[test]
    fn clientbound_debug_roundtrip() {
        roundtrip(&DebugBlockValue {
            payload: vec![1, 2, 3],
        });
        roundtrip(&DebugChunkValue {
            payload: vec![0xaa, 0xbb],
        });
        roundtrip(&DebugEntityValue {
            payload: vec![0xff],
        });
        roundtrip(&DebugEvent {
            event: 1,
            payload: vec![1, 2],
        });
        roundtrip(&DebugSample {
            sample_type: 0,
            data: vec![1, 2, 3],
        });
        roundtrip(&DebugSample {
            sample_type: 1,
            data: vec![],
        });
    }

    #[test]
    fn clientbound_game_test_and_border_roundtrip() {
        roundtrip(&GameTestHighlightPos {
            position: (1, 64, 2),
            solid: true,
            red: 255,
            green: 0,
            blue: 0,
            alpha: 128,
        });
        roundtrip(&InitializeWorldBorder {
            x: 0.0,
            z: 0.0,
            old_diameter: 60000000.0,
            new_diameter: 1000.0,
            speed: 5,
            portal_teleport_boundary: 29999984,
            warning_blocks: 5,
            warning_time: 15,
        });
        roundtrip(&WorldBorderCenter { x: 10.5, z: -20.25 });
        roundtrip(&WorldBorderLerpSize {
            old_diameter: 100.0,
            new_diameter: 200.0,
            speed: 3,
        });
        roundtrip(&WorldBorderSize { diameter: 300.0 });
        roundtrip(&WorldBorderWarningDelay { warning_time: 15 });
        roundtrip(&WorldBorderWarningReach { warning_blocks: 5 });
        roundtrip(&Camera { camera_id: 7 });
    }

    #[test]
    fn clientbound_spawn_and_movement_roundtrip() {
        roundtrip(&SpawnPosition {
            position: (12, 64, 20),
        });
        roundtrip(&FacePlayer {
            feet_or_eyes: 1,
            target_x: 1.0,
            target_y: 2.0,
            target_z: 3.0,
            is_entity: false,
            entity_id: None,
        });
        roundtrip(&FacePlayer {
            feet_or_eyes: 0,
            target_x: 1.0,
            target_y: 2.0,
            target_z: 3.0,
            is_entity: true,
            entity_id: Some(42),
        });
        roundtrip(&PlayerRotation {
            yaw: 90.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 7,
        });
        roundtrip(&MoveMinecart {
            entity_id: 3,
            x: 1.0,
            y: 64.0,
            z: 2.0,
        });
        roundtrip(&ClientboundVehicleMove {
            x: 1.0,
            y: 64.0,
            z: 2.0,
            yaw: 90.0,
            pitch: 0.0,
        });
        roundtrip(&OpenBook { hand: 0 });
    }

    #[test]
    fn clientbound_respawn_roundtrip() {
        roundtrip(&Respawn {
            dimension: "minecraft:overworld".to_string(),
            world_name: "minecraft:overworld".to_string(),
            hashed_seed: 0x1234_5678_9abc_def0,
            game_mode: 0,
            previous_game_mode: 255,
            is_debug: false,
            is_flat: false,
            copy_metadata: true,
            death: None,
        });
        roundtrip(&Respawn {
            dimension: "minecraft:the_nether".to_string(),
            world_name: "minecraft:the_nether".to_string(),
            hashed_seed: 0,
            game_mode: 1,
            previous_game_mode: 0,
            is_debug: false,
            is_flat: false,
            copy_metadata: false,
            death: Some(GlobalPos {
                dimension: "minecraft:overworld".to_string(),
                position: (12i64 << 38) | (20i64 << 12) | 64,
            }),
        });
    }

    #[test]
    fn clientbound_ping_keepalive_roundtrip() {
        roundtrip(&ClientboundPing { id: 42 });
        roundtrip(&ClientboundPingResponse { id: 42 });
        roundtrip(&ClientboundKeepAlive {
            keep_alive_id: 1000,
        });
        roundtrip(&ClientboundPlayerAbilities {
            flags: 0x07,
            flying_speed: 0.05,
            field_of_view_modifier: 0.1,
        });
        roundtrip(&Transfer {
            host: "localhost".to_string(),
            port: 25565,
        });
        roundtrip(&StartConfiguration);
    }

    #[test]
    fn clientbound_ping_keepalive_packet_ids() {
        assert_eq!(ClientboundPing { id: 0 }.packet_id(), 0x3b);
        assert_eq!(ClientboundPingResponse { id: 0 }.packet_id(), 0x3c);
        assert_eq!(ClientboundKeepAlive { keep_alive_id: 0 }.packet_id(), 0x2b);
        assert_eq!(
            ClientboundPlayerAbilities {
                flags: 0,
                flying_speed: 0.0,
                field_of_view_modifier: 0.0,
            }
            .packet_id(),
            0x3e
        );
        assert_eq!(
            ClientboundTabComplete {
                transaction_id: 0,
                start: 0,
                length: 0,
                matches: vec![],
            }
            .packet_id(),
            0x0f
        );
    }

    #[test]
    fn clientbound_tab_complete_plugin_roundtrip() {
        roundtrip(&ClientboundTabComplete {
            transaction_id: 1,
            start: 0,
            length: 5,
            matches: vec![
                (
                    "minecraft:stone".to_string(),
                    true,
                    Some(r#"{"text":"t"}"#.to_string()),
                ),
                ("minecraft:dirt".to_string(), false, None),
            ],
        });
        roundtrip(&ClientboundTabComplete {
            transaction_id: 2,
            start: 1,
            length: 0,
            matches: vec![],
        });
        roundtrip(&ClientboundPluginMessage {
            channel: "minecraft:brand".to_string(),
            data: vec![0x00, 0x01],
        });
    }

    #[test]
    fn clientbound_map_trade_roundtrip() {
        roundtrip(&MapData {
            map_id: 1,
            scale: 2,
            locked: false,
            columns: 3,
            rows: 4,
            data: vec![0x00, 0x01, 0x02],
        });
        roundtrip(&TradeList {
            window_id: 1,
            trades: vec![0x00, 0x01],
            villager_level: 2,
            experience: 100,
            is_regular_villager: true,
            can_restock: true,
        });
        roundtrip(&TradeList {
            window_id: 0,
            trades: vec![],
            villager_level: 0,
            experience: 0,
            is_regular_villager: false,
            can_restock: false,
        });
    }

    #[test]
    fn clientbound_combat_events_roundtrip() {
        assert_eq!(EnterCombatEvent.packet_id(), 0x41);
        roundtrip(&EnterCombatEvent);
        roundtrip(&EndCombatEvent {
            duration: 100,
            killer_id: 42,
        });
    }

    #[test]
    fn clientbound_resource_pack_roundtrip() {
        roundtrip(&ResourcePackPush {
            uuid: Uuid::from_u128(1),
            url: "https://example.com/pack.zip".to_string(),
            hash: "0123456789abcdef0123456789abcdef01234567".to_string(),
            required: true,
            prompt: Some(r#"{"text":"install"}"#.to_string()),
        });
        roundtrip(&ResourcePackPush {
            uuid: Uuid::nil(),
            url: "https://example.com/pack.zip".to_string(),
            hash: String::new(),
            required: false,
            prompt: None,
        });
        roundtrip(&ResourcePackPop {
            uuid: Some(Uuid::from_u128(2)),
        });
        roundtrip(&ResourcePackPop { uuid: None });
    }

    #[test]
    fn clientbound_misc_server_roundtrip() {
        roundtrip(&ServerDifficulty {
            difficulty: 2,
            locked: false,
        });
        roundtrip(&ProjectilePower {
            entity_id: 5,
            power: 1.5,
        });
        roundtrip(&CustomReportDetails {
            details: vec![("key".to_string(), "value".to_string())],
        });
        roundtrip(&CustomReportDetails { details: vec![] });
        roundtrip(&ServerLinks {
            links: vec![(true, "https://minecraft.net".to_string())],
        });
        roundtrip(&ServerLinks { links: vec![] });
        roundtrip(&SelectAdvancementTab {
            tab_id: Some("minecraft:story".to_string()),
        });
        roundtrip(&SelectAdvancementTab { tab_id: None });
    }

    #[test]
    fn clientbound_waypoint_test_status_roundtrip() {
        roundtrip(&TrackedWaypoint {
            waypoint_id: 1,
            tracking: true,
            name: Some("home".to_string()),
            position: Some((10, 64, 20)),
        });
        roundtrip(&TrackedWaypoint {
            waypoint_id: 2,
            tracking: false,
            name: None,
            position: None,
        });
        roundtrip(&TestInstanceBlockStatus {
            status: 0,
            position: (1, 64, 2),
            error_message: Some("boom".to_string()),
        });
        roundtrip(&TestInstanceBlockStatus {
            status: 1,
            position: (1, 64, 2),
            error_message: None,
        });
        roundtrip(&NbtQueryResponse {
            transaction_id: 7,
            nbt: Some(vec![0x0a, 0x00]),
        });
        roundtrip(&NbtQueryResponse {
            transaction_id: 8,
            nbt: None,
        });
    }

    // ---- 计分板/进度/配方/对话框/BossBar clientbound 包（T20）----

    #[test]
    fn clientbound_scoreboard_roundtrip() {
        roundtrip(&ScoreboardObjective {
            objective_name: "sb".to_string(),
            action: 0,
            display_name: r#"{"text":"SB"}"#.to_string(),
            objective_type: 0,
        });
        roundtrip(&DisplayScoreboard {
            position: 1,
            objective_name: "sb".to_string(),
        });
        roundtrip(&UpdateScore {
            entity_name: "Steve".to_string(),
            action: 0,
            objective_name: "sb".to_string(),
            value: Some(10),
        });
        roundtrip(&UpdateScore {
            entity_name: "Steve".to_string(),
            action: 1,
            objective_name: "sb".to_string(),
            value: None,
        });
        roundtrip(&ResetScore {
            entity_name: "Steve".to_string(),
            has_objective: true,
            objective_name: Some("sb".to_string()),
        });
        roundtrip(&ResetScore {
            entity_name: "Steve".to_string(),
            has_objective: false,
            objective_name: None,
        });
        roundtrip(&Teams {
            team_name: "red".to_string(),
            action: 0,
            display_name: "Red".to_string(),
            prefix: "[R]".to_string(),
            suffix: "[/R]".to_string(),
            color: 1,
            members: vec!["Steve".to_string(), "Alex".to_string()],
        });
    }

    #[test]
    fn clientbound_boss_bar_roundtrip() {
        roundtrip(&BossBar {
            uuid: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            action: BossBarAction::Add {
                title: r#"{"text":"Dragon"}"#.to_string(),
                health: 1.0,
                color: 0,
                division: 0,
                flags: 0x03,
            },
        });
        roundtrip(&BossBar {
            uuid: Uuid::from_u128(1),
            action: BossBarAction::Remove,
        });
        roundtrip(&BossBar {
            uuid: Uuid::from_u128(2),
            action: BossBarAction::UpdateHealth(0.5),
        });
        roundtrip(&BossBar {
            uuid: Uuid::from_u128(3),
            action: BossBarAction::UpdateTitle(r#"{"text":"New"}"#.to_string()),
        });
        roundtrip(&BossBar {
            uuid: Uuid::from_u128(4),
            action: BossBarAction::UpdateStyle {
                color: 2,
                division: 1,
            },
        });
        roundtrip(&BossBar {
            uuid: Uuid::from_u128(5),
            action: BossBarAction::UpdateFlags(0x01),
        });
    }

    #[test]
    fn clientbound_advancements_recipes_roundtrip() {
        roundtrip(&Advancements {
            clear: false,
            advancements: vec![(
                "minecraft:story/root".to_string(),
                None,
                vec!["impossible".to_string()],
            )],
            removed: vec!["minecraft:story/old".to_string()],
        });
        roundtrip(&Advancements {
            clear: true,
            advancements: vec![],
            removed: vec![],
        });
        roundtrip(&DeclareRecipes {
            item_properties: vec![
                (RecipeProperty::FurnaceInput(None), vec![1, 2]),
                (RecipeProperty::SmithingTemplate(None), vec![650]),
            ],
            stonecutter_recipes: vec![StonecutterRecipe {
                ingredient: Ingredient::Items(vec![1]),
                option_display: SlotDisplay::ItemStack(ItemStack::new(44, 2)),
            }],
        });
        roundtrip(&DeclareRecipes {
            item_properties: vec![],
            stonecutter_recipes: vec![],
        });
        roundtrip(&RecipeBookAdd {
            entries: vec![
                (
                    0,
                    RecipeDisplay::Stonecutter {
                        ingredient: SlotDisplay::Item(1),
                        result: SlotDisplay::Item(44),
                        crafting_station: SlotDisplay::Item(449),
                    },
                ),
                (
                    5,
                    RecipeDisplay::Furnace {
                        ingredient: SlotDisplay::Item(15),
                        fuel: SlotDisplay::Item(263),
                        result: SlotDisplay::Item(265),
                        crafting_station: SlotDisplay::Item(61),
                        duration: 200,
                        experience: 0.7,
                    },
                ),
            ],
            replace: true,
        });
        roundtrip(&RecipeBookAdd {
            entries: vec![],
            replace: false,
        });
        roundtrip(&RecipeBookRemove {
            recipe_ids: vec!["minecraft:stick".to_string()],
        });
        roundtrip(&RecipeBookRemove { recipe_ids: vec![] });
        roundtrip(&RecipeBookSettings {
            crafting_open: true,
            crafting_filter: false,
            smelting_open: false,
            smelting_filter: true,
            blast_furnace_open: true,
            blast_furnace_filter: true,
            smoker_open: false,
            smoker_filter: false,
        });
    }

    #[test]
    fn clientbound_dialog_roundtrip() {
        roundtrip(&ShowDialog {
            dialog_id: Uuid::from_u128(0xabcdef),
            display_name: Component::text("Dialog"),
            dialog_type: 0,
            actions: vec![(0, "Confirm".to_string(), Some("Click me".to_string()))],
        });
        roundtrip(&ShowDialog {
            dialog_id: Uuid::nil(),
            display_name: Component::Translatable {
                key: "dialog.title".to_string(),
                fallback: Some("Plain".to_string()),
                args: vec![],
            },
            dialog_type: 1,
            actions: vec![],
        });
        roundtrip(&ClearDialog {
            dialog_id: Uuid::from_u128(0x1234),
        });
    }

    // ---- T6 复杂包真实化：命令节点属性提取 / 过滤掩码 / 畸形数据 ----

    #[test]
    fn declare_commands_node_properties_min_max_roundtrip() {
        // INTEGER：min/max 位掩码（flags 0x01=min、0x02=max）+ 各 4 字节大端值。
        let mut min_max = Vec::new();
        min_max.push(0x03u8);
        min_max.extend_from_slice(&(-10i32).to_be_bytes());
        min_max.extend_from_slice(&10i32.to_be_bytes());
        let node = CommandNode {
            flags: 0x02, // ARGUMENT
            children: vec![],
            redirect: None,
            name: Some("count".to_string()),
            parser: Some(ArgumentParserType::INTEGER),
            properties: min_max.clone(),
            suggestions_type: None,
        };
        let mut raw = ByteBuffer::with_capacity(32);
        node.encode(&mut raw).unwrap();
        let mut buf = ByteBuffer::new(raw.into_inner());
        let decoded = CommandNode::decode(&mut buf).unwrap();
        assert_eq!(decoded.properties, min_max);
        assert_eq!(decoded, node);

        // min 仅 / max 仅的位掩码变体，properties 提取应与输入字节一致。
        for (flag, val) in [(0x01u8, 5i32), (0x02u8, 7i32)] {
            let mut props = vec![flag];
            props.extend_from_slice(&val.to_be_bytes());
            let n = CommandNode {
                flags: 0x02,
                children: vec![],
                redirect: None,
                name: Some("n".to_string()),
                parser: Some(ArgumentParserType::INTEGER),
                properties: props.clone(),
                suggestions_type: None,
            };
            let mut b = ByteBuffer::with_capacity(16);
            n.encode(&mut b).unwrap();
            let mut b = ByteBuffer::new(b.into_inner());
            let back = CommandNode::decode(&mut b).unwrap();
            assert_eq!(back.properties, props);
        }
    }

    #[test]
    fn declare_commands_malformed_redirect_truncated_is_eof() {
        // 声明 HAS_REDIRECT(0x08) 但截断（无 redirect VarInt）→ UnexpectedEof，不 panic。
        let mut raw = ByteBuffer::with_capacity(16);
        raw.put_varint(1); // 1 个节点
        raw.put_i8(0x08); // ROOT | redirect
        raw.put_varint(0); // children 空
        // 缺 redirect varint
        raw.put_varint(0); // root_index
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            DeclareCommands::decode(&mut buf),
            Err(ProtocolError::UnexpectedEof)
        );
    }

    #[test]
    fn declare_commands_unknown_parser_id_rejected() {
        // ARGUMENT 节点携带未知 parser id（999）→ InvalidValue，不 panic。
        let mut raw = ByteBuffer::with_capacity(32);
        raw.put_varint(1); // 1 个节点
        raw.put_i8(0x02); // ARGUMENT
        raw.put_varint(0); // children 空
        raw.put_string("x");
        raw.put_varint(999); // 未知 parser id
        raw.put_varint(0); // root_index
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            DeclareCommands::decode(&mut buf),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn filter_mask_all_variants_roundtrip() {
        // PassThrough：无可选字段。
        roundtrip(&PlayerChatMessage {
            global_index: 0,
            sender: Uuid::nil(),
            index: 0,
            signature: Some(vec![1, 2, 3]),
            message_body: SignedMessageBodyPacked {
                timestamp: 1,
                salt: 2,
                content: "m".to_string(),
            },
            unsigned_content: None,
            filter_mask: FilterMask::PassThrough,
            msg_type_id: 0,
            msg_type_name: Component::text("chat"),
            msg_type_target: None,
        });
        // Filtered：位掩码数组。
        roundtrip(&PlayerChatMessage {
            global_index: 0,
            sender: Uuid::nil(),
            index: 0,
            signature: None,
            message_body: SignedMessageBodyPacked {
                timestamp: 1,
                salt: 2,
                content: "m".to_string(),
            },
            unsigned_content: Some(Component::text("u")),
            filter_mask: FilterMask::Filtered(vec![0, u64::from_be_bytes([0xff; 8])]),
            msg_type_id: 0,
            msg_type_name: Component::text("chat"),
            msg_type_target: Some(Component::text("@e")),
        });
        // FullyFiltered。
        roundtrip(&PlayerChatMessage {
            global_index: 0,
            sender: Uuid::nil(),
            index: 0,
            signature: None,
            message_body: SignedMessageBodyPacked {
                timestamp: 1,
                salt: 2,
                content: "m".to_string(),
            },
            unsigned_content: None,
            filter_mask: FilterMask::FullyFiltered,
            msg_type_id: 0,
            msg_type_name: Component::text("chat"),
            msg_type_target: None,
        });
    }

    #[test]
    fn player_chat_message_truncated_is_eof() {
        // 完整包体截断（只给前两个字段）→ UnexpectedEof，不 panic。
        let mut raw = ByteBuffer::with_capacity(16);
        raw.put_varint(0); // global_index
        raw.put_uuid(Uuid::nil()); // sender
        // 缺剩余字段
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            PlayerChatMessage::decode(&mut buf),
            Err(ProtocolError::UnexpectedEof)
        );
    }

    #[test]
    fn recipe_book_and_declare_recipes_malformed() {
        // RecipeBookSettings 只给 4 个 Bool → UnexpectedEof。
        let mut raw = ByteBuffer::with_capacity(8);
        for _ in 0..4 {
            raw.put_bool(true);
        }
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            RecipeBookSettings::decode(&mut buf),
            Err(ProtocolError::UnexpectedEof)
        );
        // DeclareRecipes 声明 1 条属性但截断（缺 material 计数）→ EOF。
        let mut raw = ByteBuffer::with_capacity(16);
        raw.put_varint(1); // 1 条属性
        raw.put_string("furnace_input");
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert!(DeclareRecipes::decode(&mut buf).is_err());
        // 未知 RecipeProperty key → InvalidValue。
        let mut raw = ByteBuffer::with_capacity(16);
        raw.put_varint(1);
        raw.put_string("not_a_real_category");
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            DeclareRecipes::decode(&mut buf),
            Err(ProtocolError::InvalidValue)
        );
        // RecipeBookAdd 声明 1 条但缺 RecipeDisplay → EOF。
        let mut raw = ByteBuffer::with_capacity(16);
        raw.put_varint(1); // 1 条 entry
        raw.put_varint(0); // display_id
        // 缺 display
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert!(RecipeBookAdd::decode(&mut buf).is_err());
    }

    #[test]
    fn set_cooldown_death_combat_show_dialog_malformed() {
        // SetCooldown 只给 cooldown_group → UnexpectedEof。
        let mut raw = ByteBuffer::with_capacity(16);
        raw.put_string("minecraft:shield");
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            SetCooldown::decode(&mut buf),
            Err(ProtocolError::UnexpectedEof)
        );
        // DeathCombatEvent 组件前导非法（TAG_END）→ InvalidValue。
        let mut raw = ByteBuffer::with_capacity(8);
        raw.put_varint(42);
        raw.put_u8(0x00);
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            DeathCombatEvent::decode(&mut buf),
            Err(ProtocolError::InvalidValue)
        );
        // ShowDialog 组件前导不是 0x0a → InvalidValue。
        let mut raw = ByteBuffer::with_capacity(32);
        raw.put_uuid(Uuid::nil());
        raw.put_u8(0x09);
        raw.put_varint(0);
        raw.put_varint(0); // actions 空
        let mut buf = ByteBuffer::new(raw.into_inner());
        assert_eq!(
            ShowDialog::decode(&mut buf),
            Err(ProtocolError::InvalidValue)
        );
    }
}
