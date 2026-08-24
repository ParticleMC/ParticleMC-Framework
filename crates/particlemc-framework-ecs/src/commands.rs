// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 延迟命令缓冲（IC-6 / R5）：系统内入队，tick 起始批量 apply。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 对齐 旧 ECS 方案 Commands 语义：`Commands` 持有 [`CommandBuffer`] 的可变借用，
//! 系统执行期间仅入队命令（不触碰 World），每轮 tick 起始由
//! [`Commands::apply`] 按入队顺序批量应用（R5.3 确定性）。
//!
//! 延迟实体（spawn 占位）：apply 前真实句柄未知，以单调递增 token 标识
//! （`next_token` 跨 tick 不复用）；apply 时经 token_map 解析为实际实体。
//! `EntityCommands::id()` 对 spawn 路径返回 [`Entity::PLACEHOLDER`] 占位
//! （文档注明：apply 前不可知，仅作句柄比较），对 `entity(e)` 路径返回 `e`。
//!
//! 借用限制：`EntityCommands` 持有缓冲的可变借用，同一时刻仅能存活一个
//! （与 旧 ECS 方案 一致）；以链式调用或块作用域释放后再创建下一个。
//!
//! 零分配边界（R3.4）：热路径迭代/更新不涉及本模块；命令创建每命令一次
//! `Box` 分配、apply 阶段每轮一次 `HashMap` 分配，均属命令路径而非 tick
//! 热路径；`Vec<Command>` 经 `drain` 清空但保留容量跨 tick 复用（R5.2）。
//!
//! # unsafe 白名单（A9 扩展）
//!
//! 本模块 `#![allow(unsafe_code)]`：`CommandBuffer` 需作为 `Resource` 注入
//! World（T7 Commands 系统参数 + Schedule 每 tick 起始 apply），而
//! `CommandApply` 无 `Send` 约束（A7）使派生 Send/Sync 不成立，故补
//! `unsafe impl Send/Sync`（缓冲跨线程仅随 World 整体迁移，迁移前后无并发
//! 访问，契约见 impl 处文档）。

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::marker::PhantomData;

use crate::archetype::ArchetypeId;
use crate::component::Component;
use crate::entity::Entity;
use crate::world::{Bundle, World};

/// 命令目标：延迟实体（spawn 产生的 token）或已知实体。
#[derive(Copy, Clone)]
enum Target {
    /// spawn 占位 token：apply 时经 token_map 解析为实际实体。
    Token(u32),
    /// 已存在实体句柄（`Commands::entity` / 直接方法路径）。
    Entity(Entity),
}

/// 类型擦除命令：apply 时拿到解析后的实体后执行。
///
/// 不加 `Send` 超约束：`RemoveCmd<T>` 需 `PhantomData<T>: Send`（即 `T: Send`），
/// 而本 crate 的 `Component` 无 `Send` 约束、IC-6 冻结的 `remove<T: Component>`
/// 也不含 `Send` bound——两者不可同时满足。CommandBuffer 在单线程内按 tick
/// 应用，命令本身无跨线程消费者（跨线程通道为 T8 queue.rs 的职责），故去掉
/// `Send`（蓝图内部设计调整，不改 IC-6 冻结签名）。
trait CommandApply {
    fn apply(self: Box<Self>, world: &mut World, entity: Entity);
}

/// 缓冲内的单条命令（按入队顺序应用，R5.3）。
enum Command {
    /// 生成实体：apply 时 `world.spawn(arch)` 并以 token 登记，供后续命令解析。
    Spawn { arch: ArchetypeId, token: u32 },
    /// 销毁实体（target 可能是延迟 token）。
    Despawn { target: Target },
    /// 类型擦除组件操作（insert/remove 等）。
    Typed {
        target: Target,
        apply: Box<dyn CommandApply>,
    },
    /// 延迟 bundle 生成：apply 时经 `Bundle::spawn` 实际创建实体并以 token 登记
    /// （T11：对齐 旧 ECS 方案 `Commands::spawn_bundle`，bundle 装箱延迟到 tick 起始）。
    SpawnBundle {
        apply: Box<dyn BundleSpawn>,
        token: u32,
    },
}

