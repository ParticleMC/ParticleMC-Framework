//! 实体 AI 子系统：goal / target 选择器与内置目标行为（T6）。
//!
//! 语义对齐 Minestom Java 的 `entity/ai` 包，但面向 自研 ECS（particlemc-framework-ecs） 重写：
//!
//! - [`Goal`] 为纯状态机（`priority` / `can_start` / `start` / `tick` /
//!   `stop`），不直接触碰 ECS 世界；系统在每 tick 通过
//!   [`Goal::update_context`] 注入目标实体与坐标上下文，运行中的 goal 经
//!   [`Goal::navigation_target`] 暴露期望导航点，由
//!   [`crate::system::entity_ai`] 写入 `EntityCreature.navigation_target`。
//! - [`TargetSelector`] 按优先级查找目标实体；[`ClosestEntityTarget`] 经
//!   `&World` 枚举（遍历 archetype 实体表，无需额外 `QueryState`）。
//! - [`EntityAIGroup`] 聚合 goal 与 target 选择器，作为
//!   [`crate::component::Living`] 的 `ai` 字段挂载（取代 T4 空壳）。
//!
//! 契约偏差（T6 报告注明的简化）：
//! - `GoalSelector` 每次只运行一个 goal（最高优先级且可启动者），与 Java
//!   单运行选择器语义一致。
//! - `ClosestEntityTarget` 的过滤器为 `Box<dyn Fn(&Entity) -> bool + Send + Sync>`
//!   （`Target: Send + Sync` 约束下闭包需为线程安全）。
//!
//! 变更标识符：`complete-missing-subsystems`。

use crate::prelude::{Component, Entity};
use particlemc_framework_ecs::world::World;

use crate::component::Position;

/// 系统每 tick 注入 goal 的上下文（目标实体与双方坐标）。
///
/// 移动决策（追击 / 撤退 / 保持）依赖自身坐标与目标坐标，goal 在
/// [`Goal::update_context`] 中缓存这些值，供 `can_start` / `tick` 使用。
#[derive(Debug, Clone, Copy)]
pub struct GoalContext {
    /// 本实体当前坐标。
    pub self_position: [f64; 3],
    /// 当前目标实体（可能为 `None`）。
    pub target: Option<Entity>,
    /// 目标实体当前坐标（目标缺失或未挂 `Position` 时为 `None`）。
    pub target_position: Option<[f64; 3]>,
}

/// AI 目标（goal）：可被 [`GoalSelector`] 调度运行的状态机。
pub trait Goal: Send + Sync {
    /// 优先级（数值越大越优先）。
    fn priority(&self) -> i32;
    /// 是否满足启动条件。
    fn can_start(&self) -> bool;
    /// 开始运行（仅当通过 [`GoalSelector`] 仲裁后调用）。
    fn start(&mut self);
    /// 运行中的每 tick 更新。
    fn tick(&mut self);
    /// 停止运行。
    fn stop(&mut self);

    /// 接收系统注入的上下文（默认忽略；移动类 goal 覆写以缓存坐标）。
    fn update_context(&mut self, _ctx: &GoalContext) {}

    /// 运行中的移动类 goal 期望写回实体的导航目标点。
    ///
    /// 由 [`crate::system::entity_ai`] 读取并写入
    /// `EntityCreature.navigation_target`，供速度计算使用。
    fn navigation_target(&self) -> Option<[f64; 3]> {
        None
    }

    /// 运行中的移动类 goal 期望的移动速度（方块 / tick）。
    fn movement_speed(&self) -> f64 {
        0.1
    }
}

/// 目标选择器：按优先级注册多个 [`Target`]，取优先级最高者命中结果。
#[derive(Default)]
pub struct TargetSelector {
    targets: Vec<(i32, Box<dyn Target>)>,
}

