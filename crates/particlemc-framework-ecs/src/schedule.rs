// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 世界内调度器：系统注册、依赖排序、固定步长时钟。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! [`Schedule`] 对齐 旧 ECS 方案 语义（IC-9 / R7）：`add_system`（链式 `.after` 建立
//! 严格偏序）→ 注册期/首次运行期拓扑排序（Kahn 分层，环检测 debug_assert）→
//! `run` 按依赖序执行。每 tick 执行流程：
//!
//! 1. **命令应用**：每系统执行后即时 `apply` 缓冲（旧 ECS 方案 `ApplyDeferred` 语义，
//!    使同 tick 内 `Commands` 生产者对后续系统可见），tick 起始再补应用上一
//!    tick 残留/外部提交命令；remove/insert 规避借用冲突（R5.1）；
//! 2. **按序执行系统**：排他系统（`fn(&mut World)`）与其他系统互斥；借用
//!    系统内部经 `SystemParam::get` 自 `&mut World` 拆借参数（A9）；
//! 3. **tick 末清空消息**：对注册的每种 `Message` 的 inbox 调用 `clear`
//!    （R6.3 tick 内生命周期）。
//!
//! 并行分组（R7.3，默认关闭）：`set_parallel(true)` 时按依赖关系分层，同层
//! 系统互不依赖；**当前实现仍单线程串行执行**（同层按注册序），真正的多线程
//! 并行由 T16 实现（届时经 `Arc<World>` + 分片），接口与分组计算先行就位。
//!
//! 固定步长时钟：独立 [`FixedClock`]（accumulator 累计 dt，达 1/hz 步进）；
//! `Schedule::tick_clock` 内嵌一枚时钟（默认 20Hz，`set_fixed_hz` 可调），
//! 返回本轮应执行次数——T10 particlemc-framework-core 每帧 `dt` 推进并据此调用 `run`。

use std::collections::HashMap;
use std::time::Duration;

use crate::commands::{CommandBuffer, Commands};
use crate::message::{Message, MessageInbox};
use crate::system::{ExclusiveMarker, IntoSystem, System};
use crate::world::World;

/// 系统标签：`.after` 依赖关联用（按系统名匹配）。
pub trait SystemLabel {
    /// 标签字符串（系统名）。
    fn label(&self) -> String;
}

impl SystemLabel for &'static str {
    fn label(&self) -> String {
        (*self).to_string()
    }
}

impl SystemLabel for String {
    fn label(&self) -> String {
        self.clone()
    }
}

/// 固定步长时钟（R7.4）：以指定频率（Hz）累计 `dt`，满一步执行一轮。
///
/// 与 自研时钟 的 `Fixed` 对齐：`accumulator += dt`，超过 `1/hz` 即产生
/// 一次步进（返回执行次数，余数保留滚入下轮）。除法取整避免循环累加，
/// 单次调用 O(1)。
pub struct FixedClock {
    /// 固定步长频率（次/秒）。
    pub hz: f64,
    /// 未消耗的时间余量（跨 tick 保留）。
    accumulator: Duration,
}

impl FixedClock {
    /// 以指定频率构造时钟。
    ///
    /// # Panics
    ///
    /// debug 构建下 `hz` 非正或非有限时断言失败（非法频率属装配错误）；
    /// release 下以最近合法值（1.0）兜底。
    pub fn from_hz(hz: f64) -> Self {
        debug_assert!(hz > 0.0 && hz.is_finite(), "固定步长频率必须为正有限数");
        FixedClock {
            hz: if hz > 0.0 && hz.is_finite() { hz } else { 1.0 },
            accumulator: Duration::ZERO,
        }
    }

    /// 推进时钟：累计 `dt`，返回本轮应执行的步数（余数保留）。
    pub fn tick(&mut self, dt: Duration) -> usize {
        let step_ns = step_nanos(self.hz);
        self.accumulator += dt;
        let total_ns = self.accumulator.as_nanos();
        let n = total_ns / step_ns;
        if n > 0 {
            // 余数 < step_ns 滚入下轮；极端低 hz 下余数超 u64 表示范围时饱和
            let rem_ns = total_ns % step_ns;
            let rem = u64::try_from(rem_ns).unwrap_or(u64::MAX);
            self.accumulator = Duration::from_nanos(rem);
        }
        match usize::try_from(n) {
            Ok(v) => v,
            // 步数超出 usize 表示范围：饱和（物理上不可达，仅形式性兜底）
            Err(_) => usize::MAX,
        }
    }