/// 类型擦除 bundle 生成：apply 阶段持有 bundle 并经 `world` 实际生成实体。
///
/// 不加 `Send` 超约束会与 `Command::Typed` 同源（组件无 `Send` 约束）；此处
/// 显式要求 `Send` 使 `Box<dyn BundleSpawn>` 可随 `CommandBuffer` 跨线程迁移
/// （`CommandBuffer` 已 `unsafe impl Send`，契约见其 impl 文档）。
trait BundleSpawn: Send {
    fn spawn(self: Box<Self>, world: &mut World) -> Entity;
}

/// 装箱具体 bundle（B 须 `Bundle + Send`）。
struct BundleSpawnCmd<B: Bundle + Send> {
    bundle: B,
}

impl<B: Bundle + Send> BundleSpawn for BundleSpawnCmd<B> {
    fn spawn(self: Box<Self>, world: &mut World) -> Entity {
        self.bundle.spawn(world)
    }
}

/// 延迟命令缓冲：系统入队 → tick 起始批量应用；跨 tick 复用容量（R5.2）。
pub struct CommandBuffer {
    commands: Vec<Command>,
    /// 下一个 spawn token：单调递增、跨 tick 不复用（防歧义）。
    next_token: u32,
}

impl CommandBuffer {
    /// 空缓冲（commands 无预分配，首次入队时按需扩容）。
    pub fn new() -> Self {
        CommandBuffer {
            commands: Vec::new(),
            next_token: 0,
        }
    }

    /// 是否无待应用命令。
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// 清空待应用命令（保留 Vec 容量复用；token 单调递增不复位）。
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    /// 按入队顺序批量应用全部命令（R5.3）。
    ///
    /// - 命令按队列严格顺序执行；
    /// - spawn 命令以 token 登记实体，后续 Token 目标据此解析；token 未生成
    ///   （防御性，公开 API 不可达）时跳过对应命令；
    /// - 对无效实体的操作被忽略（世界 API 返回 Err/None/false，不 panic）；
    /// - 执行后命令清空，Vec 容量保留（R5.2 复用）；
    /// - apply 阶段一次 `HashMap` 分配用于 token 解析（命令路径，非热路径）。
    fn apply(&mut self, world: &mut World) {
        if self.commands.is_empty() {
            return;
        }
        let mut token_map: HashMap<u32, Entity> = HashMap::new();
        // drain 保容量：元素逐条消费，Vec 分配保留供下轮复用
        for command in self.commands.drain(..) {
            match command {
                Command::Spawn { arch, token } => {
                    let entity = world.spawn(arch);
                    token_map.insert(token, entity);
                }
                Command::SpawnBundle { apply, token } => {
                    let entity = apply.spawn(world);
                    token_map.insert(token, entity);
                }
                Command::Despawn { target } => {
                    if let Some(entity) = resolve_target(&token_map, target) {
                        let _ = world.despawn(entity);
                    }
                }
                Command::Typed { target, apply } => {
                    if let Some(entity) = resolve_target(&token_map, target) {
                        apply.apply(world, entity);
                    }
                }
            }
        }
    }
}

impl Default for CommandBuffer {
    fn default() -> Self {
        CommandBuffer::new()
    }
}

// SAFETY: 命令缓冲在单线程内按 tick 应用（A7），命令值归缓冲独占；跨线程仅
// 随 World 整体迁移（R9 调度器 tick_all），迁移前后无并发访问（同一 World
// 单线程 tick）。故此 Send/Sync 契约成立，使 CommandBuffer 可作为 Resource
// 注入 World（T7 Commands 系统参数 + Schedule 每 tick 起始 apply 用）。
unsafe impl Send for CommandBuffer {}
unsafe impl Sync for CommandBuffer {}

/// 解析命令目标为实际实体：Token 查 token_map（缺 → None，跳过），
/// 已知实体直接返回。
fn resolve_target(token_map: &HashMap<u32, Entity>, target: Target) -> Option<Entity> {
    match target {
        Target::Token(token) => token_map.get(&token).copied(),
        Target::Entity(entity) => Some(entity),
    }
}