impl TargetSelector {
    /// 注册一个目标查找器（按 `priority` 降序命中）。
    pub fn add_target<T: Target + 'static>(&mut self, priority: i32, target: T) {
        self.targets.push((priority, Box::new(target)));
    }

    /// 在所有目标查找器中取优先级最高且命中的实体。
    ///
    /// 同优先级保留先注册者（首个命中即胜出）。
    pub fn find_target(&self, world: &World, self_entity: Entity) -> Option<Entity> {
        let mut best: Option<(i32, Entity)> = None;
        for (priority, target) in self.targets.iter() {
            let Some(entity) = target.find(world, self_entity) else {
                continue;
            };
            if best.is_none_or(|(bp, _)| *priority > bp) {
                best = Some((*priority, entity));
            }
        }
        best.map(|(_, entity)| entity)
    }
}

/// 目标查找器：在给定世界中为 `self_entity` 寻找目标实体。
pub trait Target: Send + Sync {
    /// 查找目标实体；未命中返回 `None`。
    fn find(&self, world: &World, self_entity: Entity) -> Option<Entity>;
}

/// 单运行 goal 调度器：每次只运行一个 goal（优先级最高且可启动者）。
#[derive(Default)]
pub struct GoalSelector {
    goals: Vec<GoalEntry>,
    running: Option<usize>,
}

/// goal 注册条目。
struct GoalEntry {
    priority: i32,
    goal: Box<dyn Goal>,
    running: bool,
}

impl GoalSelector {
    /// 注册一个 goal（不立即运行，等待 `tick` 仲裁）。
    pub fn add_goal<G: Goal + 'static>(&mut self, priority: i32, goal: G) {
        self.goals.push(GoalEntry {
            priority,
            goal: Box::new(goal),
            running: false,
        });
    }

    /// 单次 AI tick：停止不可用的运行中 goal，否则启动最高优先级可用 goal，
    /// 最后推进运行中 goal 的 `tick`。
    pub fn tick(&mut self) {
        // 1. 运行中 goal 不再可用 → 停止。
        if let Some(idx) = self.running {
            let stop = self
                .goals
                .get(idx)
                .is_some_and(|entry| !entry.goal.can_start());
            if stop {
                if let Some(entry) = self.goals.get_mut(idx) {
                    entry.goal.stop();
                    entry.running = false;
                }
                self.running = None;
            }
        }
        // 2. 无运行 goal → 启动优先级最高的可用 goal。
        if self.running.is_none() {
            let mut best: Option<usize> = None;
            for (i, entry) in self.goals.iter().enumerate() {
                if entry.running || !entry.goal.can_start() {
                    continue;
                }
                let better = best.is_none_or(|b| {
                    self.goals
                        .get(b)
                        .is_some_and(|e| entry.priority > e.priority)
                });
                if better {
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                if let Some(entry) = self.goals.get_mut(i) {
                    entry.goal.start();
                    entry.running = true;
                }
                self.running = Some(i);
            }
        }
        // 3. 推进运行中 goal。
        if let Some(idx) = self.running
            && let Some(entry) = self.goals.get_mut(idx)
        {
            entry.goal.tick();
        }
    }

    /// 向全部 goal 注入当前上下文。
    pub fn update_context(&mut self, ctx: &GoalContext) {
        for entry in self.goals.iter_mut() {
            entry.goal.update_context(ctx);
        }
    }

    /// 运行中 goal 期望的导航目标点。
    pub fn navigation_target(&self) -> Option<[f64; 3]> {
        let idx = self.running?;
        self.goals.get(idx)?.goal.navigation_target()
    }

    /// 运行中 goal 期望的移动速度。
    pub fn movement_speed(&self) -> f64 {
        match self.running {
            Some(idx) => self.goals.get(idx).map_or(0.0, |e| e.goal.movement_speed()),
            None => 0.0,
        }
    }
}

/// AI 组：聚合目标与 goal 选择器，作为 [`crate::component::Living::ai`] 挂载。
#[derive(Component, Default)]
#[component(storage = "sparse")]
pub struct EntityAIGroup {
    /// 目标选择器。
    pub goals: GoalSelector,
    /// goal 选择器。
    pub targets: TargetSelector,
}

impl EntityAIGroup {
    /// 构造空 AI 组。
    pub fn new() -> Self {
        Self::default()
    }
}

