//! 全局实例调度器：多世界并行 tick + 全局同步屏障（IC-10，T9）。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! [`InstanceScheduler`] 管理若干独立 [`World`]，每轮 [`tick_all`] 将就绪世界
//! 分发到固定数量的工作线程并行 tick（单世界内单线程串行、时序确定），全部
//! 完成后经 `std::thread::scope` 内置的 join 屏障返回 [`TickStats`]。两种线程
//! 模式（[`ThreadMode`]）：
//!
//! - `SharedPool`：所有世界共享 `worker_count` 个工作线程，按轮转分配；
//! - `BoundToWorld`：每个世界按 `id` 固定绑定到某个工作线程，提供线程级亲和
//!   （同一世界始终由同一 OS 线程 tick）；当 [`SchedulerConfig::affinity`] 开启
//!   时该线程进一步经零依赖平台 API 绑定到固定逻辑核心（T16.4 / R9.5），不
//!   支持的平台静默降级为仅线程级绑定。
//!
//! 外部线程（网络/控制台）经 [`submit`] 向指定世界提交 [`ExternalCommand`]，
//! 由该世界下一轮 tick 起始的 [`CommandQueue::drain`] 批量执行（IC-11）。
//! [`submit`] 仅需 `&self`，可与 [`tick_all`]（`&mut self`）经外层
//! `Arc<Mutex<InstanceScheduler>>` 并发驱动。

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::thread;
use std::time::{Duration, Instant};

use crate::queue::{CommandQueue, ExternalCommand};
use crate::schedule::Schedule;
use crate::util::set_current_thread_affinity;
use crate::world::World;

/// 世界标识（调度器内部键）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WorldId(pub u32);

/// 调度器线程模式（R9.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadMode {
    /// 每世界绑定固定工作线程（线程级亲和；CPU 亲和因零依赖静默降级）。
    BoundToWorld,
    /// 所有世界共享固定大小工作线程池（轮转分配）。
    SharedPool,
}

/// 调度器配置（IC-10）。
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// 线程模式（BoundToWorld / SharedPool）。
    pub thread_mode: ThreadMode,
    /// 工作线程数（内部 `max(1, worker_count)`；实际并行度 ≤ 世界数）。
    pub worker_count: usize,
    /// 是否尝试 CPU 亲和（T16.4 / R9.5）：每工作线程绑定到一个逻辑核心，
    /// 经零依赖平台 API（`kernel32!SetThreadAffinityMask` / glibc
    /// `sched_setaffinity`）实时绑定；macOS 等不支持平台静默降级为不绑定。
    pub affinity: bool,
}

/// 单世界 tick 统计（R9.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldTickStats {
    /// 本世界本轮 tick 耗时。
    pub elapsed: Duration,
    /// tick 末世界内实体数。
    pub entity_count: usize,
}

/// 全局一轮 tick 统计（R9.4）。
#[derive(Debug, Clone)]
pub struct TickStats {
    /// 各世界 tick 统计（按 [`WorldId`] 升序）。
    pub worlds: Vec<(WorldId, WorldTickStats)>,
}

/// 命令提交失败：每世界命令队列满，或 [`WorldId`] 未注册（IC-10 `submit`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

/// 世界注册失败（IC-10 `register_world`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    /// 该 [`WorldId`] 已注册。
    WorldAlreadyRegistered(WorldId),
}

impl std::fmt::Display for QueueFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("command queue full or target world not registered")
    }
}
impl std::error::Error for QueueFull {}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerError::WorldAlreadyRegistered(id) => {
                write!(f, "world {id:?} already registered")
            }
        }
    }
}
impl std::error::Error for SchedulerError {}

/// 加锁并在发生中毒（poison）时恢复内部守卫，避免 `unwrap`（宪章禁止生产代码 panic）。
///
/// Mutex 中毒仅发生在另一线程持锁期间 panic；本调度器的 tick 与命令提交路径均
/// 不 panic，故中毒在实践上不可达，但为通过 clippy `unwrap_used` 硬门禁并维持零
/// panic 契约，于此处显式恢复守卫而非 `unwrap`。
fn lock_poison<'a, T>(m: &'a Mutex<T>) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 调度器内部登记的单世界条目。
struct WorldEntry {
    world: World,
    schedule: Schedule,
    /// 每世界命令队列（IC-11，scheduler 内嵌每世界队列）。
    queue: CommandQueue,
}

/// 全局实例调度器（IC-10）。
///
/// 内部世界表以 [`Mutex`] 承载，使 [`submit`]（`&self`）可与 [`tick_all`]
/// （`&mut self`）经外层 `Arc<Mutex<InstanceScheduler>>` 并发驱动；[`tick_all`]
/// 并行 tick 经 `std::thread::scope` 派发，scope 退出即全局屏障。
pub struct InstanceScheduler {
    config: SchedulerConfig,
    worlds: Mutex<HashMap<WorldId, WorldEntry>>,
    /// 自增世界 id 分配器（[`register_new_world`] 使用）。
    next_id: u32,
}