/// 延迟命令入口（IC-6）：系统内仅入队，tick 起始 `apply` 批量生效。
pub struct Commands<'w> {
    /// pub(crate)：供 T7 SystemParam 从 `&mut CommandBuffer` 构造。
    pub(crate) buffer: &'w mut CommandBuffer,
}

impl<'w> Commands<'w> {
    /// 从命令缓冲构造（T7 SystemParam 装配用）。
    pub fn new(buffer: &'w mut CommandBuffer) -> Self {
        Commands { buffer }
    }

    /// 入队一条 spawn 命令，返回延迟实体句柄。
    ///
    /// apply 前真实实体未知，`EntityCommands::id()` 返回占位符
    /// [`Entity::PLACEHOLDER`]（仅作句柄比较，文档注明）。
    pub fn spawn(&mut self, arch: ArchetypeId) -> EntityCommands<'_> {
        let token = self.buffer.next_token;
        // 饱和递增：u32 上限（2^32 条命令）物理不可达，饱和仅形式性兜底
        self.buffer.next_token = self.buffer.next_token.saturating_add(1);
        self.buffer.commands.push(Command::Spawn { arch, token });
        EntityCommands {
            buffer: &mut *self.buffer,
            target: Target::Token(token),
            id: Entity::PLACEHOLDER,
        }
    }

    /// 入队一条 bundle 生成命令，返回延迟实体句柄（T11：对齐 旧 ECS 方案
    /// `Commands::spawn_bundle`）。bundle 装箱延迟到 tick 起始经 `Bundle::spawn`
    /// 实际生成；apply 前真实实体未知，`EntityCommands::id()` 返回占位符。
    pub fn spawn_bundle<B: Bundle + Send + 'static>(&mut self, bundle: B) -> EntityCommands<'_> {
        let token = self.buffer.next_token;
        self.buffer.next_token = self.buffer.next_token.saturating_add(1);
        self.buffer.commands.push(Command::SpawnBundle {
            apply: Box::new(BundleSpawnCmd { bundle }),
            token,
        });
        EntityCommands {
            buffer: &mut *self.buffer,
            target: Target::Token(token),
            id: Entity::PLACEHOLDER,
        }
    }

    /// 绑定已知实体：返回其命令句柄（`id()` 即 `e` 本身）。
    pub fn entity(&mut self, e: Entity) -> EntityCommands<'_> {
        EntityCommands {
            buffer: &mut *self.buffer,
            target: Target::Entity(e),
            id: e,
        }
    }

    /// 入队 despawn 命令。
    pub fn despawn(&mut self, e: Entity) {
        self.buffer.commands.push(Command::Despawn {
            target: Target::Entity(e),
        });
    }

    /// 入队 insert 命令（对无效实体 apply 时 Err 被忽略，不 panic）。
    pub fn insert<T: Component + Default + Send + Sync>(&mut self, e: Entity, c: T) {
        self.buffer.commands.push(Command::Typed {
            target: Target::Entity(e),
            apply: Box::new(InsertCmd { value: c }),
        });
    }

    /// 入队 remove 命令（对无效实体 apply 时返回 None，被忽略）。
    pub fn remove<T: Component>(&mut self, e: Entity) {
        self.buffer.commands.push(Command::Typed {
            target: Target::Entity(e),
            apply: Box::new(RemoveCmd::<T>(PhantomData)),
        });
    }

    /// 批量应用全部已入队命令（消费自身，释放缓冲借用）。
    pub fn apply(self, world: &mut World) {
        self.buffer.apply(world);
    }
}

/// 实体命令句柄（IC-6）：对延迟（spawn）或已知实体链式追加命令。
pub struct EntityCommands<'c> {
    buffer: &'c mut CommandBuffer,
    target: Target,
    id: Entity,
}

impl<'c> EntityCommands<'c> {
    /// 目标实体句柄。
    ///
    /// - `entity(e)` 路径：返回 `e`；
    /// - `spawn` 路径：apply 前真实实体未知，返回 [`Entity::PLACEHOLDER`]
    ///   （仅作句柄比较，文档注明）。
    pub fn id(&self) -> Entity {
        self.id
    }