/// 追踪目标实体并持续移动（移动类 goal 示例）。
pub struct FollowTargetGoal {
    /// 当前目标实体。
    pub target: Option<Entity>,
    /// 移动速度（方块 / tick）。
    pub speed: f64,
    cached: Option<GoalContext>,
    navigation_target: Option<[f64; 3]>,
}

impl FollowTargetGoal {
    /// 以固定目标实体构造。
    pub fn new(target: Entity, speed: f64) -> Self {
        Self {
            target: Some(target),
            speed,
            cached: None,
            navigation_target: None,
        }
    }

    /// 构造无目标实体（目标由系统每 tick 经上下文注入）。
    pub fn new_unset(speed: f64) -> Self {
        Self {
            target: None,
            speed,
            cached: None,
            navigation_target: None,
        }
    }
}

impl Goal for FollowTargetGoal {
    fn priority(&self) -> i32 {
        10
    }

    fn can_start(&self) -> bool {
        self.target.is_some()
    }

    fn start(&mut self) {
        self.navigation_target = self.cached.and_then(|c| c.target_position);
    }

    fn tick(&mut self) {
        if self.target.is_some() {
            self.navigation_target = self.cached.and_then(|c| c.target_position);
        }
    }

    fn stop(&mut self) {
        self.navigation_target = None;
    }

    fn update_context(&mut self, ctx: &GoalContext) {
        self.target = ctx.target;
        self.cached = Some(*ctx);
    }

    fn navigation_target(&self) -> Option<[f64; 3]> {
        self.navigation_target
    }

    fn movement_speed(&self) -> f64 {
        self.speed
    }
}

/// 近战攻击：目标进入攻击范围后停止移动并按冷却计数触发攻击。
pub struct MeleeAttackGoal {
    /// 当前目标实体。
    pub target: Option<Entity>,
    /// 攻击范围（方块）。
    pub attack_range: f64,
    /// 攻击间隔（tick）。
    pub attack_delay_ticks: u32,
    /// 累计触发攻击次数（统计 / 测试用）。
    pub attacks_fired: u32,
    cached: Option<GoalContext>,
    ticks_since_attack: u32,
    navigation_target: Option<[f64; 3]>,
}

impl MeleeAttackGoal {
    /// 以固定目标实体与攻击参数构造。
    pub fn new(target: Entity, attack_range: f64, attack_delay_ticks: u32) -> Self {
        Self {
            target: Some(target),
            attack_range,
            attack_delay_ticks: attack_delay_ticks.max(1),
            attacks_fired: 0,
            cached: None,
            ticks_since_attack: 0,
            navigation_target: None,
        }
    }
}

impl Goal for MeleeAttackGoal {
    fn priority(&self) -> i32 {
        10
    }

    fn can_start(&self) -> bool {
        self.target.is_some()
    }

    fn start(&mut self) {
        self.ticks_since_attack = 0;
    }

    fn tick(&mut self) {
        let Some(ctx) = self.cached else {
            return;
        };
        let Some(tp) = ctx.target_position else {
            self.navigation_target = None;
            return;
        };
        if distance_sq(ctx.self_position, tp) <= self.attack_range * self.attack_range {
            // 已进入攻击距离：停下并累计攻击冷却。
            self.navigation_target = None;
            self.ticks_since_attack = self.ticks_since_attack.saturating_add(1);
            if self.ticks_since_attack >= self.attack_delay_ticks {
                self.attacks_fired = self.attacks_fired.saturating_add(1);
                self.ticks_since_attack = 0;
            }
        } else {
            // 超出攻击距离：追击目标。
            self.navigation_target = Some(tp);
            self.ticks_since_attack = 0;
        }
    }

    fn stop(&mut self) {
        self.navigation_target = None;
    }

    fn update_context(&mut self, ctx: &GoalContext) {
        self.target = ctx.target;
        self.cached = Some(*ctx);
    }

    fn navigation_target(&self) -> Option<[f64; 3]> {
        self.navigation_target
    }
}

