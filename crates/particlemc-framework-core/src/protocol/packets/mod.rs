// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 各连接状态的最小真实数据包（1.21.11）与入站包统一枚举。

pub mod configuration;
pub mod handshake;
pub mod login;
pub mod play;
pub mod status;

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::packet::Packet;

pub use configuration::{
    ClientInformation, ConfigDisconnect, FeatureFlags, FinishConfigurationC2S,
    FinishConfigurationS2C, PluginMessage, RegistryData, UpdateTags,
};
pub use handshake::Intention;
pub use login::{
    Hello, LoginChallenge, LoginCompression, LoginDisconnect, LoginHelloResponse, LoginSuccess,
};
pub use play::{
    // ---- T19 界面/杂项 ----
    ActionBar,
    // ---- T20 计分板/进度/配方/对话框/BossBar ----
    Advancements,
    BossBar,
    BossBarAction,
    Bundle,
    Camera,
    ChunkBatchFinished,
    ChunkBatchReceived,
    ChunkBatchStart,
    ClearDialog,
    ClearTitles,
    ClickContainer,
    ClickMode,
    ClientCommandChatPacket,
    ClientHeldItemChange,
    ClientSignedCommandChatPacket,
    ClientboundKeepAlive,
    ClientboundPing,
    ClientboundPingResponse,
    ClientboundPlayerAbilities,
    ClientboundPluginMessage,
    ClientboundTabComplete,
    ClientboundVehicleMove,
    CloseContainer,
    CloseWindow,
    CommandNode,
    CookieRequest,
    CookieStore,
    CustomChatCompletion,
    CustomReportDetails,
    DeathCombatEvent,
    DebugBlockValue,
    DebugChunkValue,
    DebugEntityValue,
    DebugEvent,
    DebugSample,
    DeclareCommands,
    DeclareRecipes,
    DeleteChat,
    DestroyEntities,
    Disconnect,
    DisguisedChat,
    DisplayScoreboard,
    EndCombatEvent,
    EnterCombatEvent,
    EntityEquipmentPacket,
    EntityMetaData,
    EntityTeleport,
    EquipmentEntry,
    EquipmentSlot,
    FacePlayer,
    FilterMask,
    GameStateChange,
    GameTestHighlightPos,
    Heightmap,
    HeldItemChange,
    InitializeWorldBorder,
    KeepAlive,
    Login,
    Look,
    MapChunk,
    MapData,
    MoveMinecart,
    NbtQueryResponse,
    OpenBook,
    OpenHorseWindow,
    OpenSignEditor,
    OpenWindow,
    PlaceGhostRecipe,
    PlayerChatMessage,
    PlayerInfo,
    PlayerListHeaderAndFooter,
    PlayerLoaded,
    PlayerPosition,
    PlayerPositionAndRotation,
    PlayerPositionStatus,
    PlayerRemove,
    PlayerRotation,
    Position,
    ProjectilePower,
    RecipeBookAdd,
    RecipeBookRemove,
    RecipeBookSettings,
    RelEntityMove,
    ResetScore,
    ResourcePackPop,
    ResourcePackPush,
    Respawn,
    ScoreboardObjective,
    SelectAdvancementTab,
    ServerDifficulty,
    ServerLinks,
    SetCooldown,
    SetCursorItem,
    SetSlotPacket,
    SetTitleSubTitle,
    SetTitleText,
    SetTitleTime,
    ShowDialog,
    SignedMessageBodyPacked,
    SpawnEntity,
    SpawnInfo,
    SpawnPosition,
    StartConfiguration,
    Statistics,
    Status,
    SystemChatPacket,
    Teams,
    TeleportConfirm,
    TestInstanceBlockStatus,
    TrackedWaypoint,
    TradeList,
    Transfer,
    UpdateHealth,
    UpdateScore,
    WindowItemsPacket,
    WorldBorderCenter,
    WorldBorderLerpSize,
    WorldBorderSize,
    WorldBorderWarningDelay,
    WorldBorderWarningReach,
};
pub use status::{Ping, PingResponse, StatusRequest, StatusResponse};

/// 玩家属性（皮肤 / 皮肤签名）。被 `LoginSuccess` 与 Velocity 转发共用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// 属性名（如 `textures`）。
    pub name: String,
    /// 属性值（base64 编码的 JSON）。
    pub value: String,
    /// 属性签名（由 Minecraft/Yggdrasil 或代理用其私钥签署）。
    pub signature: Option<String>,
}