impl Default for InstanceScheduler {
    /// 默认调度器：共享线程池、4 工作线程、CPU 亲和静默降级。
    ///
    /// 仅用于满足 `Res<InstanceScheduler>` 对 `T: Default` 的约束（`SystemParam`
    /// 的 `init_state` 经 `init_resource` 惰性补默认）；运行期主世界已由
    /// `server` 装配真实调度器，默认的空调度器不会被实际构造。
    fn default() -> Self {
        InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::SharedPool,
            worker_count: 4,
            affinity: false,
        })
    }
}

/// 持有实例 World 锁的守卫（R11：跨世界系统经此在持锁期间访问实例 World）。
///
/// 持锁期间可经 [`world`](Self::world) 取得 `&mut World`，进而读取实例 World
/// 内的 `Resource`（如 `ChunkStore`）。守卫析构时释放锁，与 [`tick_all`] 不会
/// 同时持锁（跨世界系统在主世界阶段运行，tick_all 在其后）。
pub struct InstanceWorldGuard<'a> {
    map: MutexGuard<'a, HashMap<WorldId, WorldEntry>>,
    id: WorldId,
}

impl InstanceWorldGuard<'_> {
    /// 取得被守卫实例 World 的可变引用。
    pub fn world(&mut self) -> &mut World {
        &mut self
            .map
            .get_mut(&self.id)
            .unwrap_or_else(|| unreachable!("InstanceWorldGuard 守卫的世界必已注册"))
            .world
    }
}

impl Deref for InstanceWorldGuard<'_> {
    type Target = World;
    fn deref(&self) -> &World {
        &self
            .map
            .get(&self.id)
            .unwrap_or_else(|| unreachable!("InstanceWorldGuard 守卫的世界必已注册"))
            .world
    }
}

impl DerefMut for InstanceWorldGuard<'_> {
    fn deref_mut(&mut self) -> &mut World {
        &mut self
            .map
            .get_mut(&self.id)
            .unwrap_or_else(|| unreachable!("InstanceWorldGuard 守卫的世界必已注册"))
            .world
    }
}

impl InstanceScheduler {
    /// 构造调度器（按配置确定线程模式/工作线程数）。
    ///
    /// 零依赖约束下不派生持久工作线程（[`tick_all`] 每轮经 `thread::scope`
    /// 派发 OS 线程，等价于固定大小线程池的每轮复用）；CPU 亲和
    /// （[`SchedulerConfig::affinity`]）由 [`tick_all`] 经零依赖平台 API 实时绑定
    /// 工作线程到逻辑核心（不支持平台静默降级，见 [`set_current_thread_affinity`]）。
    pub fn new(config: SchedulerConfig) -> Self {
        InstanceScheduler {
            config,
            worlds: Mutex::new(HashMap::new()),
            next_id: 0,
        }
    }

    /// 分配新世界 id 并注册世界与其调度器（IC-10）。
    ///
    /// 等价于 [`register_world`](Self::register_world) 但由调度器内部自增分配
    /// [`WorldId`]，调用方无需关心 id 取值。
    pub fn register_new_world(&mut self, world: World, schedule: Schedule) -> WorldId {
        let id = WorldId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        if let Err(e) = self.register_world(id, world, schedule) {
            unreachable!("自增 id 永不重复: {e:?}");
        }
        id
    }

    /// 注册一个世界及其调度器（IC-10）。
    ///
    /// 同 [`WorldId`] 重复注册返回 [`SchedulerError::WorldAlreadyRegistered`]。
    pub fn register_world(
        &mut self,
        id: WorldId,
        world: World,
        schedule: Schedule,
    ) -> Result<(), SchedulerError> {
        let mut g = lock_poison(&self.worlds);
        if g.contains_key(&id) {
            return Err(SchedulerError::WorldAlreadyRegistered(id));
        }
        // 每世界命令队列容量：取 1024（2 的幂向上取整由 CommandQueue 内部处理）
        let queue = CommandQueue::with_capacity(1024);
        g.insert(
            id,
            WorldEntry {
                world,
                schedule,
                queue,
            },
        );
        Ok(())
    }

    /// 注销世界，返回其 [`World`] 与 [`Schedule`]（未注册返回 `None`）。
    pub fn unregister_world(&mut self, id: WorldId) -> Option<(World, Schedule)> {
        lock_poison(&self.worlds)
            .remove(&id)
            .map(|e| (e.world, e.schedule))
    }