/// 远程攻击：保持最佳射程（过近后撤、过远接近、射程内保持站位）。
pub struct RangedAttackGoal {
    /// 当前目标实体。
    pub target: Option<Entity>,
    /// 最小射程（过近后撤）。
    pub min_range: f64,
    /// 最大射程（过远接近）。
    pub max_range: f64,
    cached: Option<GoalContext>,
    navigation_target: Option<[f64; 3]>,
}

impl RangedAttackGoal {
    /// 以固定目标实体与射程参数构造。
    pub fn new(target: Entity, min_range: f64, max_range: f64) -> Self {
        Self {
            target: Some(target),
            min_range,
            max_range,
            cached: None,
            navigation_target: None,
        }
    }
}

impl Goal for RangedAttackGoal {
    fn priority(&self) -> i32 {
        10
    }

    fn can_start(&self) -> bool {
        self.target.is_some()
    }

    fn start(&mut self) {}

    fn tick(&mut self) {
        let Some(ctx) = self.cached else {
            return;
        };
        let Some(tp) = ctx.target_position else {
            self.navigation_target = None;
            return;
        };
        let d2 = distance_sq(ctx.self_position, tp);
        let min2 = self.min_range * self.min_range;
        let max2 = self.max_range * self.max_range;
        if d2 < min2 {
            // 过近：沿目标反方向后撤一个方块。
            let [sx, sy, sz] = ctx.self_position;
            let [tx, ty, tz] = tp;
            let (dx, dy, dz) = (sx - tx, sy - ty, sz - tz);
            let len = (dx * dx + dy * dy + dz * dz).sqrt();
            if len > 1e-9 {
                self.navigation_target = Some([sx + dx / len, sy + dy / len, sz + dz / len]);
            } else {
                self.navigation_target = None;
            }
        } else if d2 <= max2 {
            // 最佳射程：保持站位。
            self.navigation_target = None;
        } else {
            // 过远：接近目标。
            self.navigation_target = Some(tp);
        }
    }

    fn stop(&mut self) {
        self.navigation_target = None;
    }

    fn update_context(&mut self, ctx: &GoalContext) {
        self.target = ctx.target;
        self.cached = Some(*ctx);
    }

    fn navigation_target(&self) -> Option<[f64; 3]> {
        self.navigation_target
    }
}

/// 随机闲逛：按冷却间隔在半径内随机选取导航点。
pub struct RandomStrollGoal {
    /// 闲逛半径（方块）。
    pub radius: i32,
    /// 移动速度（方块 / tick）。
    pub speed: f64,
    cached: Option<GoalContext>,
    navigation_target: Option<[f64; 3]>,
    cooldown_ticks: u32,
    rng_state: u64,
}

/// 一次闲逛的持续 tick 数。
const STROLL_TICKS: u32 = 20;