    /// 是否该跑一轮（恰好消耗一步，不累计多步）。
    pub fn step(&mut self) -> bool {
        let step = Duration::from_nanos(step_nanos(self.hz) as u64);
        if self.accumulator >= step {
            self.accumulator -= step;
            true
        } else {
            false
        }
    }

    /// 清零累计余量（切换频率/场景重启时）。
    pub fn reset(&mut self) {
        self.accumulator = Duration::ZERO;
    }

    /// 仅累计时间、不消费步数（充能入口）。
    ///
    /// 与 [`tick`](Self::tick)（累计并消费步数）互补：单步消费模式先
    /// `advance(dt)` 充能，再 `while clock.step() { run(); }` 逐次消耗，
    /// 二者不可混用（`tick` 会清零已累计余量）。
    pub fn advance(&mut self, dt: Duration) {
        self.accumulator += dt;
    }
}

/// 单步时长（纳秒）：`1e9 / hz`，作为 u128 以容纳极端低 hz。
fn step_nanos(hz: f64) -> u128 {
    (1_000_000_000.0 / hz) as u128
}

/// 世界内调度器（IC-9）。
pub struct Schedule {
    /// 已注册系统（注册序）。
    systems: Vec<Box<dyn System>>,
    /// 串行执行序（拓扑序，`groups` 展平）。
    order: Vec<usize>,
    /// 并行分组（拓扑分层：同层无依赖，可并行）。
    groups: Vec<Vec<usize>>,
    /// 名字级依赖（later, earlier）：recompute 时解析为下标。
    deps: Vec<(String, String)>,
    /// tick 末清空消息 inbox 的类型擦除钩子（add_message 注册）。
    #[allow(clippy::type_complexity)]
    clear_hooks: Vec<Box<dyn Fn(&mut World) + Send + Sync>>,
    /// 并行分组开关（R7.3，默认关闭）。
    parallel: bool,
    /// order/groups 是否需要重算（注册/依赖变更后置脏）。
    order_dirty: bool,
    /// 内嵌固定步长时钟（tick_clock 驱动，默认 20Hz）。
    clock: FixedClock,
}

impl Schedule {
    /// 空调度器（20Hz 时钟）。
    pub fn new() -> Self {
        Schedule {
            systems: Vec::new(),
            order: Vec::new(),
            groups: Vec::new(),
            deps: Vec::new(),
            clear_hooks: Vec::new(),
            parallel: false,
            order_dirty: false,
            clock: FixedClock::from_hz(20.0),
        }
    }

    /// 注册消息类型：tick 末对该 inbox 统一 `clear`（R6.3）。
    ///
    /// inbox 资源在首次运行系统时经 `SystemParam::init_state` 注入
    /// （`init_resource::<MessageInbox<T>>()`）；若无系统使用该消息，钩子
    /// 恒 no-op。
    pub fn add_message<T: Message>(&mut self) -> &mut Self {
        self.clear_hooks.push(Box::new(|world: &mut World| {
            // inbox 未注入（无消费系统）时资源缺失，跳过清空
            if let Some(inbox) = world.resource_mut::<MessageInbox<T>>() {
                inbox.clear();
            }
        }));
        self
    }

    /// 注册借用系统（0-4 参数函数/闭包；经 [`IntoSystem`]`<M>` 转换）。
    ///
    /// `M` 由函数签名正推唯一确定：`IntoSystem<fn(P0, …)>` 的 `P` 经
    /// [`SystemParamFunction`] 的 `FnMut(P)` 正向约束推断，无投影逆推歧义
    /// （E0283）；`&mut World` 不构成 `SystemParam`，故排他函数不会误匹配
    /// 非排他 `M`（A9）。
    pub fn add_system<M>(&mut self, sys: impl IntoSystem<M> + 'static) -> &mut Self {
        self.systems.push(Box::new(sys.into_system()));
        self.order_dirty = true;
        self
    }