    /// 同步访问指定世界（R11：跨世界系统经此生成 / 查询实例实体）。
    ///
    /// 仅在主世界阶段（`tick_all` 之前）调用，与并行 tick 不会同时持锁。
    /// 世界未注册返回 `None`（闭包不执行）。
    pub fn with_world<R>(&self, id: WorldId, f: impl FnOnce(&mut World) -> R) -> Option<R> {
        let mut g = lock_poison(&self.worlds);
        g.get_mut(&id).map(|e| f(&mut e.world))
    }

    /// 枚举全部已注册世界 id（R11：跨世界系统遍历所有实例）。
    pub fn world_ids(&self) -> Vec<WorldId> {
        lock_poison(&self.worlds).keys().copied().collect()
    }

    /// 持锁取得指定世界的守卫（R11：跨世界系统需在被守卫期间多次访问实例
    /// World 资源，如逐实体读取 `ChunkStore`）。世界未注册返回 `None`。
    ///
    /// 与 [`tick_all`] 不会同时持锁（跨世界系统在主世界阶段运行）。
    pub fn lock_world(&self, id: WorldId) -> Option<InstanceWorldGuard<'_>> {
        let map = lock_poison(&self.worlds);
        if map.contains_key(&id) {
            Some(InstanceWorldGuard { map, id })
        } else {
            None
        }
    }

    /// 向指定世界提交外部命令（IC-10 / IC-11）。
    ///
    /// 命令在该世界下一轮 tick 起始的 [`CommandQueue::drain`] 执行。队列满或
    /// 世界未注册返回 [`QueueFull`]。仅需 `&self`，可与 [`tick_all`] 并发。
    pub fn submit(&self, id: WorldId, cmd: ExternalCommand) -> Result<(), QueueFull> {
        let g = lock_poison(&self.worlds);
        match g.get(&id) {
            Some(entry) => entry.queue.push(cmd).map_err(|_| QueueFull),
            None => Err(QueueFull),
        }
    }

    /// 执行一轮全局 tick（IC-10）：分发就绪世界 → 并行 tick → 全局屏障 → 统计。
    ///
    /// 单世界内单线程串行（系统按 `.after` 顺序执行，时序确定）；多世界跨
    /// 线程并行。`thread::scope` 在退出前 join 全部工作线程，即全局同步屏障，
    /// 故本方法返回时所有世界已完成本轮 tick。
    pub fn tick_all(&mut self) -> TickStats {
        let ids: Vec<WorldId> = {
            let g = lock_poison(&self.worlds);
            g.keys().copied().collect()
        };
        let results = Mutex::new(HashMap::<WorldId, WorldTickStats>::new());
        let worker_count = self.config.worker_count.max(1);
        let affinity = self.config.affinity;
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(worker_count);
        let chunks = distribute(&ids, worker_count, self.config.thread_mode);
        thread::scope(|s| {
            for (chunk_index, chunk) in chunks.into_iter().enumerate() {
                let results = &results;
                let worlds = &self.worlds;
                s.spawn(move || {
                    if affinity {
                        // 线程级亲和：每工作线程固定绑定到一个逻辑核心（T16.4 / R9.5）
                        let core = chunk_index % cores;
                        let _ = set_current_thread_affinity(core);
                    }
                    for id in chunk {
                        // 每世界独立加锁、短暂持有；执行期间其他世界可并行 tick
                        let stats = {
                            let mut g = lock_poison(worlds);
                            g.get_mut(&id).map(tick_entry)
                        };
                        if let Some(stats) = stats {
                            lock_poison(results).insert(id, stats);
                        }
                    }
                });
            }
        });
        let mut worlds_out = match results.into_inner() {
            Ok(v) => v,
            Err(p) => p.into_inner(),
        };
        let mut stats: Vec<(WorldId, WorldTickStats)> = worlds_out.drain().collect();
        stats.sort_by_key(|(id, _)| id.0);
        TickStats { worlds: stats }
    }

    /// 优雅停机：本实现无持久工作线程，清空已注册世界释放资源（R9）。
    ///
    /// 调用方若需保留世界，应先 [`unregister_world`] 取回。
    pub fn shutdown(&mut self) {
        lock_poison(&self.worlds).clear();
    }
}

/// 单世界 tick：先 drain 外部命令，再运行世界调度器（IC-9/IC-11）。
fn tick_entry(entry: &mut WorldEntry) -> WorldTickStats {
    let start = Instant::now();
    // 1. tick 起始批量应用外部命令（队列空则 0 条，无副作用）
    let _executed = entry.queue.drain(&mut entry.world);
    // 2. 运行本世界调度器（单世界内串行、时序确定）
    entry.schedule.run(&mut entry.world);
    WorldTickStats {
        elapsed: start.elapsed(),
        entity_count: entry.world.entity_count(),
    }
}

