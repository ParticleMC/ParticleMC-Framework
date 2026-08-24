//! Minestom（Rust 重写版）服务器入口库。
//!
//! 暴露 [`run`] 供二进制入口与集成测试复用：启动 tokio 运行时、装配自研 `App`、
//! 绑定真实 TCP 监听并进入 20Hz 主循环。`network_receive` / `network_send` 系统
//! 在每 tick 内衔接异步监听与同步游戏循环。

// 测试代码允许使用 unwrap/expect（生产代码仍受 `-D clippy::unwrap_used` 约束）。
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::net::SocketAddr;
use std::sync::Arc;

use particlemc_framework_core::app::App;
use tokio::runtime::Builder;

use particlemc_framework_core::console::{Console, dispatch};
use particlemc_framework_core::network::bridge::{NetworkBridge, empty_bridge};
use particlemc_framework_core::network::client::ClientNetworks;
use particlemc_framework_core::network::listener::ConnectionListener;
use particlemc_framework_core::plugin::McServerPlugin;
use particlemc_framework_core::resource::InstanceManager;
use particlemc_framework_core::resource::compression_config::CompressionConfig;
use particlemc_framework_core::resource::instance_manager::{SharedRegistries, build_instance_world};
use particlemc_framework_core::resource::registries::{
    BlockRegistry, EntityTypeRegistry, GenericRegistry, ItemRegistry,
};
use particlemc_framework_core::resource::velocity_config::VelocityConfig;
use particlemc_framework_ecs::scheduler::{InstanceScheduler, SchedulerConfig, ThreadMode, TickStats};

/// 启动并运行 ParticleMCFramework 服务器，直到进程退出。
///
/// - 创建多线程 tokio 运行时承载异步监听任务；
/// - 装配 旧 ECS 方案 `App`（含 20Hz tick 管线；出生平台等世界内容由应用侧提供）；
/// - 绑定 `addr` 的真实 TCP 监听；
/// - 主线程以 ~50ms 步长驱动 `app.update()`，构成游戏主循环。
pub fn run(addr: SocketAddr) -> std::io::Result<()> {
    let rt = Builder::new_multi_thread().enable_all().build()?;

    let mut app = App::new();
    app.add_plugins(McServerPlugin::with_preload());

    // 构造真实桥接（入站通道 + 出站表），覆盖插件插入的占位桥接。
    let (bridge_placeholder, inbound_tx, outbound) = empty_bridge();
    let bridge = NetworkBridge::new(bridge_placeholder.inbound, outbound.clone());
    app.insert_resource(bridge);
    // 压缩阈值与 `CompressionConfig` 保持一致（读取 `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD`，
    // 默认 256；`run` 保持读 env，测试在 spawn 前显式设置该环境变量，见 T7 集成测试）。
    let compression_threshold = CompressionConfig::from_env().threshold;
    app.insert_resource(ClientNetworks::with_compression_threshold(
        compression_threshold,
    ));
    app.insert_resource(VelocityConfig::load());

    // 应用侧装配最小默认世界（空实例）：框架不生成地形内容（出生平台等），
    // 但玩家登录后必须落入一个真实实例，chunk_send / entity_sync 才有可操作对象。
    // 实例 World 由 `InstanceScheduler` 托管（R11），区块数据存于实例 World 的
    // `ChunkStore`；实际世界由应用通过 `InstanceManager` / `SpawnConfig` 装配。

    // 跨实例共享只读注册表（R11.5 / 12.5）：由主 World 已注入的 4 个注册表克隆
    // 出 `Arc` 值，零拷贝注入实例 World，供实体迁移后其内部系统只读访问。
    // `World::resource` 返回 `Option<&T>`，生产代码禁止 unwrap，故以 `let else`
    // 在缺失时提前以 io 错误返回（注册表由 McServerPlugin 保证注入，正常不可达）。
    let Some(block) = app.world().resource::<BlockRegistry>() else {
        return Err(std::io::Error::other("BlockRegistry 未注入主 World"));
    };
    let Some(item) = app.world().resource::<ItemRegistry>() else {
        return Err(std::io::Error::other("ItemRegistry 未注入主 World"));
    };
    let Some(entity_type) = app.world().resource::<EntityTypeRegistry>() else {
        return Err(std::io::Error::other("EntityTypeRegistry 未注入主 World"));
    };
    let Some(generic) = app.world().resource::<GenericRegistry>() else {
        return Err(std::io::Error::other("GenericRegistry 未注入主 World"));
    };
    let shared = SharedRegistries {
        block: Arc::new(block.clone()),
        item: Arc::new(item.clone()),
        entity_type: Arc::new(entity_type.clone()),
        generic: Arc::new(generic.clone()),
    };

    let mut scheduler = InstanceScheduler::new(SchedulerConfig {
        thread_mode: ThreadMode::SharedPool,
        worker_count: 4,
        affinity: false,
    });
    let default_world = build_instance_world(&mut scheduler, None, None, &shared);
    app.insert_resource(scheduler);
    let Some(im) = app.world_mut().resource_mut::<InstanceManager>() else {
        return Err(std::io::Error::other("InstanceManager 未注入主 World"));
    };
    im.set_default(default_world);

    // 在 tokio 运行时内启动监听循环（绑定 + 每连接读写任务）。
    rt.spawn(async move {
        if let Err(e) = ConnectionListener::start(addr, inbound_tx, outbound).await {
            eprintln!("[listener] 监听启动失败：{e}");
        }
    });

    println!("ParticleMCFramework (Rust) 监听于 {addr}（20Hz tick）");

    // 启动控制台：独立 stdin 线程读取命令，主循环取出后分发（禁止控制台线程直接
    // 读写 World，见 `console` 模块文档，T13）。
    let console = Console::start();

    loop {
        app.update();
        // 驱动全部实例 World 并行 tick（R11 / IC-10）：主世界阶段（含 physics /
        // chunk_send 等跨世界系统）结束后再 tick 实例 World，二者不重叠持锁。
        let stats = if let Some(sched) = app.world_mut().resource_mut::<InstanceScheduler>() {
            sched.tick_all()
        } else {
            TickStats { worlds: Vec::new() }
        };
        // 刷新 tick 统计快照（best-effort：锁被毒化则跳过，不 panic）。
        if let Ok(mut g) = console.state().stats.lock() {
            *g = Some(stats);
        }

        // 分发控制台命令（运行于主线程，可安全访问主 World 的 CommandManager 与调度器）。
        for line in console.drain_input() {
            match app.world().resource::<InstanceScheduler>() {
                Some(sched) => {
                    let out = dispatch(&line, app.world(), sched, console.state());
                    for l in out {
                        println!("[console] {l}");
                    }
                }
                None => println!("[console] 调度器不可用，无法执行命令：{line}"),
            }
        }

        // 优雅停机：stop 命令置位停机信号后退出主循环。
        if console.should_shutdown() {
            println!("[console] 收到停机信号，正在退出主循环");
            break Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
