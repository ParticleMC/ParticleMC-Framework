// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 自建 `App` 装配抽象（替代 自建 App，RM1）。
//!
//! 变更标识符：`implement-custom-ecs`（T10）
//!
//! particlemc-framework-ecs 不提供 `App`：本模块用 `World` + `Schedule` 组合出自研 `App`，
//! 保留 旧 ECS 方案 的 `Plugin` 装配语义（`build(&self, app: &mut App)`）与常用方法
//! （`add_plugins` / `init_resource` / `insert_resource` / `add_system` /
//! `add_systems` / `add_message` / `after` / `update` / `world` / `world_mut` /
//! `contains_resource` / `run_schedule`），使 `McServerPlugin` 装配层几乎无需
//! 改变形态（T10.5）。

use std::any::type_name_of_val;
use std::time::Duration;

use particlemc_framework_ecs::message::{Message, MessageInbox};
use particlemc_framework_ecs::resource::Resource;
use particlemc_framework_ecs::schedule::Schedule;
use particlemc_framework_ecs::system::IntoSystem;
use particlemc_framework_ecs::world::World;

/// 阶段标记：对应 旧 ECS 方案 的 `Update`（每帧运行的主阶段）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Update;

/// 阶段标记：对应 旧 ECS 方案 的 `FixedUpdate`（固定步长阶段，本框架即 20Hz tick）。
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedUpdate;

/// 时间推进策略（旧 ECS 方案 `TimeUpdateStrategy` 对齐）。
///
/// 仅 `ManualDuration` 被 `App::update` 消费：每轮 update 按指定 `dt` 推进
/// 固定步长时钟，使测试可确定性步进（与 旧 ECS 方案 `Time<Fixed>` 等效）。未设置
/// 或 `RealTime` 时退化为单轮 `run`（与旧行为一致）。
#[derive(Clone, Copy, Debug)]
pub enum TimeUpdateStrategy {
    /// 每轮 update 手动推进固定时长。
    ManualDuration(Duration),
    /// 真实墙钟时间（本框架不自动采集，暂等价于不额外推进）。
    RealTime,
}

impl TimeUpdateStrategy {
    /// 本策略对应的每轮 `dt`；`RealTime` 回退到零。
    pub fn dt(&self) -> Duration {
        match self {
            crate::app::TimeUpdateStrategy::ManualDuration(d) => *d,
            TimeUpdateStrategy::RealTime => Duration::ZERO,
        }
    }
}

/// 插件装配 trait（对应 旧 ECS 方案 `Plugin`）。
///
/// `build` 接收 `&mut App`，往其中的 `World` 注入资源、往 `Schedule` 注册系统
/// 与消息，建立依赖顺序。
pub trait Plugin {
    /// 将插件内容装配进 `app`。
    fn build(&self, app: &mut App);
}

/// particlemc-framework-core 自建应用容器：包裹一个 [`World`] 与一个 [`Schedule`]。
pub struct App {
    /// 资源与世界状态（实体、组件、共享单例）。
    pub world: World,
    /// 20Hz tick 管线调度器。
    pub schedule: Schedule,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// 构造空应用（World + 默认 20Hz Schedule）。
    pub fn new() -> Self {
        App {
            world: World::new(),
            schedule: Schedule::new(),
        }
    }

    /// 装配一个插件（`Plugin::build`）。
    pub fn add_plugins<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        plugin.build(self);
        self
    }

    /// 以 `Default` 初始化一个资源（若已存在则跳过，旧 ECS 方案 `init_resource` 语义）。
    pub fn init_resource<R>(&mut self) -> &mut Self
    where
        R: Resource + Default,
    {
        self.world.init_resource::<R>();
        self
    }

    /// 插入一个资源（覆盖既有同类型，旧 ECS 方案 `insert_resource` 语义）。
    pub fn insert_resource<R>(&mut self, r: R) -> &mut Self
    where
        R: Resource,
    {
        self.world.insert_resource(r);
        self
    }

    /// 注册一个消息类型（tick 末统一清空 inbox，R6.3）。
    ///
    /// 同时立即预注册 `MessageInbox<M>` 资源，使 `World::write` 在首轮
    /// `update` 之前即可安全投递（旧 ECS 方案 `add_event` 语义对应）。
    pub fn add_message<M>(&mut self) -> &mut Self
    where
        M: Message,
    {
        self.schedule.add_message::<M>();
        self.world.init_resource::<MessageInbox<M>>();
        self
    }

    /// 注册一个借用系统（0-4 参数函数/闭包，经 `IntoSystem` 转换）。
    pub fn add_system<M>(&mut self, sys: impl IntoSystem<M> + 'static) -> &mut Self {
        self.schedule.add_system(sys);
        self
    }

    /// `add_system` 的别名（旧 ECS 方案 `add_systems` 语义对应）。
    pub fn add_systems<M>(&mut self, sys: impl IntoSystem<M> + 'static) -> &mut Self {
        self.schedule.add_systems(sys);
        self
    }

    /// 建立依赖：`later` 系统在 `earlier` 系统之后执行。
    ///
    /// 接收两个系统函数**值**，以 `type_name_of_val` 取其与 `add_system` 注册
    /// 一致的 `type_name` 字符串（`FunctionSystem::name` 即 `type_name::<F>`），
    /// 无需手写字符串，也规避了「函数/模块名无法直接用于 turbofish 类型参数」
    /// 的限制（旧 ECS 方案 `system::a.after(system::b)` 的等价写法）。
    pub fn after<F1, F2>(&mut self, later: F1, earlier: F2) -> &mut Self
    where
        F1: 'static,
        F2: 'static,
    {
        self.schedule
            .after(type_name_of_val(&later), type_name_of_val(&earlier));
        self
    }

    /// 运行一轮完整 tick（驱动 `Schedule::run`）。
    ///
    /// 若已注册 `TimeUpdateStrategy` 资源且 `dt > 0`，则按其推进固定步长时钟
    /// 并按步数跑多轮（测试确定性步进）；否则退化为单轮 `run`。
    pub fn update(&mut self) {
        if let Some(strategy) = self.world.resource::<TimeUpdateStrategy>().copied() {
            let dt = strategy.dt();
            if dt > Duration::ZERO {
                let steps = self.schedule.tick_clock(dt);
                if steps == 0 {
                    self.schedule.run(&mut self.world);
                } else {
                    for _ in 0..steps {
                        self.schedule.run(&mut self.world);
                    }
                }
                return;
            }
        }
        self.schedule.run(&mut self.world);
    }

    /// 按阶段运行调度（本框架仅 `Schedule` 一个主阶段）。
    pub fn run_schedule(&mut self, _stage: FixedUpdate) {
        self.schedule.run(&mut self.world);
    }

    /// 不可变访问世界。
    pub fn world(&self) -> &World {
        &self.world
    }

    /// 可变访问世界。
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// 世界是否含有某资源（旧 ECS 方案 `World::contains_resource` 语义）。
    pub fn contains_resource<R>(&self) -> bool
    where
        R: Resource,
    {
        self.world.contains_resource::<R>()
    }
}
