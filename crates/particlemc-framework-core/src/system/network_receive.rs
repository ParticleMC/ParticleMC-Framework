// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! tick 管线首段：消费 `NetworkBridge.inbound` 入站帧，按协议状态机推进连接。
//!
//! 处理流程（1.21.11 / protocol 774 离线 / Velocity 转发）：
//! Handshake → Status(StatusRequest/Ping) | Login(Hello → LoginSuccess →
//! LoginAcknowledged) → Configuration(ClientInformation → [RegistryData×N +
//! UpdateTags + FeatureFlags] → FinishConfiguration S2C) → Play(FinishConfiguration
//! C2S → Login(play) + Position + UpdateHealth + PlayerInfo → EnterPlayEvent)。
//! 登录时生成玩家实体；移动包派生为 `NetworkEvent::PlayerMoveRaw` 供 `player_input`
//! 消费；进入 Play 时写入 `EnterPlayEvent` 供 `chunk_send` / `entity_sync` 消费。

use crate::crypto::OnlineAuthContext;
use crate::prelude::{Commands, MessageWriter, Res, ResMut};
use uuid::Uuid;

use crate::component::inventory::{PlayerInventory, CLICK_QUICK_CRAFT, CRAFTING_RESULT_SLOT};
use crate::component::{Attributes, Health, InstanceRef, Player, Position, Velocity};
use crate::event::{EnterPlayEvent, NetworkEvent, PlayerChat, PlayerJoin, PlayerQuit};
use crate::network::bridge::NetworkBridge;
use crate::network::client::{ClientNetworks, Priority, enqueue_packet};
use crate::network::connection::ConnectionState;
use crate::network::listener::{OutboundMessage, RawFrame};
use crate::protocol::dispatch;
use crate::protocol::framing::encode_frame;
use crate::protocol::packets::play;
use crate::protocol::packets::{
    FeatureFlags, FinishConfigurationS2C, InboundPacket, Login, LoginCompression, LoginDisconnect,
    LoginSuccess, PingResponse, PlayerInfo, Position as PositionPacket, SpawnInfo, StatusResponse,
    SystemChatPacket, UpdateHealth, encode_clientbound,
};
use crate::protocol::velocity::verify_forwarding;
use crate::resource::TagRegistry;
use crate::resource::command::{CommandManager, PlayerSender};
use crate::resource::connection_manager::ConnectionManager;
use crate::resource::instance_manager::InstanceManager;
use crate::resource::registries::RegistrySnapshot;
use crate::resource::registries::nbt::{registry_data_packets, update_tags_packet};
use crate::resource::spawn_config::SpawnConfig;
use crate::resource::status_config::StatusConfig;
use crate::resource::velocity_config::VelocityConfig;
use particlemc_framework_ecs::scheduler::{InstanceScheduler, WorldId};

/// 从 `NetworkBridge` 取出全部入站帧并逐帧处理。
// 系统参数由调度框架决定，参数数量不受本代码控制，豁免 too_many_arguments。
#[allow(clippy::too_many_arguments)]
pub fn network_receive(
    mut bridge: ResMut<NetworkBridge>,
    mut connections: ResMut<ConnectionManager>,
    mut clients: ResMut<ClientNetworks>,
    config: Res<VelocityConfig>,
    _auth_ctx: Res<OnlineAuthContext>,
    instance_mgr: Res<InstanceManager>,
    status_config: Res<StatusConfig>,
    spawn_config: Res<SpawnConfig>,
    registry_snapshot: Res<RegistrySnapshot>,
    tag_registry: Res<TagRegistry>,
    mut commands: Commands,
    mut events: MessageWriter<NetworkEvent>,
    mut join_events: MessageWriter<PlayerJoin>,
    mut quit_events: MessageWriter<PlayerQuit>,
    mut enter_events: MessageWriter<EnterPlayEvent>,
    mut player_chat_events: MessageWriter<PlayerChat>,
    scheduler: Res<InstanceScheduler>,
) {
    while let Ok(frame) = bridge.inbound.try_recv() {
        match frame {
            RawFrame::Closed(conn_id) => {
                if let Some(entity) = connections.entity_of(conn_id) {
                    // R11.2：玩家实体已落入实例 World，跨 World 销毁；实例未就绪
                    // 时退回主 World（含单元测试场景）。
                    let wid = connections
                        .get(conn_id)
                        .map(|rt| rt.world_id)
                        .unwrap_or(WorldId(0));
                    if let Some(mut guard) = scheduler.lock_world(wid) {
                        guard.world().despawn(entity);
                    } else {
                        commands.despawn(entity);
                    }
                    let username = connections
                        .get(conn_id)
                        .and_then(|rt| rt.username.clone())
                        .unwrap_or_default();
                    quit_events.write(PlayerQuit { entity, username });
                }
                events.write(NetworkEvent::Closed(conn_id));
                connections.close(&conn_id);
                clients.remove(conn_id);
            }
            RawFrame::Packet {
                conn_id,
                state,
                packet_id,
                payload,
            } => {
                // 建连即确保存在该连接的发送状态
                clients.insert(conn_id);
                let inbound = match dispatch::dispatch(state, packet_id, &payload) {
                    Ok(p) => p,
                    Err(err) => {
                        eprintln!(
                            "[network] 无法派发 conn={conn_id} state={state:?} id={packet_id}: {err:?}"
                        );
                        continue;
                    }
                };
                handle_packet(
                    conn_id,
                    inbound,
                    &mut connections,
                    &mut clients,
                    &bridge,
                    &config,
                    &instance_mgr,
                    &status_config,
                    &spawn_config,
                    &registry_snapshot,
                    &tag_registry,
                    &scheduler,
                    &mut commands,
                    &mut events,
                    &mut join_events,
                    &mut quit_events,
                    &mut enter_events,
                    &mut player_chat_events,
                    #[cfg(feature = "online-auth")]
                    &_auth_ctx,
                );
            }
        }
    }
}

/// 命令聊天子系统：在 `network_receive` 之后运行，消费 `ClientNetworks.command_inbox`
/// 中由 0x06 / 0x07 包写入的 `(conn_id, message)`。反查连接绑定的玩家实体，构造
/// `PlayerSender`，经 `emit` 回发 `SystemChatPacket`(0x77) 后执行命令。无绑定实体
/// 则跳过（不 panic）。
///
/// 独立成系统以持有自己的 `Res<CommandManager>`，从而让 `network_receive` 保持在
/// `SystemParam` 元组 16 上限之内（其 16 个既有参数均不可删：网络参数 /
/// `PlayerJoin` / `PlayerQuit` / `NetworkEvent` 均被下游系统消费）。资源冲突由
/// 调度顺序约束解决（`command_chat_system` 在 `network_receive` 之后、`network_send`
/// 之前运行）。见 `.specs/implement-command-framework/`。
pub fn command_chat_system(
    command_manager: Res<CommandManager>,
    mut clients: ResMut<ClientNetworks>,
    connections: Res<ConnectionManager>,
) {
    // 取出整段收件箱（move 出来，避免与后续可变借用冲突）。
    let inbox = std::mem::take(&mut clients.command_inbox);
    for (conn_id, message) in inbox {
        let Some(entity) = connections.entity_of(conn_id) else {
            continue;
        };
        // R11.2：玩家实体已迁入实例 World，主 World 不再持有 `Player` 组件。
        // 用户名在登录落子时写入 `ConnectionRuntime.username`，此处直接读取，
        // 避免跨 World 回查。
        let Some(username) = connections.get(conn_id).and_then(|rt| rt.username.clone()) else {
            continue;
        };
        let sender = PlayerSender {
            entity_id: entity.index(),
            username,
        };
        // emit 闭包：将文本反馈编码为 SystemChatPacket 并以 Normal 优先级回发。
        let mut emit = |msg: &str| {
            enqueue_packet(
                &mut clients,
                conn_id,
                encode_clientbound(&SystemChatPacket {
                    message: msg.to_string(),
                    overlay: false,
                }),
                Priority::Normal,
            );
        };
        command_manager.execute(&message, &sender, &mut emit);
    }
}