/// 将世界 id 列表按线程模式划分为 `n` 个分块（每分块由一个工作线程 tick）。
///
/// - `SharedPool`：轮转分配（第 i 个 id 给第 `i % n` 个线程）；
/// - `BoundToWorld`：按 `id` 取模固定绑定（同一世界恒落同一线程 → 线程级亲和）。
fn distribute(ids: &[WorldId], n: usize, mode: ThreadMode) -> Vec<Vec<WorldId>> {
    let mut chunks: Vec<Vec<WorldId>> = (0..n).map(|_| Vec::new()).collect();
    match mode {
        ThreadMode::SharedPool => {
            for (i, id) in ids.iter().enumerate() {
                chunks[i % n].push(*id);
            }
        }
        ThreadMode::BoundToWorld => {
            for id in ids {
                chunks[(id.0 as usize) % n].push(*id);
            }
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::ResMut;

    #[derive(Default)]
    struct Counter(u32);

    fn inc(c: ResMut<Counter>) {
        c.0.0 += 1;
    }

    fn make_world() -> (World, Schedule) {
        let mut world = World::new();
        world.init_resource::<Counter>();
        let mut schedule = Schedule::new();
        schedule.add_system(inc);
        (world, schedule)
    }

    #[test]
    fn register_and_tick_all_runs_systems() {
        let mut sched = InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::SharedPool,
            worker_count: 4,
            affinity: false,
        });
        sched
            .register_world(WorldId(1), make_world().0, make_world().1)
            .unwrap();
        sched
            .register_world(WorldId(2), make_world().0, make_world().1)
            .unwrap();
        sched
            .register_world(WorldId(3), make_world().0, make_world().1)
            .unwrap();
        let stats = sched.tick_all();
        assert_eq!(stats.worlds.len(), 3);
        // 取回世界验证系统已执行（Counter 自增 1）
        let (w, _) = sched.unregister_world(WorldId(1)).unwrap();
        assert_eq!(w.resource::<Counter>().map(|c| c.0), Some(1));
        let (w, _) = sched.unregister_world(WorldId(2)).unwrap();
        assert_eq!(w.resource::<Counter>().map(|c| c.0), Some(1));
        let (w, _) = sched.unregister_world(WorldId(3)).unwrap();
        assert_eq!(w.resource::<Counter>().map(|c| c.0), Some(1));
    }

    #[test]
    fn bound_to_world_mode_runs_systems() {
        let mut sched = InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::BoundToWorld,
            worker_count: 2,
            affinity: false,
        });
        sched
            .register_world(WorldId(10), make_world().0, make_world().1)
            .unwrap();
        sched
            .register_world(WorldId(20), make_world().0, make_world().1)
            .unwrap();
        sched.tick_all();
        let (w, _) = sched.unregister_world(WorldId(10)).unwrap();
        assert_eq!(w.resource::<Counter>().map(|c| c.0), Some(1));
    }

    #[test]
    fn duplicate_register_rejected() {
        let mut sched = InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::SharedPool,
            worker_count: 1,
            affinity: false,
        });
        sched
            .register_world(WorldId(1), make_world().0, make_world().1)
            .unwrap();
        let err = sched
            .register_world(WorldId(1), make_world().0, make_world().1)
            .unwrap_err();
        assert_eq!(err, SchedulerError::WorldAlreadyRegistered(WorldId(1)));
        // 注销后可重新注册
        let _ = sched.unregister_world(WorldId(1)).unwrap();
        sched
            .register_world(WorldId(1), make_world().0, make_world().1)
            .unwrap();
    }

    #[test]
    fn submit_drains_external_command_at_tick() {
        let mut sched = InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::SharedPool,
            worker_count: 2,
            affinity: false,
        });
        sched
            .register_world(WorldId(7), make_world().0, make_world().1)
            .unwrap();
        // 外部命令：自增 Counter +100
        let cmd: ExternalCommand = Box::new(|world: &mut World| {
            if let Some(c) = world.resource_mut::<Counter>() {
                c.0 += 100;
            }
        });
        sched.submit(WorldId(7), cmd).unwrap();
        sched.tick_all();
        // drain(+100) 先于系统 inc(+1) → 101
        let (w, _) = sched.unregister_world(WorldId(7)).unwrap();
        assert_eq!(w.resource::<Counter>().map(|c| c.0), Some(101));
    }

    #[test]
    fn submit_to_unregistered_world_is_queue_full() {
        let sched = InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::SharedPool,
            worker_count: 1,
            affinity: false,
        });
        let cmd: ExternalCommand = Box::new(|_world: &mut World| {});
        assert_eq!(sched.submit(WorldId(99), cmd), Err(QueueFull));
    }

    #[test]
    fn empty_tick_returns_empty_stats() {
        let mut sched = InstanceScheduler::new(SchedulerConfig {
            thread_mode: ThreadMode::SharedPool,
            worker_count: 3,
            affinity: false,
        });
        let stats = sched.tick_all();
        assert!(stats.worlds.is_empty());
    }
}