impl RandomStrollGoal {
    /// 以半径与速度构造。
    pub fn new(radius: i32, speed: f64) -> Self {
        Self {
            radius: radius.max(1),
            speed,
            cached: None,
            navigation_target: None,
            cooldown_ticks: 0,
            rng_state: 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// 在 `[-radius, radius]` 内取随机整数偏移（简单 LCG，无外部依赖）。
    fn random_offset(&mut self) -> i32 {
        self.rng_state = self
            .rng_state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let size = u64::try_from(i64::from(self.radius) * 2 + 1).unwrap_or(0);
        if size == 0 {
            return 0;
        }
        let v = (self.rng_state >> 33) % size;
        i32::try_from(v).unwrap_or(0).wrapping_sub(self.radius)
    }
}

impl Goal for RandomStrollGoal {
    fn priority(&self) -> i32 {
        5
    }

    fn can_start(&self) -> bool {
        self.cooldown_ticks == 0 && self.cached.is_some()
    }

    fn start(&mut self) {
        self.cooldown_ticks = STROLL_TICKS;
        if let Some(ctx) = self.cached {
            let [sx, sy, sz] = ctx.self_position;
            let dx = self.random_offset();
            let dz = self.random_offset();
            self.navigation_target = Some([sx + f64::from(dx), sy, sz + f64::from(dz)]);
        }
    }

    fn tick(&mut self) {
        self.cooldown_ticks = self.cooldown_ticks.saturating_sub(1);
        if self.cooldown_ticks == 0 {
            self.navigation_target = None;
        }
    }

    fn stop(&mut self) {
        self.navigation_target = None;
    }

    fn update_context(&mut self, ctx: &GoalContext) {
        self.cached = Some(*ctx);
    }

    fn navigation_target(&self) -> Option<[f64; 3]> {
        self.navigation_target
    }

    fn movement_speed(&self) -> f64 {
        self.speed
    }
}

/// 随机四处张望（占位语义：不产生导航目标）。
pub struct RandomLookAroundGoal {}

impl Goal for RandomLookAroundGoal {
    fn priority(&self) -> i32 {
        1
    }

    fn can_start(&self) -> bool {
        false
    }

    fn start(&mut self) {}

    fn tick(&mut self) {}

    fn stop(&mut self) {}
}

/// 什么都不做（占位语义：永不启动）。
pub struct DoNothingGoal {}

impl Goal for DoNothingGoal {
    fn priority(&self) -> i32 {
        0
    }

    fn can_start(&self) -> bool {
        false
    }

    fn start(&mut self) {}

    fn tick(&mut self) {}

    fn stop(&mut self) {}
}

/// 组合攻击：根据距离在近战与远程之间切换。
pub struct CombinedAttackGoal {
    /// 近战部分。
    pub melee: MeleeAttackGoal,
    /// 远程部分。
    pub ranged: RangedAttackGoal,
    cached: Option<GoalContext>,
    navigation_target: Option<[f64; 3]>,
}

impl CombinedAttackGoal {
    /// 以近战 / 远程子目标构造。
    pub fn new(melee: MeleeAttackGoal, ranged: RangedAttackGoal) -> Self {
        Self {
            melee,
            ranged,
            cached: None,
            navigation_target: None,
        }
    }
}

impl Goal for CombinedAttackGoal {
    fn priority(&self) -> i32 {
        self.melee.priority().max(self.ranged.priority())
    }

    fn can_start(&self) -> bool {
        self.melee.can_start() || self.ranged.can_start()
    }

    fn start(&mut self) {
        self.melee.start();
        self.ranged.start();
    }

    fn tick(&mut self) {
        let in_melee = self
            .cached
            .zip(self.cached.and_then(|c| c.target_position))
            .is_some_and(|(ctx, tp)| {
                distance_sq(ctx.self_position, tp)
                    <= self.melee.attack_range * self.melee.attack_range
            });
        if in_melee {
            self.melee.tick();
        } else {
            self.ranged.tick();
        }
        self.navigation_target = if in_melee {
            self.melee.navigation_target()
        } else {
            self.ranged.navigation_target()
        };
    }

    fn stop(&mut self) {
        self.melee.stop();
        self.ranged.stop();
        self.navigation_target = None;
    }

    fn update_context(&mut self, ctx: &GoalContext) {
        self.cached = Some(*ctx);
        self.melee.update_context(ctx);
        self.ranged.update_context(ctx);
    }

    fn navigation_target(&self) -> Option<[f64; 3]> {
        self.navigation_target
    }

    fn movement_speed(&self) -> f64 {
        self.melee.movement_speed()
    }
}

/// 目标过滤器类型（`Target: Send + Sync` 约束下闭包需为线程安全）。
pub type EntityFilter = Box<dyn Fn(&Entity) -> bool + Send + Sync>;

/// 目标最近的可见实体（基于距离与可选过滤器）。
pub struct ClosestEntityTarget {
    /// 最大索敌范围（方块）。
    pub range: f64,
    /// 可选过滤器（额外判定目标是否可被选中）。
    pub filter: Option<EntityFilter>,
}

impl ClosestEntityTarget {
    /// 以索敌范围与可选过滤器构造。
    pub fn new(range: f64, filter: Option<EntityFilter>) -> Self {
        Self { range, filter }
    }
}

impl Target for ClosestEntityTarget {
    fn find(&self, world: &World, self_entity: Entity) -> Option<Entity> {
        let self_pos = world.get::<Position>(self_entity)?;
        let self_pos = [self_pos.x, self_pos.y, self_pos.z];
        let range_sq = self.range * self.range;
        let mut best: Option<(Entity, f64)> = None;
        for entity in world_entities(world) {
            if entity == self_entity {
                continue;
            }
            let Some(pos) = world.get::<Position>(entity) else {
                continue;
            };
            let d2 = distance_sq(self_pos, [pos.x, pos.y, pos.z]);
            if d2 > range_sq {
                continue;
            }
            if let Some(filter) = &self.filter
                && !filter(&entity)
            {
                continue;
            }
            if best.is_none_or(|(_, bd)| d2 < bd) {
                best = Some((entity, d2));
            }
        }
        best.map(|(entity, _)| entity)
    }
}

/// 目标的最近伤害来源实体（简化：直接返回记录的最后伤害者）。
pub struct LastEntityDamagerTarget {
    /// 最后伤害本实体的实体。
    pub last_damager: Option<Entity>,
}

impl LastEntityDamagerTarget {
    /// 以最近伤害者构造。
    pub fn new(last_damager: Option<Entity>) -> Self {
        Self { last_damager }
    }
}

impl Target for LastEntityDamagerTarget {
    fn find(&self, _world: &World, _self_entity: Entity) -> Option<Entity> {
        self.last_damager
    }
}

/// 枚举世界内全部实体（T11 迁移：`World::entities` 替代 archetype 实体表遍历）。
fn world_entities(world: &World) -> impl Iterator<Item = Entity> {
    world.entities().into_iter()
}

/// 两点距离平方（避免开方，供范围比较使用）。
fn distance_sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    let (dx, dy, dz) = (ax - bx, ay - by, az - bz);
    dx * dx + dy * dy + dz * dz
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 可控测试 goal 的共享状态（selector 移动 goal 后仍可读取 / 修改）。
    #[derive(Default)]
    struct MockState {
        can_start: bool,
        started: u32,
        ticked: u32,
        stopped: u32,
        nav: Option<[f64; 3]>,
    }

    /// 可控测试 goal：经 `Arc<Mutex<_>>` 共享状态记录生命周期回调。
    struct MockGoal {
        priority: i32,
        state: std::sync::Arc<std::sync::Mutex<MockState>>,
    }

    impl MockGoal {
        fn new(priority: i32, can_start: bool) -> Self {
            Self {
                priority,
                state: std::sync::Arc::new(std::sync::Mutex::new(MockState {
                    can_start,
                    ..Default::default()
                })),
            }
        }
    }

    impl Goal for MockGoal {
        fn priority(&self) -> i32 {
            self.priority
        }
        fn can_start(&self) -> bool {
            self.state.lock().unwrap().can_start
        }
        fn start(&mut self) {
            self.state.lock().unwrap().started += 1;
        }
        fn tick(&mut self) {
            self.state.lock().unwrap().ticked += 1;
        }
        fn stop(&mut self) {
            self.state.lock().unwrap().stopped += 1;
        }
        fn navigation_target(&self) -> Option<[f64; 3]> {
            self.state.lock().unwrap().nav
        }
    }

    #[test]
    fn goal_selector_starts_highest_priority_goal() {
        let mut selector = GoalSelector::default();
        let low = MockGoal::new(10, true);
        let high = MockGoal::new(100, true);
        let low_state = low.state.clone();
        let high_state = high.state.clone();
        selector.add_goal(10, low);
        selector.add_goal(100, high);
        selector.tick();
        // 高优先级 goal 启动并运行；低优先级被让位。
        assert_eq!(high_state.lock().unwrap().started, 1);
        assert_eq!(high_state.lock().unwrap().ticked, 1);
        assert_eq!(low_state.lock().unwrap().started, 0);
        selector.tick();
        assert_eq!(high_state.lock().unwrap().ticked, 2);
        assert_eq!(low_state.lock().unwrap().ticked, 0);
    }

    #[test]
    fn goal_selector_stops_unavailable_goal_and_starts_next() {
        let mut selector = GoalSelector::default();
        let high = MockGoal::new(100, true);
        let low = MockGoal::new(10, true);
        let high_state = high.state.clone();
        let low_state = low.state.clone();
        selector.add_goal(100, high);
        selector.add_goal(10, low);
        selector.tick();
        assert_eq!(high_state.lock().unwrap().started, 1);
        // 高优先级不可用 → 停止并让位给低优先级。
        high_state.lock().unwrap().can_start = false;
        selector.tick();
        assert_eq!(high_state.lock().unwrap().stopped, 1);
        assert_eq!(low_state.lock().unwrap().started, 1);
        assert_eq!(low_state.lock().unwrap().ticked, 1);
    }

    #[test]
    fn goal_selector_exposes_running_goal_navigation() {
        let mut selector = GoalSelector::default();
        let goal = MockGoal::new(10, true);
        let goal_state = goal.state.clone();
        goal_state.lock().unwrap().nav = Some([1.0, 2.0, 3.0]);
        selector.add_goal(10, goal);
        selector.tick();
        assert_eq!(selector.navigation_target(), Some([1.0, 2.0, 3.0]));
        assert!(selector.movement_speed() > 0.0);
    }

    #[test]
    fn goal_selector_without_goals_ticks_noop() {
        let mut selector = GoalSelector::default();
        selector.tick();
        assert_eq!(selector.navigation_target(), None);
    }

    #[test]
    fn closest_entity_target_picks_nearest_in_range() {
        let mut world = World::new();
        let self_entity = world.spawn_bundle(Position::new(0.0, 0.0, 0.0)).id();
        let far = world.spawn_bundle(Position::new(20.0, 0.0, 0.0)).id();
        let near = world.spawn_bundle(Position::new(3.0, 0.0, 0.0)).id();
        let target = ClosestEntityTarget::new(10.0, None);
        assert_eq!(target.find(&world, self_entity), Some(near));
        // 范围外实体不被选中。
        let out = ClosestEntityTarget::new(2.0, None);
        assert_eq!(out.find(&world, self_entity), None);
        // far 实体仍存在（不误删）。
        assert!(world.get::<Position>(far).is_some());
    }

    #[test]
    fn closest_entity_target_respects_filter() {
        let mut world = World::new();
        let self_entity = world.spawn_bundle(Position::new(0.0, 0.0, 0.0)).id();
        let excluded = world.spawn_bundle(Position::new(1.0, 0.0, 0.0)).id();
        let included = world.spawn_bundle(Position::new(2.0, 0.0, 0.0)).id();
        let filter: Box<dyn Fn(&Entity) -> bool + Send + Sync> = Box::new(move |e| *e != excluded);
        let target = ClosestEntityTarget::new(10.0, Some(filter));
        // 被过滤器排除的实体不参与竞选，最近的符合条件者为 included。
        assert_eq!(target.find(&world, self_entity), Some(included));
    }

    #[test]
    fn target_selector_uses_highest_priority_hit() {
        let mut world = World::new();
        let self_entity = world.spawn_bundle(Position::new(0.0, 0.0, 0.0)).id();
        let other = world.spawn_bundle(Position::new(1.0, 0.0, 0.0)).id();
        let mut selector = TargetSelector::default();
        selector.add_target(5, ClosestEntityTarget::new(100.0, None));
        selector.add_target(50, LastEntityDamagerTarget::new(Some(other)));
        // 高优先级（50）命中，尽管低优先级（5）也会命中。
        assert_eq!(selector.find_target(&world, self_entity), Some(other));
    }

    #[test]
    fn follow_target_goal_updates_navigation_from_context() {
        let mut world = World::new();
        let target = world.spawn_bundle(Position::new(10.0, 0.0, 0.0)).id();
        let mut goal = FollowTargetGoal::new(target, 2.0);
        let ctx = GoalContext {
            self_position: [0.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([10.0, 0.0, 0.0]),
        };
        goal.update_context(&ctx);
        assert!(goal.can_start());
        goal.start();
        assert_eq!(goal.navigation_target(), Some([10.0, 0.0, 0.0]));
        assert_eq!(goal.movement_speed(), 2.0);
        goal.stop();
        assert_eq!(goal.navigation_target(), None);
    }

    #[test]
    fn melee_goal_attacks_in_range_and_chases_outside() {
        let mut world = World::new();
        let target = world.spawn_bundle(Position::new(0.0, 0.0, 0.0)).id();
        let mut goal = MeleeAttackGoal::new(target, 2.0, 3);
        // 目标在攻击范围内：多次 tick 应累计攻击（3 tick 一次）。
        let ctx = GoalContext {
            self_position: [1.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([0.0, 0.0, 0.0]),
        };
        for _ in 0..3 {
            goal.update_context(&ctx);
            goal.tick();
        }
        assert_eq!(goal.attacks_fired, 1);
        assert_eq!(goal.navigation_target(), None);
        // 目标远离：进入追击，导航目标指向目标。
        let far_ctx = GoalContext {
            self_position: [0.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([20.0, 0.0, 0.0]),
        };
        goal.update_context(&far_ctx);
        goal.tick();
        assert_eq!(goal.navigation_target(), Some([20.0, 0.0, 0.0]));
        assert_eq!(goal.attacks_fired, 1);
    }

    #[test]
    fn ranged_goal_keeps_best_range() {
        let mut world = World::new();
        let target = world.spawn_bundle(Position::new(0.0, 0.0, 0.0)).id();
        let mut goal = RangedAttackGoal::new(target, 4.0, 10.0);
        // 最佳射程内：保持站位。
        goal.update_context(&GoalContext {
            self_position: [5.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([0.0, 0.0, 0.0]),
        });
        goal.tick();
        assert_eq!(goal.navigation_target(), None);
        // 过近：后撤（导航点远离目标）。
        goal.update_context(&GoalContext {
            self_position: [2.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([0.0, 0.0, 0.0]),
        });
        goal.tick();
        assert!(goal.navigation_target().is_some_and(|p| p[0] > 2.0));
        // 过远：接近目标。
        goal.update_context(&GoalContext {
            self_position: [20.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([0.0, 0.0, 0.0]),
        });
        goal.tick();
        assert_eq!(goal.navigation_target(), Some([0.0, 0.0, 0.0]));
    }

    #[test]
    fn stroll_goal_picks_point_within_radius() {
        let mut goal = RandomStrollGoal::new(5, 1.0);
        goal.update_context(&GoalContext {
            self_position: [10.0, 64.0, 10.0],
            target: None,
            target_position: None,
        });
        assert!(goal.can_start());
        goal.start();
        let nav = goal.navigation_target().expect("闲逛目标应存在");
        let [nx, _, nz] = nav;
        let dx = nx - 10.0;
        let dz = nz - 10.0;
        assert!(dx.abs() <= 5.0 && dz.abs() <= 5.0);
        // 持续 tick 后闲逛结束。
        for _ in 0..STROLL_TICKS {
            goal.tick();
        }
        assert_eq!(goal.navigation_target(), None);
    }

    #[test]
    fn combined_attack_switches_between_melee_and_ranged() {
        let mut world = World::new();
        let target = world.spawn_bundle(Position::new(0.0, 0.0, 0.0)).id();
        let melee = MeleeAttackGoal::new(target, 2.0, 2);
        let ranged = RangedAttackGoal::new(target, 5.0, 20.0);
        let mut combined = CombinedAttackGoal::new(melee, ranged);
        // 近战距离内：导航目标为空（停下攻击），攻击计数随 tick 增长。
        combined.update_context(&GoalContext {
            self_position: [1.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([0.0, 0.0, 0.0]),
        });
        combined.tick();
        assert_eq!(combined.navigation_target(), None);
        // 远程距离内：接近目标。
        combined.update_context(&GoalContext {
            self_position: [30.0, 0.0, 0.0],
            target: Some(target),
            target_position: Some([0.0, 0.0, 0.0]),
        });
        combined.tick();
        assert_eq!(combined.navigation_target(), Some([0.0, 0.0, 0.0]));
    }
}