    /// 链式入队 insert 命令。
    pub fn insert<T: Component + Default + Send + Sync>(&mut self, c: T) -> &mut Self {
        self.buffer.commands.push(Command::Typed {
            target: self.target,
            apply: Box::new(InsertCmd { value: c }),
        });
        self
    }

    /// 链式入队 remove 命令。
    pub fn remove<T: Component>(&mut self) -> &mut Self {
        self.buffer.commands.push(Command::Typed {
            target: self.target,
            apply: Box::new(RemoveCmd::<T>(PhantomData)),
        });
        self
    }

    /// 入队 despawn 命令。
    pub fn despawn(&mut self) {
        self.buffer.commands.push(Command::Despawn {
            target: self.target,
        });
    }
}

/// insert 命令（类型擦除装箱）。
struct InsertCmd<T: Component + Default + Send + Sync + 'static> {
    value: T,
}

impl<T: Component + Default + Send + Sync + 'static> CommandApply for InsertCmd<T> {
    fn apply(self: Box<Self>, world: &mut World, entity: Entity) {
        // Err 忽略：实体可能已被 despawn 等命令销毁，不 panic
        let _ = world.insert(entity, self.value);
    }
}

/// remove 命令（类型擦除装箱；值经 world.remove 返回后丢弃）。
struct RemoveCmd<T: Component + 'static>(PhantomData<T>);

