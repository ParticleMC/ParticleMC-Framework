// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! `McServerPlugin`：组装 Minestom 服务器内核的自研插件（替代旧 ECS 方案 插件，RM1）。
//!
//! 负责把全部 Manager 类 `Resource`、全部注册表、全部 `Message`（事件）以及
//! tick 管线系统装配进自建 `App`（包裹 `World` + `Schedule`），并把固定步长
//! 配置为 20Hz。插件可在无真实网络环境下构建 `App`，骨架阶段不启动任何监听。
//! 框架不生成任何世界内容（出生平台等），世界与出生配置由应用侧通过
//! `InstanceManager` / `SpawnConfig` 装配。

use std::path::PathBuf;

use crate::app::{App, Plugin};
use crate::event;
use crate::event::bus::EventBus;
#[cfg(feature = "wasm-extensions")]
use crate::extension::ExtensionLoader;
use crate::network::bridge::empty_bridge;
use crate::network::client::ClientNetworks;
use crate::resource::command::Command;
use crate::resource::registries::nbt::RegistrySnapshot;
use crate::resource::registries::{
    BiomeRegistry, BlockRegistry, DimensionTypeRegistry, EnchantmentRegistry, EntityTypeRegistry,
    FluidRegistry, ItemRegistry, ParticleRegistry, PotionEffectRegistry, SoundEventRegistry,
    TagRegistry,
};
use crate::resource::velocity_config::VelocityConfig;
use crate::resource::{
    AttributeRegistry, CommandManager, CompressionConfig, ConnectionManager, DamageTypeRegistry,
    EntitySpawner, GenericRegistry, InstanceManager, LootTableRegistry, SpawnConfig, StatusConfig,
    TaskScheduler,
};
use crate::schedule::configure_20hz;
use crate::system;

/// 返回 Minestom 数据目录的默认位置（相对当前 crate 的 `CARGO_MANIFEST_DIR`）。
fn default_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/data")
}

/// Minestom 服务器插件。
///
/// - 默认构造（[`McServerPlugin::new`]）：加载核心注册表（方块 / 物品 / 实体类型）。
/// - [`McServerPlugin::with_preload`]：额外加载全部世界类注册表与标签。
pub struct McServerPlugin {
    /// 是否加载全部注册表（含世界类与标签）。
    load_all: bool,
    /// 注册数据目录。
    data_dir: PathBuf,
}

impl Default for McServerPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl McServerPlugin {
    /// 以默认数据目录构造插件（仅加载核心注册表）。
    pub fn new() -> Self {
        Self {
            load_all: false,
            data_dir: default_data_dir(),
        }
    }

    /// 构造插件并启用全量注册表预热。
    pub fn with_preload() -> Self {
        Self {
            load_all: true,
            data_dir: default_data_dir(),
        }
    }

    /// 覆盖注册数据目录（主要用于测试）。
    pub fn with_data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = dir;
        self
    }
}