    /// [`add_system`] 的别名（旧 ECS 方案 `add_systems` 语义对应）。
    pub fn add_systems<M>(&mut self, sys: impl IntoSystem<M> + 'static) -> &mut Self {
        self.add_system(sys)
    }

    /// 注册排他系统（`fn(&mut World)`，IC-9 exclusive = true；`M` 固定为
    /// `ExclusiveMarker`，无推断歧义，A9）。
    pub fn add_exclusive_system(
        &mut self,
        sys: impl IntoSystem<ExclusiveMarker> + 'static,
    ) -> &mut Self {
        self.systems.push(Box::new(sys.into_system()));
        self.order_dirty = true;
        self
    }

    /// 建立依赖：`later` 系统在 `earlier` 系统之后执行。
    ///
    /// 按系统名（`System::name`，即函数路径）关联；引用了未注册的系统名时
    /// debug 构建断言失败（装配错误），release 下忽略该依赖。
    pub fn after(&mut self, later: impl SystemLabel, earlier: impl SystemLabel) -> &mut Self {
        self.deps.push((later.label(), earlier.label()));
        self.order_dirty = true;
        self
    }

    /// 打开/关闭并行分组（R7.3，默认关闭）。
    ///
    /// 当前实现开启后仅改变分组调度结构（同层系统在拓扑序内连续、可并行），
    /// 实际仍单线程串行执行；真正的多线程并行由 T16 落地。
    pub fn set_parallel(&mut self, on: bool) -> &mut Self {
        self.parallel = on;
        self
    }

    /// 固定步长推进（IC-9 `tick_clock`）：累计 `dt`，返回本轮应执行次数。
    ///
    /// 调用方按返回值调用 `run` 相应次数（T10 particlemc-framework-core 的 20Hz 驱动）。
    pub fn tick_clock(&mut self, dt: Duration) -> usize {
        self.clock.tick(dt)
    }

    /// 调整内嵌固定步长时钟频率（默认 20Hz）。
    pub fn set_fixed_hz(&mut self, hz: f64) -> &mut Self {
        self.clock = FixedClock::from_hz(hz);
        self
    }

    /// 已注册系统数。
    pub fn len(&self) -> usize {
        self.systems.len()
    }

    /// 是否无已注册系统。
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }

    /// 可见系统列表（执行序，含 `.after` 修正）：`(name, exclusive)`。
    ///
    /// 供测试断言（R7 Scenario：`systems()` 返回可见顺序）。
    pub fn systems(&self) -> impl Iterator<Item = (&'static str, bool)> {
        let order = if self.order_dirty {
            let names: Vec<&'static str> = self.systems.iter().map(|s| s.name()).collect();
            topo_groups(&names, &self.deps)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        } else {
            self.order.clone()
        };
        // 先收集为所有权元组 Vec，再转迭代器（返回类型不含 &self 隐式借用）
        let items: Vec<(&'static str, bool)> = order
            .into_iter()
            .map(|i| (self.systems[i].name(), self.systems[i].exclusive()))
            .collect();
        items.into_iter()
    }

    /// 并行分组可见结构（拓扑分层，每层为系统名列表）。
    ///
    /// 与 `set_parallel` 开关无关，始终按依赖关系计算（供测试断言分组正确性）。
    pub fn parallel_groups(&self) -> Vec<Vec<&'static str>> {
        let groups = if self.order_dirty {
            let names: Vec<&'static str> = self.systems.iter().map(|s| s.name()).collect();
            topo_groups(&names, &self.deps)
        } else {
            self.groups.clone()
        };
        groups
            .iter()
            .map(|group| group.iter().map(|&i| self.systems[i].name()).collect())
            .collect()
    }

    /// 执行一轮完整 tick（IC-9 `run`）。
    ///
    /// 流程：命令起始应用 → 按执行序运行系统 → tick 末清空全部消息 inbox。
    pub fn run(&mut self, world: &mut World) {
        if self.order_dirty {
            self.recompute_order();
        }
        // tick 起始：应用上一 tick 残留 / 外部（T9 submit）提交的命令（兜底）
        apply_deferred_commands(world);
        if self.parallel {
            // 并行模式：按拓扑分层执行（同层当前串行，T16 落地并行）
            for group in &self.groups {
                for &i in group {
                    self.systems[i].run(world);
                    // 每系统后即时 apply（自建 ApplyDeferred）：同 tick 内
                    // Commands 生产者对后续系统可见
                    apply_deferred_commands(world);
                }
            }
        } else {
            for &i in &self.order {
                self.systems[i].run(world);
                apply_deferred_commands(world);
            }
        }
        for hook in &self.clear_hooks {
            hook(world);
        }
    }

    /// 重算执行序与分组（Kahn 分层，环检测 debug_assert）。
    fn recompute_order(&mut self) {
        let names: Vec<&'static str> = self.systems.iter().map(|s| s.name()).collect();
        let groups = topo_groups(&names, &self.deps);
        self.order = groups.iter().flatten().copied().collect();
        self.groups = groups;
        self.order_dirty = false;
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule::new()
    }
}