impl<T: Component + 'static> CommandApply for RemoveCmd<T> {
    fn apply(self: Box<Self>, world: &mut World, entity: Entity) {
        let _ = world.remove::<T>(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetype::ArchetypeDef;
    use crate::component::{ComponentId, ComponentStorage};
    use crate::entity::{EntityTypeId, Generation, Slot};

    // ---- 测试组件（手工实现 Component，避免依赖宏 crate）----

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    impl Component for Position {
        fn id() -> ComponentId {
            ComponentId(1)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Velocity {
        dx: f32,
        dy: f32,
    }

    impl Component for Velocity {
        fn id() -> ComponentId {
            ComponentId(2)
        }
        const STORAGE: ComponentStorage = ComponentStorage::SoA;
        type Registry = ();
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Health {
        hp: u32,
    }

    impl Component for Health {
        fn id() -> ComponentId {
            ComponentId(3)
        }
        const STORAGE: ComponentStorage = ComponentStorage::Sparse;
        type Registry = ();
    }

    // ---- 测试 Archetype 定义（'static，可直接注册）----

    /// 玩家：SoA 组件 Position + Velocity，实体类型 1。
    static PLAYER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(0),
        name: "PlayerArchetype",
        component_ids: &[ComponentId(1), ComponentId(2)],
        entity_kind: EntityTypeId(1),
        component_types: &[],
    };

    /// 怪物：SoA 组件 Position + Sparse 组件 Health，实体类型 2。
    static MONSTER_DEF: ArchetypeDef = ArchetypeDef {
        id: ArchetypeId(1),
        name: "MonsterArchetype",
        component_ids: &[ComponentId(1), ComponentId(3)],
        entity_kind: EntityTypeId(2),
        component_types: &[],
    };

    /// 注册全部测试 Archetype 的新世界。
    fn fresh_world() -> World {
        let mut world = World::new();
        world.register_archetype(&PLAYER_DEF);
        world.register_archetype(&MONSTER_DEF);
        world
    }

    #[test]
    fn spawn_multiple_entities_after_apply() {
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands.spawn(ArchetypeId(0));
            commands.spawn(ArchetypeId(0));
            commands.spawn(ArchetypeId(1));
            commands.apply(&mut world);
        }
        assert_eq!(world.entity_count(), 3);
        // 全新世界：槽位升序 = 生成顺序，句柄可直接定位
        let p1 = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        let p2 = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(1));
        let m1 = Entity::from_parts(EntityTypeId(2), Generation(0), Slot(0));
        assert!(world.contains(p1));
        assert!(world.contains(p2));
        assert!(world.contains(m1));
    }

    #[test]
    fn entity_commands_chained_insert_visible() {
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands
                .spawn(ArchetypeId(0))
                .insert(Position { x: 5.0, y: 6.0 })
                .insert(Velocity { dx: 1.0, dy: -2.0 });
            commands.apply(&mut world);
        }
        assert_eq!(world.entity_count(), 1);
        let e = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        assert_eq!(world.get::<Position>(e), Some(&Position { x: 5.0, y: 6.0 }));
        assert_eq!(
            world.get::<Velocity>(e),
            Some(&Velocity { dx: 1.0, dy: -2.0 })
        );
    }

    #[test]
    fn despawn_applied_makes_entity_gone() {
        let mut world = fresh_world();
        let p = world.spawn(ArchetypeId(0));
        let m = world.spawn(ArchetypeId(1));
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands.despawn(p);
            commands.apply(&mut world);
        }
        assert!(!world.contains(p));
        assert!(world.contains(m));
        assert_eq!(world.entity_count(), 1);
    }

    #[test]
    fn entity_commands_id_returns_entity_or_placeholder() {
        let mut world = fresh_world();
        let e = world.spawn(ArchetypeId(0));
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            {
                // entity 路径：id 即实体本身
                let ec = commands.entity(e);
                assert_eq!(ec.id(), e);
            }
            {
                // spawn 路径：apply 前 id 为占位（仅作句柄比较，文档注明）；
                // 块作用域释放 ec 借用后再创建下一个句柄
                let sc = commands.spawn(ArchetypeId(0));
                assert_eq!(sc.id(), Entity::PLACEHOLDER);
            }
        }
    }

    #[test]
    fn entity_path_insert_remove_applies() {
        let mut world = fresh_world();
        let e = world.spawn(ArchetypeId(0));
        let _ = world.insert(e, Health { hp: 10 });
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands
                .entity(e)
                .insert(Position { x: 3.0, y: 4.0 })
                .insert(Health { hp: 99 })
                .remove::<Health>();
            commands.apply(&mut world);
        }
        assert_eq!(world.get::<Position>(e), Some(&Position { x: 3.0, y: 4.0 }));
        // Health 按入队顺序：先覆盖 99 再移除 → 最终不可见
        assert!(world.get::<Health>(e).is_none());
    }

    #[test]
    fn commands_direct_insert_remove_apply() {
        let mut world = fresh_world();
        let e = world.spawn(ArchetypeId(0));
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands.insert(e, Position { x: 7.0, y: 8.0 });
            commands.remove::<Position>(e);
            commands.apply(&mut world);
        }
        // SoA remove 语义：重置默认值（不移除列，列仍存在）
        assert_eq!(world.get::<Position>(e), Some(&Position::default()));
    }

    #[test]
    fn apply_obeys_queue_order_with_late_token_reference() {
        // 精确模拟蓝图场景：spawn A → spawn B → insert 到 A。
        // 公开 API 下 EntityCommands 同一时刻仅能存活一个，无法同时持有 A/B
        // 句柄，故直接构造等价队列验证「延迟 token 在后续命令中仍正确解析」。
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        buffer.commands.push(Command::Spawn {
            arch: ArchetypeId(0),
            token: 0,
        });
        buffer.commands.push(Command::Spawn {
            arch: ArchetypeId(0),
            token: 1,
        });
        buffer.commands.push(Command::Typed {
            target: Target::Token(0),
            apply: Box::new(InsertCmd {
                value: Position { x: 1.0, y: 2.0 },
            }),
        });
        buffer.commands.push(Command::Typed {
            target: Target::Token(1),
            apply: Box::new(InsertCmd {
                value: Position { x: 9.0, y: 9.0 },
            }),
        });
        buffer.apply(&mut world);
        assert_eq!(world.entity_count(), 2);
        // 全新世界：槽位升序 = 生成顺序，组件归属必须与 token 一致
        let a = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        let b = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(1));
        assert_eq!(world.get::<Position>(a), Some(&Position { x: 1.0, y: 2.0 }));
        assert_eq!(world.get::<Position>(b), Some(&Position { x: 9.0, y: 9.0 }));
    }

    #[test]
    fn commands_queue_order_deterministic() {
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            // 入队顺序：A 的 spawn+insert 先入队，B 后入队
            commands
                .spawn(ArchetypeId(0))
                .insert(Position { x: 1.0, y: 2.0 });
            commands
                .spawn(ArchetypeId(0))
                .insert(Position { x: 9.0, y: 9.0 });
            commands.apply(&mut world);
        }
        assert_eq!(world.entity_count(), 2);
        let a = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(0));
        let b = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(1));
        assert!(world.contains(a));
        assert!(world.contains(b));
        assert_eq!(world.get::<Position>(a), Some(&Position { x: 1.0, y: 2.0 }));
        assert_eq!(world.get::<Position>(b), Some(&Position { x: 9.0, y: 9.0 }));
    }

    #[test]
    fn deferred_spawn_then_despawn_in_same_tick() {
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            {
                // 同一缓冲内先建后销：token 解析完整生命周期
                let mut ec = commands.spawn(ArchetypeId(0));
                ec.insert(Position { x: 1.0, y: 2.0 });
                ec.despawn();
                // 块作用域释放 ec 借用，随后 apply
            }
            commands.apply(&mut world);
        }
        // 若 despawn 未正确解析 token，则残留 1 个实体
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn invalid_entity_commands_ignored_no_panic() {
        let mut world = fresh_world();
        let ghost = Entity::from_parts(EntityTypeId(1), Generation(0), Slot(999));
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands.insert(ghost, Position { x: 1.0, y: 1.0 });
            commands.remove::<Health>(ghost);
            commands.despawn(ghost);
            commands.apply(&mut world); // 不 panic，全部忽略
        }
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn unknown_token_commands_skipped_no_panic() {
        // 防御性场景（公开 API 不可达）：Token 指向从未 spawn 的实体
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        buffer.commands.push(Command::Typed {
            target: Target::Token(42),
            apply: Box::new(InsertCmd {
                value: Position::default(),
            }),
        });
        buffer.commands.push(Command::Despawn {
            target: Target::Token(42),
        });
        buffer.apply(&mut world);
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn buffer_reuses_capacity_across_applies() {
        let mut world = fresh_world();
        let mut buffer = CommandBuffer::new();
        // 首轮：100 条命令触发 Vec 扩容
        for token in 0..100u32 {
            buffer.commands.push(Command::Spawn {
                arch: ArchetypeId(0),
                token,
            });
        }
        let capacity = buffer.commands.capacity();
        assert!(capacity >= 100);
        buffer.apply(&mut world);
        assert!(buffer.is_empty());
        // apply 后清空但容量保留（drain 保分配）
        assert_eq!(buffer.commands.capacity(), capacity);
        assert_eq!(world.entity_count(), 100);
        // 次轮入队/应用：容量不变
        buffer.commands.push(Command::Despawn {
            target: Target::Token(0),
        });
        assert_eq!(buffer.commands.capacity(), capacity);
        buffer.apply(&mut world);
        assert_eq!(buffer.commands.capacity(), capacity);
    }

    #[test]
    fn buffer_is_empty_and_clear_preserves_capacity() {
        let mut buffer = CommandBuffer::new();
        assert!(buffer.is_empty());
        for token in 0..64u32 {
            buffer.commands.push(Command::Spawn {
                arch: ArchetypeId(0),
                token,
            });
        }
        assert!(!buffer.is_empty());
        let capacity = buffer.commands.capacity();
        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.commands.capacity(), capacity);
    }

    #[test]
    fn entity_commands_despawn_and_commands_despawn() {
        let mut world = fresh_world();
        let p = world.spawn(ArchetypeId(0));
        let m = world.spawn(ArchetypeId(1));
        let mut buffer = CommandBuffer::new();
        {
            let mut commands = Commands::new(&mut buffer);
            commands.entity(p).despawn();
            commands.despawn(m);
            commands.apply(&mut world);
        }
        assert!(!world.contains(p));
        assert!(!world.contains(m));
        assert_eq!(world.entity_count(), 0);
    }
}