/// 入站（serverbound）数据包统一枚举。
#[derive(Debug, Clone)]
pub enum InboundPacket {
    /// 握手意图。
    Intention(Intention),
    /// 状态请求。
    StatusRequest,
    /// 状态 Ping。
    Ping(Ping),
    /// 登录 Hello。
    Hello(Hello),
    /// 登录挑战响应（在线模式，客户端回传加密的共享密钥与验证 token）。
    LoginChallenge(LoginChallenge),
    /// 客户端信息（配置阶段首个包，触发服务端发送 FinishConfiguration）。
    ClientInformation(crate::protocol::packets::configuration::ClientInformation),
    /// 登录确认（客户端收到 LoginSuccess 后发出，触发进入配置）。
    LoginAcknowledged,
    /// 配置完成确认（客户端发出，触发进入游玩）。
    FinishConfiguration,
    /// 确认传送（Play, id 0x00）。
    TeleportConfirm(play::TeleportConfirm),
    /// 区块批接收确认（Play, id 0x0a）。
    ChunkBatchReceived(play::ChunkBatchReceived),
    /// 客户端状态（Play, id 0x0b，wire 名 `client_status`）：请求重生 / 统计。
    ClientStatus(play::Status),
    /// 命令聊天（Play, id 0x06）：含 "/" 前缀命令或无前缀指令文本，见 `.specs/implement-command-framework/`。
    ClientCommandChat(play::ClientCommandChatPacket),
    /// 签名命令聊天（Play, id 0x07）：签名版命令文本，见 `.specs/implement-command-framework/`。
    ClientSignedCommandChat(play::ClientSignedCommandChatPacket),
    /// 心跳回复（Play, id 0x1b）。
    KeepAlive(play::KeepAlive),
    /// 玩家移动（Play, id 0x1d，wire 名 position）。
    PlayerPosition(play::PlayerPosition),
    /// 玩家移动 + 旋转（Play, id 0x1e，wire 名 position_look）。
    PlayerPositionAndRotation(play::PlayerPositionAndRotation),
    /// 玩家旋转（Play, id 0x1f，wire 名 look）。
    Look(play::Look),
    /// 玩家地面状态（Play, id 0x20，wire 名 flying）。
    PlayerPositionStatus(play::PlayerPositionStatus),
    /// 客户端已加载完成（Play, id 0x2b，wire 名 player_loaded）。
    PlayerLoaded(play::PlayerLoaded),
    /// 容器点击（Play, id 0x11）：权威重算玩家库存。
    /// 见 `.specs/implement-item-click/`。
    ClickContainer(ClickContainer),
    /// 关闭容器（Play, id 0x12）：清空玩家光标（见 `.specs/implement-item-inventory/`）。
    CloseContainer(play::CloseContainer),
    /// 手持物品切换（Play, id 0x34）：serverbound `ClientHeldItemChange`，服务端接受后回发 clientbound `HeldItemChange`（见 `.specs/implement-item-inventory/`）。
    HeldItemChange(play::ClientHeldItemChange),
    /// 客户端包（Play, id 0x01）：QueryBlockNbt，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    QueryBlockNbt(play::QueryBlockNbt),
    /// 客户端包（Play, id 0x02）：SelectBundleItem，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    SelectBundleItem(play::SelectBundleItem),
    /// 客户端包（Play, id 0x03）：ChangeDifficulty，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ChangeDifficulty(play::ChangeDifficulty),
    /// 客户端包（Play, id 0x04）：ChangeGameMode，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ChangeGameMode(play::ChangeGameMode),
    /// 客户端包（Play, id 0x05）：ChatAck，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ChatAck(play::ChatAck),
    /// 客户端包（Play, id 0x08）：ChatMessage，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ChatMessage(play::ChatMessage),
    /// 客户端包（Play, id 0x09）：ChatSessionUpdate，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ChatSessionUpdate(play::ChatSessionUpdate),
    /// 客户端包（Play, id 0x0c）：TickEnd，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    TickEnd(play::TickEnd),
    /// 客户端包（Play, id 0x0d）：Settings，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    Settings(play::Settings),
    /// 客户端包（Play, id 0x0e）：TabComplete，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    TabComplete(play::TabComplete),
    /// 客户端包（Play, id 0x0f）：ConfigurationAck，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ConfigurationAck(play::ConfigurationAck),
    /// 客户端包（Play, id 0x10）：ClickWindowButton，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ClickWindowButton(play::ClickWindowButton),
    /// 客户端包（Play, id 0x13）：WindowSlotState，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    WindowSlotState(play::WindowSlotState),
    /// 客户端包（Play, id 0x14）：CookieResponse，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    CookieResponse(play::CookieResponse),
    /// 客户端包（Play, id 0x15）：ClientPluginMessage，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ClientPluginMessage(play::ClientPluginMessage),
    /// 客户端包（Play, id 0x16）：DebugSubscriptionRequest，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    DebugSubscriptionRequest(play::DebugSubscriptionRequest),
    /// 客户端包（Play, id 0x17）：EditBook，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    EditBook(play::EditBook),
    /// 客户端包（Play, id 0x18）：QueryEntityNbt，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    QueryEntityNbt(play::QueryEntityNbt),
    /// 客户端包（Play, id 0x19）：InteractEntity，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    InteractEntity(play::InteractEntity),
    /// 客户端包（Play, id 0x1a）：GenerateStructure，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    GenerateStructure(play::GenerateStructure),
    /// 客户端包（Play, id 0x1c）：LockDifficulty，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    LockDifficulty(play::LockDifficulty),
    /// 客户端包（Play, id 0x21）：VehicleMove，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    VehicleMove(play::VehicleMove),
    /// 客户端包（Play, id 0x22）：SteerBoat，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    SteerBoat(play::SteerBoat),
    /// 客户端包（Play, id 0x23）：PickItemFromBlock，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PickItemFromBlock(play::PickItemFromBlock),
    /// 客户端包（Play, id 0x24）：PickItemFromEntity，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PickItemFromEntity(play::PickItemFromEntity),
    /// 客户端包（Play, id 0x25）：PingRequest，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PingRequest(play::PingRequest),
    /// 客户端包（Play, id 0x26）：PlaceRecipe，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PlaceRecipe(play::PlaceRecipe),
    /// 客户端包（Play, id 0x27）：PlayerAbilities，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PlayerAbilities(play::PlayerAbilities),
    /// 客户端包（Play, id 0x28）：PlayerAction，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PlayerAction(play::PlayerAction),
    /// 客户端包（Play, id 0x29）：EntityAction，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    EntityAction(play::EntityAction),
    /// 客户端包（Play, id 0x2a）：Input，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    Input(play::Input),
    /// 客户端包（Play, id 0x2c）：Pong，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    Pong(play::Pong),
    /// 客户端包（Play, id 0x2d）：SetRecipeBookState，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    SetRecipeBookState(play::SetRecipeBookState),
    /// 客户端包（Play, id 0x2e）：RecipeBookSeenRecipe，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    RecipeBookSeenRecipe(play::RecipeBookSeenRecipe),
    /// 客户端包（Play, id 0x2f）：NameItem，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    NameItem(play::NameItem),
    /// 客户端包（Play, id 0x30）：ResourcePackStatus，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    ResourcePackStatus(play::ResourcePackStatus),
    /// 客户端包（Play, id 0x31）：AdvancementTab，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    AdvancementTab(play::AdvancementTab),
    /// 客户端包（Play, id 0x32）：SelectTrade，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    SelectTrade(play::SelectTrade),
    /// 客户端包（Play, id 0x33）：SetBeaconEffect，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    SetBeaconEffect(play::SetBeaconEffect),
    /// 客户端包（Play, id 0x35）：UpdateCommandBlock，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    UpdateCommandBlock(play::UpdateCommandBlock),
    /// 客户端包（Play, id 0x36）：UpdateCommandBlockMinecart，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    UpdateCommandBlockMinecart(play::UpdateCommandBlockMinecart),
    /// 客户端包（Play, id 0x37）：CreativeInventoryAction，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    CreativeInventoryAction(play::CreativeInventoryAction),
    /// 客户端包（Play, id 0x38）：UpdateJigsawBlock，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    UpdateJigsawBlock(play::UpdateJigsawBlock),
    /// 客户端包（Play, id 0x39）：UpdateStructureBlock，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    UpdateStructureBlock(play::UpdateStructureBlock),
    /// 客户端包（Play, id 0x3a）：SetTestBlock，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    SetTestBlock(play::SetTestBlock),
    /// 客户端包（Play, id 0x3b）：UpdateSign，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    UpdateSign(play::UpdateSign),
    /// 客户端包（Play, id 0x3c）：Animation，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    Animation(play::Animation),
    /// 客户端包（Play, id 0x3d）：Spectate，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    Spectate(play::Spectate),
    /// 客户端包（Play, id 0x3e）：TestInstanceBlockAction，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    TestInstanceBlockAction(play::TestInstanceBlockAction),
    /// 客户端包（Play, id 0x3f）：PlayerBlockPlacement，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    PlayerBlockPlacement(play::PlayerBlockPlacement),
    /// 客户端包（Play, id 0x40）：UseItem，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    UseItem(play::UseItem),
    /// 客户端包（Play, id 0x41）：CustomClickAction，本框架当前仅解码不处理（见 `.specs/implement-framework-capabilities/`）。
    CustomClickAction(play::CustomClickAction),
    /// 已知但本框架当前不解析其包体的包（直接忽略）。
    Ignored { packet_id: i32 },
}

/// 将客户端包（实现 [`Packet`]）编码为完整帧负载（packet_id + 包体）。
pub fn encode_clientbound<P: Packet>(packet: &P) -> Vec<u8> {
    let mut buf = ByteBuffer::with_capacity(64);
    buf.put_varint(packet.packet_id());
    match packet.encode(&mut buf) {
        Ok(()) => buf.into_inner(),
        Err(_) => Vec::new(),
    }
}