impl Plugin for McServerPlugin {
    fn build(&self, app: &mut App) {
        // 1. 时间：固定步长配置为 20Hz（自研 FixedClock，无需 旧 ECS 方案 TimePlugin）。
        configure_20hz(app);

        // 1.5 WASM 扩展加载（WS4-T2）：启用 `wasm-extensions` feature 时扫描 `extensions/` 目录并加载。
        #[cfg(feature = "wasm-extensions")]
        {
            use crate::extension::ExtensionManager;
            let loader = ExtensionLoader::new();
            let mut manager = ExtensionManager::new();
            let ext_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("extensions");
            if ext_dir.exists() {
                for entry in std::fs::read_dir(&ext_dir).unwrap_or_else(|e| {
                    eprintln!("无法扫描扩展目录 {ext_dir:?}: {e}");
                    std::fs::read_dir(".").unwrap()
                }) {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            eprintln!("读取扩展目录条目失败: {e}");
                            continue;
                        }
                    };
                    let path = entry.path();
                    if path.extension().map_or(false, |ext| ext == "wasm") {
                        if let Some(wrapped) = loader.load(&path) {
                            // 将包装后的实例注册到管理器（ExtensionManager 持有列表供 tick_all 调用）。
                            if manager.register(wrapped).is_err() {
                                eprintln!(
                                    "扩展注册失败：超出 MAX_EXTENSIONS（{}）",
                                    crate::extension::MAX_EXTENSIONS
                                );
                            }
                        }
                    }
                }
            }
            app.insert_resource(manager);
        }

        // 2. Manager 类 Resource（占位）。
        app.init_resource::<ConnectionManager>()
            .init_resource::<InstanceManager>()
            .init_resource::<CommandManager>()
            .init_resource::<TaskScheduler>();

        // 13.3 内置运维命令：help 已由 `CommandManager` 内置；stop / status 在此注册
        // 以便 `help` 列表与 `command_exists` 一致。实际行为由控制台（`console::dispatch`）
        // 拦截处理，注册仅为可见性与一致性。
        if let Some(mgr) = app.world_mut().resource_mut::<CommandManager>() {
            let _ = mgr.register(Command::new("stop", &[]).description("优雅停机服务器"));
            let _ = mgr
                .register(Command::new("status", &[]).description("显示 tick 耗时/实体数/世界数"));
        }

        // 2.0 框架能力 Resource：事件总线、实体生成器、调度时钟。
        //     见 `.specs/implement-framework-capabilities/`。
        app.init_resource::<EventBus>()
            .init_resource::<EntitySpawner>()
            .init_resource::<system::TickCounter>()
            // 属性同步收件箱（R8）：初始为空即天然不主动下发（登录/生成不发属性包）。
            .init_resource::<system::AttributeInbox>();

        // 2.1 网络侧 Resource：空桥接（真实监听由二进制入口覆盖）、发包表、转发配置。
        //     插件内提供默认以便单元测试可直接 `app.update()`；真实服务器在入口处覆盖桥接。
        app.insert_resource(empty_bridge().0);
        app.init_resource::<ClientNetworks>();
        app.init_resource::<VelocityConfig>();

        // 2.1.5 在线认证上下文（WS5b）：feature 开启时生成 RSA 密钥、建立待验证通道并
        //       启动异步 worker；feature 关闭时仅注入默认（enabled=false）占位资源，
        //       保持离线语义零开销（网络侧 `network_receive` 据 `enabled` 跳过握手）。
        #[cfg(feature = "online-auth")]
        {
            use crate::crypto::{OnlineAuthContext, PendingAuth, run_auth_worker};
            use crate::network::bridge::NetworkBridge;
            use tokio::sync::mpsc::unbounded_channel;

            let mut ctx = OnlineAuthContext::generate();
            let (tx, rx) = unbounded_channel::<PendingAuth>();
            ctx.pending_tx = Some(tx);
            let url = ctx.has_joined_url.clone();
            let timeout = ctx.timeout;
            let der = ctx.private_key_der.clone();
            app.insert_resource(ctx);

            // 启动异步验证 worker（需 tokio runtime；无 runtime 的纯单元测试场景跳过，
            // 此时在线认证实际不工作，但默认构建门禁不涉及该 feature）。
            // `NetworkBridge` 由上方 `empty_bridge().0` 注入，此处用 `if let Some` 取值，
            // 避免在生产路径使用被 clippy 门禁禁止的 `unwrap`/`expect`。
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                if let Some(bridge) = app.world().resource::<NetworkBridge>() {
                    let outbound = bridge.outbound.clone();
                    handle.spawn(async move {
                        run_auth_worker(rx, outbound, url, timeout, der).await;
                    });
                }
            }
        }
        #[cfg(not(feature = "online-auth"))]
        {
            use crate::crypto::OnlineAuthContext;
            app.init_resource::<OnlineAuthContext>();
        }

        // 2.2 应用配置 Resource：状态响应（MOTD）与出生点，默认值可直接使用，
        //     应用侧可覆盖。压缩配置读取环境变量 `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD`。
        app.init_resource::<StatusConfig>();
        app.init_resource::<SpawnConfig>();
        app.insert_resource(CompressionConfig::from_env());

        // 3. 核心注册表（方块 / 物品 / 实体类型）始终加载。
        app.insert_resource(
            BlockRegistry::from_toml_file(&self.data_dir.join("blocks.toml")).unwrap_or_default(),
        )
        .insert_resource(
            ItemRegistry::from_toml_file(&self.data_dir.join("items.toml")).unwrap_or_default(),
        )
        .insert_resource(
            EntityTypeRegistry::from_toml_file(&self.data_dir.join("entity_types.toml"))
                .unwrap_or_default(),
        );

        // 3.1 通用注册表与战利品表：始终加载（R4），覆盖 generic/ 与 loot_tables/ 全部注册数据。
        //     目录缺失或解析失败时回退为空表（unwrap_or_default），不 panic。
        app.insert_resource(
            GenericRegistry::load_directory(&self.data_dir.join("generic")).unwrap_or_default(),
        )
        .insert_resource(
            LootTableRegistry::load_directory(&self.data_dir.join("loot_tables"))
                .unwrap_or_default(),
        );

        // 3.2 属性注册表（R8）：始终加载，`attributes.toml` 缺失或解析失败时回退为空表。
        app.insert_resource(
            AttributeRegistry::from_toml_file(&self.data_dir.join("attributes.toml"))
                .unwrap_or_default(),
        );

        // 4. 世界类注册表与标签：仅在全量预热时加载。
        if self.load_all {
            app.insert_resource(
                BiomeRegistry::from_toml_file(&self.data_dir.join("biomes.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                DimensionTypeRegistry::from_toml_file(&self.data_dir.join("dimension_types.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                FluidRegistry::from_toml_file(&self.data_dir.join("fluids.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                ParticleRegistry::from_toml_file(&self.data_dir.join("particles.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                SoundEventRegistry::from_toml_file(&self.data_dir.join("sound_events.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                DamageTypeRegistry::from_toml_file(&self.data_dir.join("damage_types.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                EnchantmentRegistry::from_toml_file(&self.data_dir.join("enchantments.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                PotionEffectRegistry::from_toml_file(&self.data_dir.join("potion_effects.toml"))
                    .unwrap_or_default(),
            )
            .insert_resource(
                TagRegistry::load_directory(&self.data_dir.join("tags")).unwrap_or_default(),
            );
        } else {
            app.init_resource::<BiomeRegistry>()
                .init_resource::<DimensionTypeRegistry>()
                .init_resource::<FluidRegistry>()
                .init_resource::<ParticleRegistry>()
                .init_resource::<SoundEventRegistry>()
                .init_resource::<DamageTypeRegistry>()
                .init_resource::<EnchantmentRegistry>()
                .init_resource::<PotionEffectRegistry>()
                .init_resource::<TagRegistry>();
        }

        // 4.1 注册表快照（配置阶段注册表同步数据）：从数据目录构建。
        app.insert_resource(RegistrySnapshot::from_data_dir(&self.data_dir));

        // 5. 注册全部事件（自研 Message）。
        app.add_message::<event::BlockBreak>()
            .add_message::<event::BlockPlace>()
            .add_message::<event::PlayerJoin>()
            .add_message::<event::PlayerQuit>()
            .add_message::<event::PlayerMove>()
            .add_message::<event::EntityDamage>()
            .add_message::<event::EntityDeath>()
            .add_message::<event::NetworkEvent>()
            .add_message::<event::EnterPlayEvent>()
            .add_message::<event::PlayerBlockInteract>()
            .add_message::<event::EntitySpawn>()
            .add_message::<event::EntityRemove>()
            .add_message::<event::PlayerChat>()
            .add_message::<event::BlockUpdate>()
            .add_message::<event::EntityMove>()
            // 框架动作事件（implement-framework-capabilities T28）：由
            // `packet_action_system` 消费收件箱后写入消息收件箱，
            // 经 `block_interaction_validator` 校验后派发至 EventBus。
            .add_message::<event::EntityInteract>()
            .add_message::<event::PlayerActionEvent>()
            .add_message::<event::BlockInteractionRejected>()
            .add_message::<event::PlayerUseItem>()
            .add_message::<event::PlayerAnimation>();

        // PacketSendEvent 已弃用（发包统一走 ClientNetwork 队列），此处仅保留注册
        // 以兼容 `tick_end` 等旧引用，故局部豁免弃用警告。
        #[allow(deprecated)]
        app.add_message::<event::PacketSendEvent>();

        // 6. 注册 20Hz tick 管线，并通过 `after` 固定执行顺序。
        //    网络接收置于最前（推进连接状态、落玩家实体、触发进入 Play），
        //    chunk_send / entity_sync 紧随其后（消费 EnterPlayEvent），
        //    网络发送置于最后（flush 出站队列）。
        app.add_system(system::network_receive);
        app.add_system(system::command_chat_system)
            .after(system::command_chat_system, system::network_receive);
        app.add_system(system::packet_action_system)
            .after(system::packet_action_system, system::network_receive);
        app.add_system(system::block_interaction_validator)
            .after(system::block_interaction_validator, system::packet_action_system);
        app.add_system(system::chunk_send)
            .after(system::chunk_send, system::network_receive);
        app.add_system(system::entity_sync)
            .after(system::entity_sync, system::network_receive);
        app.add_system(system::inventory_sync)
            .after(system::inventory_sync, system::entity_sync);
        app.add_system(system::tick_begin)
            .after(system::tick_begin, system::entity_sync);
        app.add_system(system::scheduler_tick)
            .after(system::scheduler_tick, system::tick_begin);
        app.add_system(system::player_input)
            .after(system::player_input, system::scheduler_tick);
        // R11.2：player_movement / entity_ai / physics 已迁入实例 World Schedule
        // （见 `build_instance_world`），不再于主 World 驱动；故 chunk_dirty_sync
        // 的先序改为 player_input（原 physics 先序已随迁移移除）。
        app.add_system(system::chunk_dirty_sync)
            .after(system::chunk_dirty_sync, system::player_input);
        app.add_system(system::tick_end)
            .after(system::tick_end, system::chunk_dirty_sync);
        app.add_system(system::attribute_sync)
            .after(system::attribute_sync, system::tick_end);
        app.add_system(system::registry_sync)
            .after(system::registry_sync, system::attribute_sync);
        app.add_system(system::network_send)
            .after(system::network_send, system::attribute_sync)
            .after(system::network_send, system::tick_end)
            .after(system::network_send, system::command_chat_system)
            .after(system::network_send, system::packet_action_system);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn app_smoke_test_does_not_panic() {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());
        app.update();
    }

    #[test]
    fn fixed_update_pipeline_order_is_correct() {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::new());
        // 运行一次以完成调度初始化（recompute_order），确保 systems() 可见执行顺序。
        app.update();

        let names: Vec<String> = app
            .schedule
            .systems()
            .map(|(name, _)| name.to_string())
            .collect();

        // R11.2：实体相关系统（player_movement / entity_ai / physics）已迁入实例
        // World 的实例调度器，不再出现在主 World 管线中；此处仅断言主 World 仍按
        // 预期顺序装配下列系统。
        let expected = [
            "network_receive",
            "tick_begin",
            "player_input",
            "chunk_dirty_sync",
            "tick_end",
            "network_send",
        ];

        let mut previous: Option<usize> = None;
        for name in expected {
            let position = names
                .iter()
                .position(|system_name| system_name.contains(name));
            assert!(position.is_some(), "tick 管线缺少系统 {name}");
            let position = position.unwrap();
            if let Some(prev) = previous {
                assert!(position > prev, "系统顺序错误：{name} 未排在预期位置之后");
            }
            previous = Some(position);
        }
    }

    #[test]
    fn preload_plugin_loads_world_registries_without_panic() {
        let mut app = App::new();
        app.add_plugins(McServerPlugin::with_preload());
        app.update();
        // 核心注册表应已注入 World。
        assert!(app.world().contains_resource::<BlockRegistry>());
        assert!(app.world().contains_resource::<ItemRegistry>());
        assert!(app.world().contains_resource::<EntityTypeRegistry>());
    }
}