/// 按入站包类型推进单条连接的状态，并发出入站包。
#[allow(clippy::too_many_arguments, unused_mut)]
fn handle_packet(
    conn_id: u32,
    inbound: InboundPacket,
    mut connections: &mut ConnectionManager,
    mut clients: &mut ClientNetworks,
    bridge: &NetworkBridge,
    config: &VelocityConfig,
    instance_mgr: &InstanceManager,
    status_config: &StatusConfig,
    spawn_config: &SpawnConfig,
    registry_snapshot: &RegistrySnapshot,
    tag_registry: &TagRegistry,
    scheduler: &InstanceScheduler,
    commands: &mut Commands,
    events: &mut MessageWriter<NetworkEvent>,
    join_events: &mut MessageWriter<PlayerJoin>,
    _quit_events: &mut MessageWriter<PlayerQuit>,
    enter_events: &mut MessageWriter<EnterPlayEvent>,
    player_chat_events: &mut MessageWriter<PlayerChat>,
    #[cfg(feature = "online-auth")] _auth_ctx: &OnlineAuthContext,
) {
    match inbound {
        InboundPacket::Intention(intention) => {
            let enforce = config.enforce_proxy;
            let secret = config.secret.clone();
            let mut accept = true;
            {
                let rt = connections.open(conn_id, None);
                rt.state = ConnectionState::Handshake;
                if let Some(blob) = &intention.forwarding {
                    if let Some(s) = &secret {
                        match verify_forwarding(s.as_bytes(), blob) {
                            Ok(identity) => rt.forwarded = Some(identity),
                            Err(_) if enforce => accept = false,
                            Err(_) => {}
                        }
                    }
                } else if enforce {
                    accept = false;
                }
            }
            if !accept {
                // 规格合规：enforce_proxy 拒绝直连时须先发送 LoginDisconnect，再关闭连接。
                // urgent 优先级确保本 tick 末 network_send 能 flush 出去；不要立即
                // clients.remove(conn_id)——否则该 conn 的发送状态消失，LoginDisconnect
                // 无法被 flush。保留 ClientNetwork 记录，待监听侧 Closed 帧到达后由
                // `RawFrame::Closed` 分支统一移除。
                enqueue_packet(
                    clients,
                    conn_id,
                    encode_clientbound(&LoginDisconnect {
                        reason: "connection not forwarded through Velocity".into(),
                    }),
                    Priority::Urgent,
                );
                connections.close(&conn_id);
            }
        }
        InboundPacket::StatusRequest => {
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&StatusResponse {
                    json: status_config.to_status_json(connections.active_count()),
                }),
                Priority::Urgent,
            );
        }
        InboundPacket::Ping(ping) => {
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&PingResponse {
                    payload: ping.payload,
                }),
                Priority::Urgent,
            );
        }
        InboundPacket::Hello(hello) => {
            // 连接已被拒绝/关闭时（如 enforce_proxy 拒绝直连）忽略后续登录包：
            // 不生成实体、不发送 LoginSuccess，避免在 LoginDisconnect 之后泄漏登录成功。
            if connections.get(conn_id).is_none() {
                return;
            }
            // 身份解析：Velocity 转发优先，否则离线（使用客户端 uuid 或随机）。
            let (uuid, username) =
                match connections.get(conn_id).and_then(|rt| rt.forwarded.clone()) {
                    Some(fwd) => (fwd.uuid, fwd.name),
                    None => {
                        let uuid = hello.uuid.unwrap_or_else(Uuid::new_v4);
                        (uuid, hello.name.clone())
                    }
                };
            // 在线认证（WS5b，feature `online-auth`）：Hello 阶段进入加密握手，
            // 下发公钥 + challenge token，记录 pending 后 return（暂不发 LoginSuccess）。
            #[cfg(feature = "online-auth")]
            {
                use crate::protocol::packets::LoginHelloResponse;
                use rand::rngs::OsRng;
                use rsa::RsaPrivateKey;
                use rsa::pkcs8::DecodePrivateKey;
                if _auth_ctx.enabled {
                    if let Some(priv_der) = &_auth_ctx.private_key_der {
                        if let Ok(private_key) = RsaPrivateKey::from_pkcs8_der(&priv_der) {
                            let default_instance =
                                instance_mgr.default_instance().unwrap_or(WorldId(0));
                            let (sx, sy, sz) = spawn_config.position();
                            let bundle = (
                                Player::new(uuid, &username),
                                Position::new(sx, sy, sz),
                                Health::new(20.0, 20.0),
                                Velocity::zero(),
                                InstanceRef(default_instance),
                                PlayerInventory::new(),
                                Attributes::default(),
                            );
                            let entity =
                                if let Some(mut guard) = scheduler.lock_world(default_instance) {
                                    guard.world().spawn_bundle(bundle).id()
                                } else {
                                    commands.spawn_bundle(bundle).id()
                                };
                            if let Some(rt) = connections.get_mut(conn_id) {
                                rt.entity = Some(entity);
                                rt.state = ConnectionState::Login;
                                rt.world_id = default_instance;
                                rt.uuid = Some(uuid);
                                rt.username = Some(username.clone());
                            }
                            // 生成 16 字节 challenge token + 服务端 RSA 公钥，下发 LoginHelloResponse。
                            let token = crate::crypto::generate_verify_token(&mut OsRng);
                            let public_key = crate::crypto::public_key_der(&private_key);
                            if let Some(rt) = connections.get_mut(conn_id) {
                                rt.pending_challenge_token = Some(token.to_vec());
                            }
                            enqueue_packet(
                                clients,
                                conn_id,
                                encode_clientbound(&LoginHelloResponse {
                                    public_key,
                                    verify_token: token.to_vec(),
                                }),
                                Priority::Urgent,
                            );
                            return;
                        }
                    }
                }
            }
            let default_instance = instance_mgr.default_instance().unwrap_or(WorldId(0));
            let (sx, sy, sz) = spawn_config.position();
            let bundle = (
                Player::new(uuid, &username),
                Position::new(sx, sy, sz),
                Health::new(20.0, 20.0),
                Velocity::zero(),
                InstanceRef(default_instance),
                PlayerInventory::new(),
                // R8：玩家生成即挂载属性组件（初始为空表，不主动下发任何属性包）。
                Attributes::default(),
            );
            // R11.2：玩家实体跨 World 落入默认实例 World；实例未就绪时退回主
            // World（含单元测试场景）。落子后记录所在 WorldId / UUID / 用户名，
            // 供 `player_input` / `inventory_sync` 跨 World 定位与发包。
            let entity = if let Some(mut guard) = scheduler.lock_world(default_instance) {
                guard.world().spawn_bundle(bundle).id()
            } else {
                commands.spawn_bundle(bundle).id()
            };
            if let Some(rt) = connections.get_mut(conn_id) {
                rt.entity = Some(entity);
                rt.state = ConnectionState::Login;
                rt.world_id = default_instance;
                rt.uuid = Some(uuid);
                rt.username = Some(username.clone());
            }
            join_events.write(PlayerJoin {
                entity,
                username: username.to_string(),
            });
            let login_success = LoginSuccess {
                uuid,
                username: username.to_string(),
                properties: Vec::new(),
            };
            let threshold = clients
                .clients
                .get(&conn_id)
                .map(|c| c.compression_threshold)
                .unwrap_or(0);
            if threshold > 0 {
                // T7 压缩启用：LoginSuccess 之后（进入 Configuration 前）下发 LoginCompression。
                // 两者必须以「未压缩」原帧格式直写出站通道（客户端在收到 LoginCompression
                // 前无法解压），随后**立即**置位该连接出站压缩标志——否则本 tick 内后续
                // flush（如 `inventory_sync` 的库存全量下发）会以明文帧发出，而客户端已按
                // 压缩帧格式解帧导致错位。`EnableCompression` 先入通道，写任务在写出
                // LoginCompression 前即置位读侧压缩标志，客户端回包即可解帧。
                if let Ok(guard) = bridge.outbound.lock()
                    && let Some(tx) = guard.get(&conn_id)
                {
                    let _ = tx.try_send(OutboundMessage::EnableCompression);
                    let mut frame = Vec::new();
                    if encode_frame(&mut frame, &encode_clientbound(&login_success)).is_ok() {
                        let _ = tx.try_send(OutboundMessage::Frame(frame));
                    }
                    let mut frame = Vec::new();
                    if encode_frame(
                        &mut frame,
                        &encode_clientbound(&LoginCompression { threshold }),
                    )
                    .is_ok()
                    {
                        let _ = tx.try_send(OutboundMessage::Frame(frame));
                    }
                }
                if let Some(client) = clients.clients.get_mut(&conn_id) {
                    client.compression_enabled = true;
                }
            } else {
                enqueue_packet(
                    clients,
                    conn_id,
                    encode_clientbound(&login_success),
                    Priority::Urgent,
                );
            }
        }
        #[cfg(feature = "online-auth")]
        InboundPacket::LoginChallenge(challenge) => {
            // 在线认证第二步（WS5b）：RSA 解密客户端回传的 verify_token 与本地 pending
            // 比较；解密共享密钥；验证失败 → LoginDisconnect + 关闭；成功 → 入队待
            // hasJoined 异步验证（由 `crypto::run_auth_worker` 下发 LoginSuccess）。
            use rsa::RsaPrivateKey;
            use rsa::pkcs8::DecodePrivateKey;

            fn reject(
                conn_id: u32,
                clients: &mut ClientNetworks,
                connections: &mut ConnectionManager,
                bridge: &NetworkBridge,
                reason: &str,
            ) {
                use crate::network::listener::OutboundMessage;
                use crate::protocol::framing::encode_frame;
                use crate::protocol::packets::{LoginDisconnect, encode_clientbound};
                if let Ok(guard) = bridge.outbound.lock() {
                    if let Some(tx) = guard.get(&conn_id) {
                        let mut frame = Vec::new();
                        if encode_frame(
                            &mut frame,
                            &encode_clientbound(&LoginDisconnect {
                                reason: reason.to_string(),
                            }),
                        )
                        .is_ok()
                        {
                            // `OutboundMap` 为有界 `tokio::sync::mpsc::Sender`，`send` 是异步
                            // 调用（需 await 才能真正发出），此处为同步系统必须用 `try_send`。
                            let _ = tx.try_send(OutboundMessage::Frame(frame));
                        }
                        // 服务端主动断开该连接：发送 FIN（位于已写数据之后），写任务据此关闭。
                        let _ = tx.try_send(OutboundMessage::Close);
                    }
                }
                connections.close(&conn_id);
                clients.clients.remove(&conn_id);
            }

            let (pending, priv_der, username, uuid, compression_threshold) = {
                let Some(rt) = connections.get(conn_id) else {
                    return;
                };
                (
                    rt.pending_challenge_token.clone(),
                    _auth_ctx.private_key_der.clone(),
                    rt.username.clone().unwrap_or_default(),
                    rt.uuid.unwrap_or_default(),
                    clients
                        .clients
                        .get(&conn_id)
                        .map(|c| c.compression_threshold)
                        .unwrap_or(0),
                )
            };

            let (private_key, pending) = match (
                priv_der.and_then(|d| RsaPrivateKey::from_pkcs8_der(&d).ok()),
                pending,
            ) {
                (Some(k), Some(p)) => (k, p),
                _ => {
                    reject(
                        conn_id,
                        &mut clients,
                        &mut connections,
                        &bridge,
                        "Authentication required",
                    );
                    return;
                }
            };

            match crate::crypto::verify_challenge_token(
                &private_key,
                &challenge.verify_token,
                &pending,
            ) {
                Ok(()) => {
                    match crate::crypto::decrypt_with_rsa(&private_key, &challenge.shared_secret) {
                        Ok(shared_secret) => {
                            let item = crate::crypto::PendingAuth {
                                conn_id,
                                username,
                                uuid,
                                shared_secret,
                                compression_threshold,
                            };
                            if let Some(tx) = &_auth_ctx.pending_tx {
                                let _ = tx.send(item);
                            }
                        }
                        Err(_) => reject(
                            conn_id,
                            &mut clients,
                            &mut connections,
                            &bridge,
                            "Authentication failed",
                        ),
                    }
                }
                Err(_) => reject(
                    conn_id,
                    &mut clients,
                    &mut connections,
                    &bridge,
                    "Authentication failed",
                ),
            }
        }
        // 默认构建（feature `online-auth` 关闭）：在线认证不启用，加密握手分支不应到达；
        // 若客户端误发 LoginChallenge（0x01），安全忽略并继续（不推进状态、不 panic）。
        #[cfg(not(feature = "online-auth"))]
        InboundPacket::LoginChallenge(_) => {}

        InboundPacket::LoginAcknowledged => {
            // 出站压缩已在 Hello 分支启用（T7）；此处仅记录状态机标志并推进配置阶段。
            let threshold = clients
                .clients
                .get(&conn_id)
                .map(|c| c.compression_threshold)
                .unwrap_or(0);
            if let Some(rt) = connections.get_mut(conn_id) {
                rt.state = ConnectionState::Configuration;
                if threshold > 0 {
                    rt.compression_enabled = true;
                }
            }
        }
        InboundPacket::ClientInformation(_) => {
            // 配置阶段：先同步注册表（RegistryData×N）与标签（UpdateTags）、
            // 特性（FeatureFlags），最后才发送 FinishConfiguration (S2C)。
            for packet in registry_data_packets(registry_snapshot) {
                enqueue_packet(
                    clients,
                    conn_id,
                    encode_clientbound(&packet),
                    Priority::Urgent,
                );
            }
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&update_tags_packet(tag_registry)),
                Priority::Urgent,
            );
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&FeatureFlags { flags: Vec::new() }),
                Priority::Urgent,
            );
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&FinishConfigurationS2C),
                Priority::Urgent,
            );
        }
        InboundPacket::FinishConfiguration => {
            if let Some(rt) = connections.get_mut(conn_id) {
                rt.state = ConnectionState::Play;
            }
            // 进入 Play：先发 Login(play) 包（客户端解析世界状态必需），
            // 再发出生坐标 / 生命值 / 玩家列表；最后写入 EnterPlayEvent
            // 供 chunk_send（出区块）与 entity_sync（互见广播）消费。
            let Some(entity) = connections.entity_of(conn_id) else {
                return;
            };
            // 玩家 UUID / 用户名在登录握手时已写入连接运行时，跨 World 读取无需回查实例实体。
            let Some(rt) = connections.get(conn_id) else {
                return;
            };
            let Some(uuid) = rt.uuid else {
                return;
            };
            let username = match &rt.username {
                Some(u) => u.clone(),
                None => return,
            };
            let (sx, sy, sz) = spawn_config.position();
            // 维度类型注册表序号：`minecraft:overworld` 在 1.21.11 注册数据中
            // id=0（注册表按 id 升序同步），故 Login(play) 的 dimension 恒为 0。
            const OVERWORLD_DIMENSION_ID: i32 = 0;

            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&Login {
                    entity_id: conn_id as i32,
                    is_hardcore: false,
                    world_names: vec!["minecraft:overworld".to_string()],
                    max_players: status_config.max_players,
                    view_distance: 10,
                    simulation_distance: 10,
                    reduced_debug_info: false,
                    enable_respawn_screen: true,
                    do_limited_crafting: false,
                    world_state: SpawnInfo {
                        dimension: OVERWORLD_DIMENSION_ID,
                        name: "minecraft:overworld".to_string(),
                        hashed_seed: 0,
                        gamemode: 0,
                        previous_gamemode: 255,
                        is_debug: false,
                        is_flat: true,
                        death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    enforces_secure_chat: false,
                }),
                Priority::Urgent,
            );
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&PositionPacket {
                    teleport_id: 1,
                    x: sx,
                    y: sy,
                    z: sz,
                    dx: 0.0,
                    dy: 0.0,
                    dz: 0.0,
                    yaw: spawn_config.yaw,
                    pitch: spawn_config.pitch,
                    flags: 0,
                }),
                Priority::Urgent,
            );
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&UpdateHealth {
                    health: 20.0,
                    food: 20,
                    food_saturation: 5.0,
                }),
                Priority::Urgent,
            );
            enqueue_packet(
                clients,
                conn_id,
                encode_clientbound(&PlayerInfo {
                    uuid,
                    name: username,
                    properties: Vec::new(),
                }),
                Priority::Urgent,
            );
            enter_events.write(EnterPlayEvent { conn_id, entity });
        }
        InboundPacket::PlayerPosition(p) => {
            if connections.entity_of(conn_id).is_some() {
                events.write(NetworkEvent::PlayerMoveRaw {
                    conn_id,
                    position: Position::new(p.x, p.y, p.z),
                    yaw: 0.0,
                    pitch: 0.0,
                    grounded: p.grounded,
                });
            }
        }
        InboundPacket::PlayerPositionAndRotation(p) => {
            if connections.entity_of(conn_id).is_some() {
                events.write(NetworkEvent::PlayerMoveRaw {
                    conn_id,
                    position: Position::new(p.x, p.y, p.z),
                    yaw: p.yaw,
                    pitch: p.pitch,
                    grounded: p.grounded,
                });
            }
        }
        InboundPacket::ClickContainer(click) => {
            // 仅处理默认玩家库存窗口（window_id == 0）；其它窗口本框架暂不支持，忽略。
            // 见 .specs/implement-item-click/。
            if click.window_id != 0 {
                return;
            }
            // 反查该连接绑定的玩家实体；无则忽略（不 panic）。
            let Some(entity) = connections.entity_of(conn_id) else {
                return;
            };
            // 权威重算库存：忽略客户端 predicted 的 changed_slots / carried_item，防作弊与不一致。
            // 见 .specs/implement-item-click/。
            // R11.2：玩家实体已迁入实例 World，背包组件随之移动，须跨 World 取可变引用。
            let wid = connections
                .get(conn_id)
                .map(|rt| rt.world_id)
                .unwrap_or(WorldId(0));
            if let Some(mut guard) = scheduler.lock_world(wid)
                && let Some(inv) = guard.world().get_mut::<PlayerInventory>(entity)
            {
                // 直接将客户端窗口序传入：`apply_click` 入参契约为窗口序 0..=45，
                // 其内部会再做一次窗口序 → 内部序转换。此处不再预先转换，
                // 否则会双重转换、点击路由到错误槽位（本 bug 根因）。
                inv.apply_click(i32::from(click.slot), click.button, click.mode);
                // 若为 QUICK_CRAFT 模式且点击了合成结果槽（窗口序 4 → 内部序 40），
                // 将产物暂存于 `crafting_result_pending`，供应用侧合成系统消费。
                //（此判断须用内部序比较：为此分支内部序转换保留，仅用于判断合成结果槽 40）
                if click.mode == CLICK_QUICK_CRAFT
                    && crate::component::inventory::window_slot_to_minestom_slot(i32::from(
                        click.slot,
                    )) == CRAFTING_RESULT_SLOT
                {
                    let result_slot = inv.get(CRAFTING_RESULT_SLOT as usize);
                    if !result_slot.is_air() {
                        inv.crafting_result_pending = Some(result_slot);
                    }
                }
            }
        }
        InboundPacket::CloseContainer(_close) => {
            // 客户端关闭窗口（serverbound, id 0x12）：清空玩家光标物品，无论 window_id。
            // 框架暂无掉落实体，直接丢弃（见 `.specs/implement-item-inventory/`）。
            let Some(entity) = connections.entity_of(conn_id) else {
                return;
            };
            // R11.2：玩家实体已迁入实例 World，背包组件随之移动，须跨 World 取可变引用。
            let wid = connections
                .get(conn_id)
                .map(|rt| rt.world_id)
                .unwrap_or(WorldId(0));
            if let Some(mut guard) = scheduler.lock_world(wid)
                && let Some(inv) = guard.world().get_mut::<PlayerInventory>(entity)
            {
                inv.drop_cursor();
            }
        }
        InboundPacket::HeldItemChange(c) => {
            // 客户端切换手持槽（serverbound, id 0x34，c.slot: i16）。
            // 仅接受 0..=8；越界（含负值）不处理、不回发（见 `.specs/implement-item-inventory/`）。
            let Some(entity) = connections.entity_of(conn_id) else {
                return;
            };
            // R11.2：玩家实体已迁入实例 World，背包组件随之移动，须跨 World 取可变引用。
            let wid = connections
                .get(conn_id)
                .map(|rt| rt.world_id)
                .unwrap_or(WorldId(0));
            if (0..=8).contains(&c.slot)
                && let Ok(slot_u8) = u8::try_from(c.slot)
                && let Some(mut guard) = scheduler.lock_world(wid)
                && let Some(inv) = guard.world().get_mut::<PlayerInventory>(entity)
                && inv.set_held_slot(slot_u8)
                && let Ok(slot_i8) = i8::try_from(c.slot)
            {
                // 回发 clientbound HeldItemChange（id 0x67，slot: i8）确认。
                enqueue_packet(
                    clients,
                    conn_id,
                    encode_clientbound(&play::HeldItemChange { slot: slot_i8 }),
                    Priority::Normal,
                );
            }
        }
        InboundPacket::ClientCommandChat(p) => {
            // 命令聊天入口（0x06）：写入收件箱，由 `command_chat_system` 在本 tick
            // 稍后异步执行。解耦设计以避免 `network_receive` 的 `SystemParam` 元组
            // 超出 旧 ECS 方案 16 上限。见 .specs/implement-command-framework/。
            clients.command_inbox.push((conn_id, p.message));
        }
        InboundPacket::ClientSignedCommandChat(p) => {
            // 签名命令聊天入口（0x07）：同上。见 .specs/implement-command-framework/。
            clients.command_inbox.push((conn_id, p.message));
        }
        InboundPacket::Settings(settings) => {
            // 客户端设置（视距、聊天模式等）：更新连接运行时中的视距，供
            // `entity_sync` 的距离裁剪使用。
            if let Some(rt) = connections.get_mut(conn_id) {
                rt.set_view_distance(settings.view_distance as u8);
            }
        }
        InboundPacket::ClientStatus(_)
        | InboundPacket::TeleportConfirm(_)
        | InboundPacket::KeepAlive(_)
        | InboundPacket::Look(_)
        | InboundPacket::PlayerPositionStatus(_)
        | InboundPacket::PlayerLoaded(_)
        | InboundPacket::QueryBlockNbt(_)
        | InboundPacket::SelectBundleItem(_)
        | InboundPacket::ChangeDifficulty(_)
        | InboundPacket::ChangeGameMode(_)
        | InboundPacket::ChatAck(_)
        | InboundPacket::ChatSessionUpdate(_)
        | InboundPacket::TickEnd(_)
        | InboundPacket::TabComplete(_)
        | InboundPacket::ConfigurationAck(_)
        | InboundPacket::ClickWindowButton(_)
        | InboundPacket::WindowSlotState(_)
        | InboundPacket::CookieResponse(_)
        | InboundPacket::ClientPluginMessage(_)
        | InboundPacket::DebugSubscriptionRequest(_)
        | InboundPacket::EditBook(_)
        | InboundPacket::QueryEntityNbt(_)
        | InboundPacket::GenerateStructure(_)
        | InboundPacket::LockDifficulty(_)
        | InboundPacket::VehicleMove(_)
        | InboundPacket::SteerBoat(_)
        | InboundPacket::PickItemFromBlock(_)
        | InboundPacket::PickItemFromEntity(_)
        | InboundPacket::PingRequest(_)
        | InboundPacket::PlaceRecipe(_)
        | InboundPacket::PlayerAbilities(_)
        | InboundPacket::EntityAction(_)
        | InboundPacket::Input(_)
        | InboundPacket::Pong(_)
        | InboundPacket::SetRecipeBookState(_)
        | InboundPacket::RecipeBookSeenRecipe(_)
        | InboundPacket::NameItem(_)
        | InboundPacket::ResourcePackStatus(_)
        | InboundPacket::AdvancementTab(_)
        | InboundPacket::SelectTrade(_)
        | InboundPacket::SetBeaconEffect(_)
        | InboundPacket::UpdateCommandBlock(_)
        | InboundPacket::UpdateCommandBlockMinecart(_)
        | InboundPacket::CreativeInventoryAction(_)
        | InboundPacket::UpdateJigsawBlock(_)
        | InboundPacket::UpdateStructureBlock(_)
        | InboundPacket::SetTestBlock(_)
        | InboundPacket::UpdateSign(_)
        | InboundPacket::Spectate(_)
        | InboundPacket::TestInstanceBlockAction(_)
        | InboundPacket::CustomClickAction(_)
        | InboundPacket::Ignored { .. } => {}
        // 消费 ChatMessage（serverbound 0x08），派发 PlayerChat 事件供 chat_broadcast 广播。
        InboundPacket::ChatMessage(p) => {
            if let Some(entity) = connections.entity_of(conn_id) {
                let _ = player_chat_events.write(PlayerChat {
                    player: entity,
                    message: p.message.clone(),
                });
            }
        }
        // 框架关注的动作类 serverbound 包：写入收件箱，由 `packet_action_system`
        // 在本 tick 稍后经 EventBus 派发事件（避免 `network_receive` 增参超出
        // 旧 ECS 方案 SystemParam 16 上限）。见 `.specs/implement-framework-capabilities/`。
        InboundPacket::InteractEntity(_)
        | InboundPacket::PlayerAction(_)
        | InboundPacket::Animation(_)
        | InboundPacket::UseItem(_)
        | InboundPacket::PlayerBlockPlacement(_) => {
            clients.packet_inbox.push((conn_id, inbound));
        }
        InboundPacket::ChunkBatchReceived(_) => {
            // 客户端确认收到区块批次：推进该连接的区块信用（加速后续批次）。
            if let Some(client) = clients.clients.get_mut(&conn_id) {
                client.chunk_sender.on_batch_received();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::time::Duration;

    use crate::app::App;

    use crate::prelude::Entity;

    #[cfg(not(feature = "online-auth"))]
    use crate::component::Player;
    use crate::component::inventory::PlayerInventory;
    use crate::item_stack::ItemStack;
    use crate::network::bridge::empty_bridge;
    use crate::network::client::ClientNetworks;
    use crate::network::connection::ConnectionState;
    use crate::network::listener::{OutboundMessage, RawFrame};
    use crate::plugin::McServerPlugin;
    use crate::protocol::byte_buf::ByteBuffer;
    use crate::protocol::framing::decode_frame;
    use crate::protocol::packet::Packet;
    use crate::protocol::packets::play;
    use crate::protocol::packets::{
        ClickContainer, ClientCommandChatPacket, ClientHeldItemChange, CloseContainer, Hello,
        Intention, LoginDisconnect, LoginSuccess, SystemChatPacket, WindowItemsPacket,
    };
    use crate::resource::command::{Command, CommandManager};
    use crate::resource::compression_config::CompressionConfig;
    use crate::resource::connection_manager::ConnectionManager;
    use crate::resource::velocity_config::VelocityConfig;
    use crate::test_support::{ensure_test_instance_default, read_instance, with_instance_entity};
    use particlemc_framework_ecs::scheduler::WorldId;

    /// 构造测试 App：
    /// - 装配 `McServerPlugin`（插件内置一个空桥接，此处用注入通道的桥接覆盖，以模拟入站帧）；
    /// - 为 `conn_id` 注册出站通道，用于捕获 `network_send` flush 的真实字节；
    /// - 手动推进固定时间：每次 `app.update()` 前进 50ms（即一个 20Hz 步长）。
    fn build_app(
        conn_id: u32,
    ) -> (
        App,
        tokio::sync::mpsc::Sender<RawFrame>,
        tokio::sync::mpsc::Receiver<OutboundMessage>,
    ) {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());

        // T7 压缩：单元测试走注入帧通道、`capture_payloads` 按原帧格式解帧，
        // 显式将压缩阈值置 0（禁用压缩），避免默认 256 破坏既有包序列断言。
        app.world_mut()
            .insert_resource(CompressionConfig { threshold: 0 });
        app.world_mut()
            .insert_resource(ClientNetworks::with_compression_threshold(0));

        let (bridge, frame_tx, outbound) = empty_bridge();
        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<OutboundMessage>(64);
        outbound.lock().unwrap().insert(conn_id, out_tx);
        app.world_mut().insert_resource(bridge);

        app.world_mut()
            .insert_resource(crate::app::TimeUpdateStrategy::ManualDuration(
                Duration::from_millis(50),
            ));
        // 旧 ECS 方案 的 `Time<Real>` 首次 update 只记录 last_update 而不产生 delta（首帧为热身帧），
        // 故先空转一次，使调用方的首个 `app.update()` 真正推进 50ms 并运行一次 Schedule。
        app.update();
        (app, frame_tx, out_rx)
    }

    /// 从出站捕获通道读出全部已 flush 的完整帧，剥掉帧长度前缀后返回 payload 列表。
    /// （出站通道为 [`OutboundMessage`]；`Close` 指令不计入。）
    ///
    /// 注意：normal 队列按 MTU 聚合后可能把多帧写入同一条 `OutboundMessage::Frame`，
    /// 故此处逐帧解包直到缓冲耗尽（与真实客户端对 TCP 流的连续解帧行为一致）。
    /// 见 `.specs/implement-command-framework/`。
    fn capture_payloads(rx: &mut tokio::sync::mpsc::Receiver<OutboundMessage>) -> Vec<Vec<u8>> {
        let mut payloads = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let OutboundMessage::Frame(frame) = msg {
                let mut pos = 0usize;
                while pos < frame.len() {
                    match decode_frame(&frame, &mut pos) {
                        Ok(payload) => payloads.push(payload),
                        Err(_) => break,
                    }
                }
            }
        }
        payloads
    }

    /// 解析 payload 的 packet_id（payload 首字段为 VarInt 的 packet_id）。
    #[allow(dead_code)]
    fn payload_packet_id(payload: &[u8]) -> Option<i32> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        buf.get_varint().ok()
    }

    /// 将 payload（packet_id + 包体）解码为 `LoginSuccess`，非该包返回 `None`。
    #[allow(dead_code)]
    fn decode_login_success(payload: &[u8]) -> Option<LoginSuccess> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        if buf.get_varint().ok()? != 0x02 {
            return None;
        }
        let body = buf.get_bytes(buf.remaining()).ok()?;
        LoginSuccess::decode(&mut ByteBuffer::new(body)).ok()
    }

    /// 将 payload（packet_id + 包体）解码为 `LoginDisconnect`，非该包返回 `None`。
    fn decode_login_disconnect(payload: &[u8]) -> Option<LoginDisconnect> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        if buf.get_varint().ok()? != 0x00 {
            return None;
        }
        let body = buf.get_bytes(buf.remaining()).ok()?;
        LoginDisconnect::decode(&mut ByteBuffer::new(body)).ok()
    }

    /// 将 payload（packet_id + 包体）解码为 `WindowItemsPacket`，非该包返回 `None`。
    /// 见 .specs/implement-item-click/。
    fn decode_window_items(payload: &[u8]) -> Option<WindowItemsPacket> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        if buf.get_varint().ok()? != 0x12 {
            return None;
        }
        let body = buf.get_bytes(buf.remaining()).ok()?;
        WindowItemsPacket::decode(&mut ByteBuffer::new(body)).ok()
    }

    /// 将 payload（packet_id + 包体）解码为 `play::HeldItemChange`，非该包返回 `None`。
    /// 见 .specs/implement-item-inventory/。
    fn decode_held_item_change(payload: &[u8]) -> Option<i8> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        if buf.get_varint().ok()? != 0x67 {
            return None;
        }
        play::HeldItemChange::decode(&mut buf).ok().map(|p| p.slot)
    }

    /// 将 payload（packet_id + 包体）解码为 `SystemChatPacket`（0x77）的 `message`，非该包返回 `None`。
    /// 见 .specs/implement-command-framework/。
    fn decode_system_chat(payload: &[u8]) -> Option<String> {
        let mut buf = ByteBuffer::new(payload.to_vec());
        if buf.get_varint().ok()? != 0x77 {
            return None;
        }
        let body = buf.get_bytes(buf.remaining()).ok()?;
        SystemChatPacket::decode(&mut ByteBuffer::new(body))
            .ok()
            .map(|p| p.message)
    }

    /// 复用手势到 Play 的流程，返回 (app, frame_tx, out_rx, entity)。
    /// 见 .specs/implement-item-click/。
    fn connect_to_play(
        conn_id: u32,
    ) -> (
        App,
        tokio::sync::mpsc::Sender<RawFrame>,
        tokio::sync::mpsc::Receiver<OutboundMessage>,
        Entity,
    ) {
        let (mut app, frame_tx, out_rx) = build_app(conn_id);
        drive_to_play(&mut app, &frame_tx, conn_id);
        let cm = app.world().resource::<ConnectionManager>().unwrap();
        let entity = cm.entity_of(conn_id).expect("进入 Play 后应有玩家实体");
        (app, frame_tx, out_rx, entity)
    }

    /// 驱动已 build 的 app 走完「握手 → 登录 → 配置 → Play」状态机（帧序列与
    /// [`connect_to_play`] 一致），使 `on_join` 在某一 `app.update()` 落子玩家。
    fn drive_to_play(app: &mut App, frame_tx: &tokio::sync::mpsc::Sender<RawFrame>, conn_id: u32) {
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id,
                state: ConnectionState::Handshake,
                packet_id: 0x00,
                payload: intention_no_forwarding(),
            })
            .unwrap();
        app.update();

        let mut hbuf = ByteBuffer::with_capacity(32);
        Hello {
            name: "Tester".to_string(),
            uuid: None,
        }
        .encode(&mut hbuf)
        .unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id,
                state: ConnectionState::Login,
                packet_id: 0x00,
                payload: hbuf.into_inner(),
            })
            .unwrap();
        app.update();

        frame_tx
            .try_send(RawFrame::Packet {
                conn_id,
                state: ConnectionState::Login,
                packet_id: 0x03,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();

        frame_tx
            .try_send(RawFrame::Packet {
                conn_id,
                state: ConnectionState::Configuration,
                packet_id: 0x00,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();

        frame_tx
            .try_send(RawFrame::Packet {
                conn_id,
                state: ConnectionState::Configuration,
                packet_id: 0x03,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();
    }

    /// 同 [`connect_to_play`]，但注入 `InstanceScheduler` + `InstanceManager`
    /// （默认实例指向测试实例 World），使 `on_join` 将玩家 spawn 到实例 World
    /// （与生产路径一致）；额外返回实例 World 的 `WorldId`，供跨 World 读写玩家
    /// 组件（如 `PlayerInventory`）。见 .specs/implement-item-click/。
    fn connect_to_play_instance(
        conn_id: u32,
    ) -> (
        App,
        tokio::sync::mpsc::Sender<RawFrame>,
        tokio::sync::mpsc::Receiver<OutboundMessage>,
        Entity,
        WorldId,
    ) {
        let (mut app, frame_tx, out_rx) = build_app(conn_id);
        let inst = ensure_test_instance_default(&mut app);
        drive_to_play(&mut app, &frame_tx, conn_id);
        let cm = app.world().resource::<ConnectionManager>().unwrap();
        let entity = cm.entity_of(conn_id).expect("进入 Play 后应有玩家实体");
        (app, frame_tx, out_rx, entity, inst)
    }

    /// 编码无转发的握手意图（pv=774, localhost:25565, next_state=2）。
    fn intention_no_forwarding() -> Vec<u8> {
        let mut buf = ByteBuffer::with_capacity(32);
        Intention {
            protocol_version: 774,
            server_address: "localhost".to_string(),
            port: 25565,
            next_state: 2,
            forwarding: None,
        }
        .encode(&mut buf)
        .unwrap();
        buf.into_inner()
    }

    /// T7.4：注入通道逐帧推进完整状态机，最终进入 Play。
    // feature-on 时 Hello 分支走 RSA 握手，不发 LoginSuccess，故本测试仅在离线构建下运行。
    #[cfg(not(feature = "online-auth"))]
    #[test]
    fn handshake_to_login_success() {
        let (mut app, frame_tx, mut out_rx) = build_app(1);

        // —— 1. 握手意图（离线直连，无转发）——
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Handshake,
                packet_id: 0x00,
                payload: intention_no_forwarding(),
            })
            .unwrap();
        app.update();
        {
            let cm = app.world().resource::<ConnectionManager>().unwrap();
            assert_eq!(cm.get(1).unwrap().state, ConnectionState::Handshake);
        }

        // —— 2. 登录 Hello ——
        let mut hbuf = ByteBuffer::with_capacity(32);
        Hello {
            name: "Tester".to_string(),
            uuid: None,
        }
        .encode(&mut hbuf)
        .unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Login,
                packet_id: 0x00,
                payload: hbuf.into_inner(),
            })
            .unwrap();
        app.update();
        let entity = {
            let cm = app.world().resource::<ConnectionManager>().unwrap();
            assert_eq!(cm.get(1).unwrap().state, ConnectionState::Login);
            cm.entity_of(1).expect("登录后应绑定玩家实体")
        };
        // 玩家实体存在且用户名正确（players 查询应可见）
        {
            let found = {
                let q = app.world_mut().query::<&Player, ()>();
                q.iter().any(|p| p.username() == "Tester")
            };
            assert!(found, "players 查询应能见到 Tester");
            // 该实体同时带有 Position / Health / Velocity / InstanceRef 组件
            let player = app.world().get::<Player>(entity).unwrap();
            assert_eq!(player.username(), "Tester");
        }
        // LoginSuccess 已入 urgent 队列并随本 tick flush 出去
        //（urgent 队列在 network_send 末被清空，故从出站捕获通道验证实际发出的字节）
        let flushed = capture_payloads(&mut out_rx);
        let ls = flushed
            .iter()
            .find_map(|p| decode_login_success(p))
            .expect("flush 应含 LoginSuccess");
        assert_eq!(ls.username, "Tester");

        // —— 3. LoginAcknowledged → Configuration ——
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Login,
                packet_id: 0x03,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();
        {
            let cm = app.world().resource::<ConnectionManager>().unwrap();
            assert_eq!(cm.get(1).unwrap().state, ConnectionState::Configuration);
        }

        // —— 4. ClientInformation → 服务端下发 FinishConfiguration(S2C) ——
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Configuration,
                packet_id: 0x00,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();
        {
            let cm = app.world().resource::<ConnectionManager>().unwrap();
            assert_eq!(cm.get(1).unwrap().state, ConnectionState::Configuration);
        }
        let flushed = capture_payloads(&mut out_rx);
        assert!(
            flushed.iter().any(|p| payload_packet_id(p) == Some(0x03)),
            "ClientInformation 后应 flush FinishConfiguration(S2C)"
        );

        // —— 5. FinishConfiguration (C2S) → Play ——
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Configuration,
                packet_id: 0x03,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();
        {
            let cm = app.world().resource::<ConnectionManager>().unwrap();
            assert_eq!(cm.get(1).unwrap().state, ConnectionState::Play);
            assert!(cm.entity_of(1).is_some(), "进入 Play 后玩家实体仍在线");
        }
        // 网络层仍保留该连接的发送状态
        {
            let clients = app.world().resource::<ClientNetworks>().unwrap();
            assert!(clients.clients.contains_key(&1));
        }
    }

    /// 规格合规回归：enforce_proxy=true 且握手无转发时，须先发 LoginDisconnect 再关闭连接。
    #[test]
    fn enforce_proxy_rejects_direct_connection() {
        let (mut app, frame_tx, mut out_rx) = build_app(1);

        // 覆盖插件默认（init_resource）的 VelocityConfig：强制仅收代理流量
        app.world_mut().insert_resource(VelocityConfig {
            secret: Some("s".to_string()),
            enforce_proxy: true,
        });

        // 无转发的握手（直连）
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Handshake,
                packet_id: 0x00,
                payload: intention_no_forwarding(),
            })
            .unwrap();
        app.update();

        // 连接已被关闭：ConnectionManager 中无该连接记录、无玩家实体
        {
            let cm = app.world().resource::<ConnectionManager>().unwrap();
            assert!(cm.get(1).is_none(), "enforce=true 时直连应被拒绝");
            assert!(cm.entity_of(1).is_none());
        }

        // 缺口修复验证：LoginDisconnect 已入 urgent 队列并随本 tick flush 出去
        //（urgent 队列在 network_send 末被清空，故从出站捕获通道验证实际发出的字节）
        let flushed = capture_payloads(&mut out_rx);
        let ld = flushed
            .iter()
            .find_map(|p| decode_login_disconnect(p))
            .expect("出站应含 LoginDisconnect");
        assert_eq!(ld.reason, "connection not forwarded through Velocity");

        // 修复后不立即移除 ClientNetwork 记录（LoginDisconnect 需本 tick 末 flush；
        // 记录保留至监听侧 Closed 帧到达后由 RawFrame::Closed 分支统一移除）
        {
            let clients = app.world().resource::<ClientNetworks>().unwrap();
            let client = clients
                .clients
                .get(&1)
                .expect("LoginDisconnect flush 前不应移除 ClientNetwork");
            // 队列已被 network_send 清空（LoginDisconnect 已发出）
            assert!(client.urgent_queue.is_empty());
            assert!(client.normal_queue.is_empty());
        }

        // 监听侧 Closed 帧到达后，ClientNetwork 记录被移除
        frame_tx.try_send(RawFrame::Closed(1)).unwrap();
        app.update();
        {
            let clients = app.world().resource::<ClientNetworks>().unwrap();
            assert!(
                !clients.clients.contains_key(&1),
                "Closed 帧后应移除 ClientNetwork"
            );
            assert!(
                app.world()
                    .resource::<ConnectionManager>()
                    .unwrap()
                    .get(1)
                    .is_none()
            );
        }
    }

    /// T4：左键整取热键栏槽将物品移至光标，并同步为 WindowItemsPacket（0x12）。
    /// 见 .specs/implement-item-click/。
    #[test]
    fn click_container_pickup_moves_item_to_cursor_and_syncs() {
        let (mut app, frame_tx, mut out_rx, entity, inst) = connect_to_play_instance(1);

        // 给内部槽 0（窗口序 36、held_slot=0 热键栏）放入 10 个钻石（跨 World 写实例 World 库存）
        with_instance_entity::<PlayerInventory, _>(&mut app, inst, entity, |inv| {
            inv.set(0, ItemStack::new(264, 10));
        });

        // 清空握手/进入 Play 阶段已缓冲的出站帧，确保后续只捕获本次点击后同步的 WindowItemsPacket
        let _ = capture_payloads(&mut out_rx);

        // 注入一次 ClickContainer 帧（左键整取窗口序 36 的整堆）
        let mut cbuf = ByteBuffer::with_capacity(32);
        ClickContainer {
            window_id: 0,
            state_id: 0,
            slot: 36,
            button: 0,
            mode: 0,
            changed_slots: vec![],
            carried_item: ItemStack::AIR,
        }
        .encode(&mut cbuf)
        .unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x11,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        // 捕获出站帧，找到 packet_id == 0x12 的 WindowItemsPacket payload
        let flushed = capture_payloads(&mut out_rx);
        let wi = flushed
            .iter()
            .find_map(|p| decode_window_items(p))
            .expect("入股点击后应 flush WindowItemsPacket(0x12)");
        // 规格断言：items 长度 46；窗口槽 36（内部槽 0）应为 AIR
        assert_eq!(wi.items.len(), 46, "WindowItemsPacket 应含 46 个槽位");
        let slot36 = wi.items.get(36).expect("应有窗口槽 36");
        assert!(slot36.is_air(), "窗口槽 36 整取后应为 AIR");
        // 光标携带 10 个钻石
        assert_eq!(
            wi.carried_item,
            ItemStack::new(264, 10),
            "10 个钻石应移至光标"
        );
    }

    /// T4：window_id != 0 的点击被忽略（不 panic，库存不变）。
    /// 见 .specs/implement-item-click/。
    #[test]
    fn click_container_other_window_ignored() {
        let (mut app, frame_tx, _out_rx, entity) = connect_to_play(1);

        app.world_mut()
            .get_mut::<PlayerInventory>(entity)
            .expect("玩家应挂载 PlayerInventory")
            .set(0, ItemStack::new(264, 10));

        // 注入 window_id != 0 的 ClickContainer
        let mut cbuf = ByteBuffer::with_capacity(32);
        ClickContainer {
            window_id: 5,
            state_id: 0,
            slot: 36,
            button: 0,
            mode: 0,
            changed_slots: vec![],
            carried_item: ItemStack::AIR,
        }
        .encode(&mut cbuf)
        .unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x11,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        // 直接读组件断言：库存未被改动
        let inv = app
            .world()
            .get::<PlayerInventory>(entity)
            .expect("玩家应挂载 PlayerInventory");
        assert!(inv.cursor.is_air(), "非默认窗口点击不应改变光标");
        let slot0 = inv.get(0);
        assert_eq!(slot0, ItemStack::new(264, 10), "内部槽 0 应仍为 10 钻石");
    }

    /// T3：客户端关闭任意窗口（serverbound 0x12）清空玩家光标物品。
    /// 见 .specs/implement-item-inventory/。
    #[test]
    fn close_container_clears_cursor() {
        let (mut app, frame_tx, _out_rx, entity, inst) = connect_to_play_instance(1);

        // 预先放入光标 5 个钻石（id 264）（跨 World 写实例 World 库存）
        with_instance_entity::<PlayerInventory, _>(&mut app, inst, entity, |inv| {
            inv.cursor = ItemStack::new(264, 5);
        });

        // 注入 CloseContainer（window_id: 0）
        let mut cbuf = ByteBuffer::with_capacity(16);
        CloseContainer { window_id: 0 }.encode(&mut cbuf).unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x12,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        let inv = read_instance::<PlayerInventory>(&mut app, inst, entity)
            .expect("玩家应在实例 World 挂载 PlayerInventory");
        assert!(inv.cursor.is_air(), "关闭窗口后应清空光标");
    }

    /// T3：window_id != 0 的关闭包同样清空光标（不区分窗口），且不 panic。
    /// 见 .specs/implement-item-inventory/。
    #[test]
    fn close_container_empty_or_other_window_no_panic() {
        let (mut app, frame_tx, _out_rx, entity) = connect_to_play(1);
        // 光标为空（默认）

        // 注入 CloseContainer（window_id: 5，非默认窗口）
        let mut cbuf = ByteBuffer::with_capacity(16);
        CloseContainer { window_id: 5 }.encode(&mut cbuf).unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x12,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        // 测试正常结束（无 panic）；光标仍为空
        let inv = app
            .world()
            .get::<PlayerInventory>(entity)
            .expect("玩家应挂载 PlayerInventory");
        assert!(inv.cursor.is_air(), "空光标关闭窗口后应为依然空");
    }

    /// T3：合法槽（0..=8）切换手持槽并更新库存，且回发 clientbound HeldItemChange（0x67）。
    /// 见 .specs/implement-item-inventory/。
    #[test]
    fn held_item_change_sets_slot_and_acks() {
        let (mut app, frame_tx, mut out_rx, entity, inst) = connect_to_play_instance(1);

        // 清空握手/进入 Play 阶段已缓冲的出站帧
        let _ = capture_payloads(&mut out_rx);

        // 注入 ClientHeldItemChange（slot: 3）
        let mut cbuf = ByteBuffer::with_capacity(16);
        ClientHeldItemChange { slot: 3 }.encode(&mut cbuf).unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x34,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        let inv = read_instance::<PlayerInventory>(&mut app, inst, entity)
            .expect("玩家应在实例 World 挂载 PlayerInventory");
        assert_eq!(inv.held_slot, 3, "手持槽应切换为 3");

        let flushed = capture_payloads(&mut out_rx);
        assert!(
            flushed
                .iter()
                .any(|p| decode_held_item_change(p) == Some(3)),
            "切换手持槽后应回发 clientbound HeldItemChange(0x67, slot=3)"
        );
    }

    /// T3：越界槽（>= 9）不更新库存、不回发确认（无 clientbound HeldItemChange）。
    /// 见 .specs/implement-item-inventory/。
    #[test]
    fn held_item_change_out_of_range_no_change_no_ack() {
        let (mut app, frame_tx, mut out_rx, entity) = connect_to_play(1);

        // 清空握手/进入 Play 阶段已缓冲的出站帧
        let _ = capture_payloads(&mut out_rx);

        // 注入 ClientHeldItemChange（slot: 9，越界）
        let mut cbuf = ByteBuffer::with_capacity(16);
        ClientHeldItemChange { slot: 9 }.encode(&mut cbuf).unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x34,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        let inv = app
            .world()
            .get::<PlayerInventory>(entity)
            .expect("玩家应挂载 PlayerInventory");
        assert_eq!(inv.held_slot, 0, "越界槽不应改变默认手持槽");

        let flushed = capture_payloads(&mut out_rx);
        assert!(
            !flushed.iter().any(|p| decode_held_item_change(p).is_some()),
            "越界槽切换后不应回发任何 clientbound HeldItemChange"
        );
    }

    /// T4：注入命令聊天包（0x06, message="help"）经 SystemChatPacket(0x77) 回发命令列表。
    /// 先注册一个额外命令使 help 列表非空（内置 help 自身被排除）。
    /// 见 .specs/implement-command-framework/。
    #[test]
    fn command_chat_help_lists_commands() {
        let (mut app, frame_tx, mut out_rx, _entity) = connect_to_play(1);

        app.world_mut()
            .resource_mut::<CommandManager>()
            .unwrap()
            .register(Command::new("warp", &[]).description("传送点"))
            .expect("注册 warp 应成功");

        // 清空进入 Play 阶段已缓冲的出站帧
        let _ = capture_payloads(&mut out_rx);

        let mut cbuf = ByteBuffer::with_capacity(32);
        ClientCommandChatPacket {
            message: "help".to_string(),
        }
        .encode(&mut cbuf)
        .unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x06,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        let flushed = capture_payloads(&mut out_rx);
        // help 逐条命令生成一个独立 SystemChatPacket（字母序：status/stop 排在 warp 前）；
        // 聚合全部聊天包文本后再断言，避免仅取首个包而漏判 warp。
        let help_text: String = flushed
            .iter()
            .filter_map(|p| decode_system_chat(p))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            help_text.contains("warp"),
            "help 列表应含 warp 命令，实际：{help_text}"
        );
    }

    /// T4：注入未知命令（0x06, message="foobar"）→ 回发错误 SystemChatPacket（含 "foobar"），不 panic。
    /// 见 .specs/implement-command-framework/。
    #[test]
    fn command_chat_unknown_emits_error() {
        let (mut app, frame_tx, mut out_rx, _entity) = connect_to_play(1);
        let _ = capture_payloads(&mut out_rx);

        let mut cbuf = ByteBuffer::with_capacity(32);
        ClientCommandChatPacket {
            message: "foobar".to_string(),
        }
        .encode(&mut cbuf)
        .unwrap();
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x06,
                payload: cbuf.into_inner(),
            })
            .unwrap();
        app.update();

        let flushed = capture_payloads(&mut out_rx);
        let msg = flushed
            .iter()
            .find_map(|p| decode_system_chat(p))
            .expect("未知命令应回发 SystemChatPacket(0x77)");
        assert!(
            msg.contains("foobar"),
            "错误信息应含原输入 foobar，实际：{msg}"
        );
    }

    /// T4：注入普通聊天（0x08，不在命令范围）→ 静默忽略，不回发任何 SystemChatPacket。
    /// 见 .specs/implement-command-framework/。
    #[test]
    fn command_chat_0x08_ignored_no_reply() {
        let (mut app, frame_tx, mut out_rx, _entity) = connect_to_play(1);
        let _ = capture_payloads(&mut out_rx);

        // 0x08 无定义结构，发送空 payload（dispatch 直接 Ignored）。
        frame_tx
            .try_send(RawFrame::Packet {
                conn_id: 1,
                state: ConnectionState::Play,
                packet_id: 0x08,
                payload: Vec::new(),
            })
            .unwrap();
        app.update();

        let flushed = capture_payloads(&mut out_rx);
        assert!(
            !flushed.iter().any(|p| decode_system_chat(p).is_some()),
            "0x08 不应回发任何 SystemChatPacket"
        );
    }
}