/// 命令起始应用（R5.1）：取出缓冲资源 → apply → 插回。
///
/// remove/insert 将缓冲从 World 移动出来，规避「resource_mut 借用 World 又
/// 传 World」的借用冲突；每 tick 一次 move，HashMap 移动零分配。
fn apply_deferred_commands(world: &mut World) {
    // 无缓冲资源（本 tick 尚无系统使用 Commands）则跳过，避免注入空缓冲，
    // 保持「无 Commands 系统时 CommandBuffer 资源不出现」
    if let Some(mut buffer) = world.remove_resource::<CommandBuffer>() {
        Commands::new(&mut buffer).apply(world);
        world.insert_resource(buffer);
    }
}

/// Kahn 分层拓扑排序：返回层序（同层无依赖），层内按注册序（下标升序）。
///
/// `deps` 为名字级依赖（later, earlier）；解析到未注册名字时 debug 断言，
/// release 忽略。依赖环：debug 构建断言失败（R7.2 环检测）；release 按注册
/// 序补入剩余系统并终止（避免死循环）。
fn topo_groups(names: &[&str], deps: &[(String, String)]) -> Vec<Vec<usize>> {
    let n = names.len();
    let name_to_idx: HashMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, &name)| (name, i))
        .collect();
    // 邻接表：earlier → [later]（边方向 = 依赖方向）
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut in_deg = vec![0usize; n];
    for (later, earlier) in deps {
        let later_idx = name_to_idx.get(later.as_str()).copied();
        let earlier_idx = name_to_idx.get(earlier.as_str()).copied();
        match (later_idx, earlier_idx) {
            (Some(l), Some(e)) => {
                adj.entry(e).or_default().push(l);
                in_deg[l] += 1;
            }
            // 引用了未注册的系统名：装配错误，debug 暴露；release 忽略
            _ => debug_assert!(false, "after 引用了未注册的系统：{later} → {earlier}"),
        }
    }
    let mut remaining: Vec<usize> = (0..n).collect();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    while !remaining.is_empty() {
        // 本层就绪集：入度为零且尚未调度（保持注册序）
        let ready: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|&i| in_deg[i] == 0)
            .collect();
        if ready.is_empty() {
            // 依赖环：无法再推进。debug 断言；release 按注册序补入剩余系统
            debug_assert!(false, "依赖环检测：系统调度顺序无法确定");
            groups.push(remaining);
            break;
        }
        groups.push(ready.clone());
        for &i in &ready {
            for &j in adj.get(&i).into_iter().flatten() {
                in_deg[j] -= 1;
            }
        }
        remaining.retain(|i| !ready.contains(i));
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetype::{ArchetypeDef, ArchetypeId};
    use crate::component::{Component, ComponentId, ComponentStorage};
    use crate::entity::EntityTypeId;
    use crate::message::{MessageInbox, MessageReader, MessageWriter};
    use crate::system::{Res, ResMut};

    // ---- 测试资源 ----

    #[derive(Default, Debug, PartialEq)]
    struct Counter(u32);

    #[derive(Default)]
    struct OrderLog(Vec<String>);

    /// 是否允许本 tick 发送消息（控制 emit 仅在特定 tick 写入）。
    #[derive(Default)]
    struct ShouldEmit(bool);

    // ---- 测试组件 ----

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Position {
        x: f32,
    }

    impl Component for Position {
        fn id() -> ComponentId {
            ComponentId(30)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Velocity {
        dx: f32,
    }

    impl Component for Velocity {
        fn id() -> ComponentId {
            ComponentId(31)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    // ---- 测试 Archetype ----

    static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "SchPlayerArchetype",
        component_ids: &[ComponentId(30), ComponentId(31)],
        entity_kind: EntityTypeId(1),
        component_types: &[],
    };

    // ---- 测试消息 ----

    #[derive(Debug, PartialEq, Eq)]
    struct Tick(u32);

    impl Message for Tick {}

    // ---- 记录执行顺序的系统 ----

    fn sys_a(log: ResMut<OrderLog>) {
        log.0.0.push("A".to_string());
    }

    fn sys_b(log: ResMut<OrderLog>) {
        log.0.0.push("B".to_string());
    }

    fn sys_c(log: ResMut<OrderLog>) {
        log.0.0.push("C".to_string());
    }

    fn sys_d(log: ResMut<OrderLog>) {
        log.0.0.push("D".to_string());
    }

    fn exclusive_step(world: &mut World) {
        let log = world.resource_mut::<OrderLog>().unwrap();
        log.0.push("E".to_string());
    }

    fn world_with_log() -> World {
        let mut world = World::new();
        world.init_resource::<OrderLog>();
        world
    }

    #[test]
    fn add_system_runs_in_registration_order() {
        let mut world = world_with_log();
        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_a)
            .add_system(sys_b)
            .add_system(sys_c);
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<OrderLog>().unwrap().0,
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
    }

    #[test]
    fn after_dependency_reorders_execution() {
        // R7 Scenario: A.after(B)、B.after(C) → 执行顺序 C → B → A
        let mut world = world_with_log();
        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_a)
            .add_system(sys_b)
            .add_system(sys_c)
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_a",
                "particlemc_framework_ecs::schedule::tests::sys_b",
            )
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_b",
                "particlemc_framework_ecs::schedule::tests::sys_c",
            );
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<OrderLog>().unwrap().0,
            vec!["C".to_string(), "B".to_string(), "A".to_string()]
        );
    }

    #[test]
    fn systems_returns_visible_order() {
        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_a)
            .add_system(sys_b)
            .add_system(sys_c)
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_c",
                "particlemc_framework_ecs::schedule::tests::sys_a",
            );
        // after(C, A)：C 在 A 之后 → 可见顺序 A, B, C（C 依赖 A，移到其后）
        let visible: Vec<&str> = schedule.systems().map(|(name, _)| name).collect();
        assert_eq!(
            visible,
            vec![
                "particlemc_framework_ecs::schedule::tests::sys_a",
                "particlemc_framework_ecs::schedule::tests::sys_b",
                "particlemc_framework_ecs::schedule::tests::sys_c",
            ]
        );
        // 名字 + 排他标记成对返回
        let tagged: Vec<(&str, bool)> = schedule.systems().collect();
        assert!(tagged.iter().all(|&(_, excl)| !excl));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn dependency_cycle_panics_in_debug() {
        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_a)
            .add_system(sys_b)
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_a",
                "particlemc_framework_ecs::schedule::tests::sys_b",
            )
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_b",
                "particlemc_framework_ecs::schedule::tests::sys_a",
            );
        // run 触发重算：环检测 debug_assert 失败
        let mut world = World::new();
        schedule.run(&mut world);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn after_unknown_system_ignored_in_release() {
        // 引用未注册系统名：debug 断言（见 dependency_cycle 同类），release 忽略
        let mut world = world_with_log();
        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_a)
            .add_system(sys_b)
            .after("particlemc_framework_ecs::schedule::tests::sys_a", "ghost_system");
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<OrderLog>().unwrap().0,
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn message_written_and_cleared_per_tick() {
        fn emit(flag: Res<ShouldEmit>, mut writer: MessageWriter<Tick>) {
            if flag.0.0 {
                writer.write(Tick(1));
            }
        }

        fn read_count(reader: MessageReader<Tick>, counter: ResMut<Counter>) {
            counter.0.0 += reader.read().count() as u32;
        }

        let mut world = World::new();
        world.init_resource::<Counter>();
        world.insert_resource(ShouldEmit(true));
        let mut schedule = Schedule::new();
        schedule
            .add_message::<Tick>()
            .add_system(emit)
            .add_system(read_count);
        schedule.run(&mut world);
        // tick 1：emit 写入 1 条 → read 读到 1 → tick 末清空
        assert_eq!(world.resource::<Counter>().unwrap().0, 1);
        // tick 2：emit 关闭（无新消息）→ read 读到 0（上一 tick 已清空）
        world.insert_resource(ShouldEmit(false));
        schedule.run(&mut world);
        assert_eq!(world.resource::<Counter>().unwrap().0, 1);
        // inbox 确实清空（容量保留复用）
        let inbox = world.resource::<MessageInbox<Tick>>().unwrap();
        assert_eq!(inbox.read().count(), 0);
    }

    #[test]
    fn exclusive_system_interleaved_with_borrow_systems() {
        let mut world = world_with_log();
        let mut schedule = Schedule::new();
        schedule
            .add_system(sys_a)
            .add_exclusive_system(exclusive_step)
            .add_system(sys_b);
        // 排他系统在注册序位置执行，与借用系统互斥（顺序由注册序/.after 决定）
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<OrderLog>().unwrap().0,
            vec!["A".to_string(), "E".to_string(), "B".to_string()]
        );
        // systems() 正确标记排他系统
        let names: Vec<(&str, bool)> = schedule.systems().collect();
        assert!(names.contains(&("particlemc_framework_ecs::schedule::tests::exclusive_step", true)));
    }

    #[test]
    fn commands_applied_at_tick_start() {
        fn spawn_one(mut commands: Commands) {
            commands.spawn(ArchetypeId(0)).insert(Position { x: 3.0 });
        }

        fn count_entities(counter: ResMut<Counter>, q: crate::query::Query<(&Position,)>) {
            // 重置计数：每 tick 反映"当前可见实体数"，验证命令起始 apply 后同 tick 可见
            counter.0.0 = q.iter().count() as u32;
        }

        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.init_resource::<Counter>();
        let mut schedule = Schedule::new();
        schedule.add_system(spawn_one).add_system(count_entities);
        // 每 tick：命令起始 apply（spawn 生效）→ 计数系统可见
        schedule.run(&mut world);
        assert_eq!(world.resource::<Counter>().unwrap().0, 1);
        schedule.run(&mut world);
        assert_eq!(world.resource::<Counter>().unwrap().0, 2);
    }

    #[test]
    fn fixed_clock_tick_counts_at_20hz() {
        let mut clock = FixedClock::from_hz(20.0);
        // 1s → 20 步
        assert_eq!(clock.tick(Duration::from_secs(1)), 20);
        // 余数 0，再 tick 零
        assert_eq!(clock.tick(Duration::ZERO), 0);
        // 50ms → 恰好 1 步；再 50ms → 1 步（余数精确不漂移）
        let mut clock2 = FixedClock::from_hz(20.0);
        assert_eq!(clock2.tick(Duration::from_millis(50)), 1);
        assert_eq!(clock2.tick(Duration::from_millis(50)), 1);
    }

    #[test]
    fn fixed_clock_accumulates_fractional_steps() {
        let mut clock = FixedClock::from_hz(10.0); // 步长 100ms
        // 150ms：1 步 + 50ms 余数
        assert_eq!(clock.tick(Duration::from_millis(150)), 1);
        // 再 60ms：余数累计 110ms → 1 步 + 10ms 余数
        assert_eq!(clock.tick(Duration::from_millis(60)), 1);
        // 再 5ms：余数 15ms < 100ms → 0 步
        assert_eq!(clock.tick(Duration::from_millis(5)), 0);
    }

    #[test]
    fn fixed_clock_step_and_reset() {
        let mut clock = FixedClock::from_hz(20.0);
        clock.reset();
        assert!(!clock.step()); // 无余量
        clock.advance(Duration::from_millis(50)); // 仅充能、不消费
        assert!(clock.step()); // 消耗 50ms → 恰好 1 步
        assert!(!clock.step()); // 余量已耗尽
        clock.reset();
        assert!(!clock.step());
    }

    #[test]
    fn schedule_tick_clock_advances_internal_clock() {
        let mut schedule = Schedule::new(); // 默认 20Hz
        assert_eq!(schedule.tick_clock(Duration::from_secs(1)), 20);
        assert_eq!(schedule.tick_clock(Duration::ZERO), 0);
        // 调频后按新步长
        schedule.set_fixed_hz(100.0);
        assert_eq!(schedule.tick_clock(Duration::from_millis(10)), 1);
    }

    #[test]
    fn parallel_groups_layered_by_dependencies() {
        let mut schedule = Schedule::new();
        // 注册 D, C, B, A；依赖：B.after(D)（B 依赖 D）、C.after(B)
        schedule
            .add_system(sys_d)
            .add_system(sys_c)
            .add_system(sys_b)
            .add_system(sys_a)
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_b",
                "particlemc_framework_ecs::schedule::tests::sys_d",
            )
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_c",
                "particlemc_framework_ecs::schedule::tests::sys_b",
            );
        // 拓扑分层：
        //   层0：入度 0 的系统（A, D）
        //   层1：B（依赖 D）
        //   层2：C（依赖 B）
        let groups: Vec<Vec<&str>> = schedule
            .parallel_groups()
            .into_iter()
            .map(|g| g.into_iter().collect())
            .collect();
        assert_eq!(groups.len(), 3);
        let mut flat: Vec<&str> = groups.into_iter().flatten().collect();
        flat.sort_unstable();
        // parallel_groups 返回系统全名（System::name = type_name），非缩写
        let expected_names: Vec<&str> = {
            let mut v = vec![
                "particlemc_framework_ecs::schedule::tests::sys_a",
                "particlemc_framework_ecs::schedule::tests::sys_b",
                "particlemc_framework_ecs::schedule::tests::sys_c",
                "particlemc_framework_ecs::schedule::tests::sys_d",
            ];
            v.sort_unstable();
            v
        };
        assert_eq!(flat, expected_names);
        // 同层无依赖：layer0 为入度 0 的 A 与 D（交换执行安全，T16 并行依据）
        let first_layer: Vec<&str> = schedule.parallel_groups()[0].clone();
        assert!(
            first_layer.contains(&"particlemc_framework_ecs::schedule::tests::sys_a")
                && first_layer.contains(&"particlemc_framework_ecs::schedule::tests::sys_d")
        );
    }

    #[test]
    fn parallel_mode_runs_same_result_serially() {
        let mut world = world_with_log();
        let mut schedule = Schedule::new();
        schedule
            .set_parallel(true)
            .add_system(sys_a)
            .add_system(sys_b)
            .after(
                "particlemc_framework_ecs::schedule::tests::sys_b",
                "particlemc_framework_ecs::schedule::tests::sys_a",
            );
        // 开启并行分组：当前单线程串行执行，结果与串行拓扑序一致
        schedule.run(&mut world);
        assert_eq!(
            world.resource::<OrderLog>().unwrap().0,
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn query_system_updates_world_through_schedule() {
        fn move_entities(mut q: crate::query::Query<(&mut Position, &Velocity)>) {
            for (pos, vel) in q.iter_mut() {
                pos.x += vel.dx;
            }
        }

        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e = world.spawn(ArchetypeId(0));
        let _ = world.insert(e, Position { x: 1.0 });
        let _ = world.insert(e, Velocity { dx: 2.0 });
        let mut schedule = Schedule::new();
        schedule.add_system(move_entities);
        schedule.run(&mut world);
        assert_eq!(world.get::<Position>(e).map(|p| p.x), Some(3.0));
    }

    #[test]
    fn empty_schedule_runs_without_effect() {
        let mut world = World::new();
        let mut schedule = Schedule::new();
        schedule.run(&mut world);
        assert!(schedule.is_empty());
        assert_eq!(schedule.len(), 0);
        assert_eq!(schedule.systems().count(), 0);
    }

    #[test]
    fn world_entities_unaffected_by_schedule_without_commands() {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        let e = world.spawn(ArchetypeId(0));
        let mut schedule = Schedule::new();
        schedule.add_system(sys_a);
        // 无 Commands 系统时 CommandBuffer 资源未注入：apply 为 no-op
        schedule.run(&mut world);
        assert!(world.contains(e));
    }
}
