//! 任务调度器（框架层，见 `.specs/implement-framework-capabilities/` R9）。
//!
//! [`TaskScheduler`] 是 [`SchedulerManager`](crate::resource::scheduler_manager)
//! 占位实现的**升级版**（BREAKING：旧 `schedule()` / `cancel_one()` 移除，
//! `pending()` 保留但语义改为「待执行任务数」）。以 旧 ECS 方案 `Resource` 形式挂载，
//! 为框架与应用提供 tick 驱动的延迟 / 周期任务：
//!
//! - [`schedule_after`](TaskScheduler::schedule_after)：延迟 `delay_ticks` 后执行一次
//! - [`schedule_repeat`](TaskScheduler::schedule_repeat)：每 `period_ticks` 执行一次
//! - [`schedule_at_tick`](TaskScheduler::schedule_at_tick)：在绝对 tick 执行一次
//! - [`schedule_next_tick`](TaskScheduler::schedule_next_tick)：下个 tick 执行一次
//! - [`schedule_end_of_tick`](TaskScheduler::schedule_end_of_tick)：tick 结束阶段执行
//! - [`tick_begin`](TaskScheduler::tick_begin) / [`tick_end`](TaskScheduler::tick_end)：
//!   双阶段推进（分离执行时机）
//! - [`tick`](TaskScheduler::tick)：向后兼容，依次调用 `tick_begin` + `tick_end`
//! - [`cancel`](TaskScheduler::cancel)：取消任务（堆中懒删除）
//!
//! 内部用 `BinaryHeap<Reverse<(due_tick, id)>>` 按到期 tick 升序排列，`tick`
//! 循环取出 `due <= current_tick` 的任务执行；周期任务执行后按 `due + period`
//! 重新入堆。任务闭包约束 `Send + Sync`（满足 旧 ECS 方案 `Resource` 要求）。
//!
//! 系统接线（`scheduler_tick`，挂 `Schedule`）由 T28 统一接入。

#![forbid(unsafe_code)]

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// 任务执行时机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionType {
    /// 在 tick 开始阶段执行。
    TickStart,
    /// 在 tick 结束阶段执行。
    TickEnd,
}

/// 任务句柄：由 [`TaskScheduler`] 分配的稳定标识，用于取消任务。
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct TaskId(u64);

/// 单个任务条目。
struct TaskEntry {
    /// 是否为周期任务（执行后重新入堆）。
    is_repeat: bool,
    /// 周期（tick）；非周期任务为 0。
    period: u64,
    /// 任务执行时机。
    execution_type: ExecutionType,
    /// 任务闭包（`FnMut`，一次性任务经内部包装消费）。
    f: Box<dyn FnMut() + Send + Sync>,
}

/// 任务调度器（旧 ECS 方案 `Resource`）。
///
/// 由 [`crate::plugin::McServerPlugin`] 装配或在应用侧自行插入。调度器不依赖
/// 旧 ECS 方案 时间，`tick` 由调用方（T28 的系统接线）以当前 tick 驱动。
///
/// 支持双阶段执行：[`tick_begin`](Self::tick_begin) 在 tick 开始阶段处理
/// `TickStart` 任务，[`tick_end`](Self::tick_end) 在 tick 结束阶段处理
/// `TickEnd` 任务；[`tick`](Self::tick) 为两者合并的向后兼容入口。
#[derive(Default)]
pub struct TaskScheduler {
    /// tick 开始阶段到期队列：`Reverse<(due_tick, task_id)>`，按到期 tick 升序。
    queue: BinaryHeap<Reverse<(u64, u64)>>,
    /// tick 结束阶段到期队列（TICK_END 专用）。
    end_queue: BinaryHeap<Reverse<(u64, u64)>>,
    /// 任务表：task_id → 条目。`cancel` 直接移除；堆中的残留条目在 `tick` 取出
    /// 时因查表失败被跳过（懒删除）。
    tasks: HashMap<u64, TaskEntry>,
    /// 下一个任务 id（`wrapping_add` 防溢出 panic）。
    next_id: u64,
    /// 最近一次 `tick` 的当前 tick（供 `schedule_after` 计算到期点）。
    current_tick: u64,
}

impl TaskScheduler {
    /// 延迟 `delay_ticks` 后执行一次任务，返回 [`TaskId`]。
    ///
    /// 到期点为 `当前 tick + delay_ticks`；当前 tick 取自最近一次 [`tick`](Self::tick)。
    pub fn schedule_after(
        &mut self,
        delay_ticks: u64,
        f: impl FnOnce() + Send + Sync + 'static,
    ) -> TaskId {
        let due = self.current_tick.saturating_add(delay_ticks);
        self.insert(due, false, 0, ExecutionType::TickStart, once_as_mut(f))
    }

    /// 每 `period_ticks` 执行一次周期任务，返回 [`TaskId`]。
    ///
    /// 首次到期点为 `当前 tick + period_ticks`；执行后按 `到期 tick + period_ticks`
    /// 重新入堆。可用 [`cancel`](Self::cancel) 停止。
    pub fn schedule_repeat(
        &mut self,
        period_ticks: u64,
        f: impl FnMut() + Send + Sync + 'static,
    ) -> TaskId {
        let due = self.current_tick.saturating_add(period_ticks);
        self.insert(
            due,
            true,
            period_ticks,
            ExecutionType::TickStart,
            Box::new(f),
        )
    }

    /// 在绝对 tick 执行一次任务，返回 [`TaskId`]。
    ///
    /// 若目标 tick 已过去，则下次 [`tick`](Self::tick) 立即执行。
    pub fn schedule_at_tick(
        &mut self,
        tick: u64,
        f: impl FnOnce() + Send + Sync + 'static,
    ) -> TaskId {
        self.insert(tick, false, 0, ExecutionType::TickStart, once_as_mut(f))
    }

    /// 在下个 tick 执行一次任务（等价于 [`schedule_after`](Self::schedule_after)(1)）。
    pub fn schedule_next_tick(&mut self, f: impl FnOnce() + Send + Sync + 'static) -> TaskId {
        self.schedule_after(1, f)
    }

    /// 在 tick 结束阶段执行一次任务。
    ///
    /// 到期点为当前 tick（即本次 `tick_end` 调用时立即执行）。
    pub fn schedule_end_of_tick(&mut self, f: impl FnOnce() + Send + Sync + 'static) -> TaskId {
        let due = self.current_tick;
        self.insert(due, false, 0, ExecutionType::TickEnd, once_as_mut(f))
    }

    /// 延迟 `delay_ticks` 后执行一次（[`schedule_after`] 的语义别名，T14.4）。
    ///
    /// 命名对齐 Minestom 原生命名 `runAfter`，便于既有调用方零成本迁移。
    pub fn run_after(
        &mut self,
        delay_ticks: u64,
        f: impl FnOnce() + Send + Sync + 'static,
    ) -> TaskId {
        self.schedule_after(delay_ticks, f)
    }

    /// 每 `period_ticks` 执行一次周期任务（[`schedule_repeat`] 的语义别名，T14.4）。
    ///
    /// 命名对齐 Minestom 原生命名 `runRepeating`，便于既有调用方零成本迁移。
    pub fn run_repeating(
        &mut self,
        period_ticks: u64,
        f: impl FnMut() + Send + Sync + 'static,
    ) -> TaskId {
        self.schedule_repeat(period_ticks, f)
    }

    /// 取消一个任务，返回是否已取消成功（任务尚在待执行表中）。
    ///
    /// 堆中残留的到期项不会被移除（懒删除）；`tick` 取出时因查表失败而跳过。
    pub fn cancel(&mut self, id: TaskId) -> bool {
        self.tasks.remove(&id.0).is_some()
    }

    /// 推进调度器 tick 开始阶段，执行所有到期 TICK_START 任务。
    pub fn tick_begin(&mut self, current_tick: u64) {
        self.current_tick = current_tick;
        self.drain_queue(current_tick, ExecutionType::TickStart);
    }

    /// 推进调度器 tick 结束阶段，执行所有到期 TICK_END 任务。
    pub fn tick_end(&mut self, current_tick: u64) {
        self.current_tick = current_tick;
        self.drain_queue(current_tick, ExecutionType::TickEnd);
    }

    /// 推进调度器：依次调用 [`tick_begin`](Self::tick_begin) 与
    /// [`tick_end`](Self::tick_end)，向后兼容旧 `tick` 行为。
    pub fn tick(&mut self, current_tick: u64) {
        self.tick_begin(current_tick);
        self.tick_end(current_tick);
    }

    /// 当前待执行任务数（含已到期未推进、未取消的周期任务）。
    pub fn pending(&self) -> usize {
        self.tasks.len()
    }

    /// 从指定执行类型的队列中取出并执行全部到期任务。
    fn drain_queue(&mut self, current_tick: u64, execution_type: ExecutionType) {
        loop {
            let (due, id) = match execution_type {
                ExecutionType::TickStart => match self.queue.peek().copied() {
                    Some(Reverse((due, id))) => (due, id),
                    None => break,
                },
                ExecutionType::TickEnd => match self.end_queue.peek().copied() {
                    Some(Reverse((due, id))) => (due, id),
                    None => break,
                },
            };
            if due > current_tick {
                break;
            }
            match execution_type {
                ExecutionType::TickStart => {
                    let _ = self.queue.pop();
                }
                ExecutionType::TickEnd => {
                    let _ = self.end_queue.pop();
                }
            }
            let Some(mut entry) = self.tasks.remove(&id) else {
                continue;
            };
            if entry.execution_type != execution_type {
                continue;
            }
            (entry.f)();
            if entry.is_repeat {
                let next_due = due.saturating_add(entry.period);
                self.tasks.insert(id, entry);
                match execution_type {
                    ExecutionType::TickStart => {
                        self.queue.push(Reverse((next_due, id)));
                    }
                    ExecutionType::TickEnd => {
                        self.end_queue.push(Reverse((next_due, id)));
                    }
                }
            }
        }
    }

    /// 登记任务：分配 id、入堆、入表。
    fn insert(
        &mut self,
        due_tick: u64,
        is_repeat: bool,
        period: u64,
        execution_type: ExecutionType,
        f: Box<dyn FnMut() + Send + Sync>,
    ) -> TaskId {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        match execution_type {
            ExecutionType::TickStart => self.queue.push(Reverse((due_tick, id))),
            ExecutionType::TickEnd => self.end_queue.push(Reverse((due_tick, id))),
        }
        self.tasks.insert(
            id,
            TaskEntry {
                is_repeat,
                period,
                execution_type,
                f,
            },
        );
        TaskId(id)
    }
}

/// 把 `FnOnce` 包装为 `FnMut`：首次调用消费闭包执行，之后为空操作。
fn once_as_mut<F>(f: F) -> Box<dyn FnMut() + Send + Sync>
where
    F: FnOnce() + Send + Sync + 'static,
{
    let mut f = Some(f);
    Box::new(move || {
        if let Some(f) = f.take() {
            f();
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn schedule_after_executes_once_on_due_tick() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        sched.schedule_after(5, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });

        for t in 1..=4 {
            sched.tick(t);
            assert_eq!(counter.load(Ordering::Relaxed), 0, "tick {t} 不应执行");
        }
        sched.tick(5);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        // 之后不再执行
        sched.tick(6);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert_eq!(sched.pending(), 0);
    }

    #[test]
    fn schedule_repeat_every_n_ticks_then_cancel() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let id = sched.schedule_repeat(3, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });

        // 到期 tick：3、6、9
        sched.tick(1);
        sched.tick(2);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        sched.tick(3);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        sched.tick(6);
        assert_eq!(counter.load(Ordering::Relaxed), 2);

        assert!(sched.cancel(id));
        // 取消后 tick 9 不再执行
        sched.tick(9);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
        assert_eq!(sched.pending(), 0);
    }

    #[test]
    fn schedule_at_tick_executes_at_absolute_tick() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        sched.schedule_at_tick(7, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });

        sched.tick(6);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        sched.tick(7);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        sched.tick(8);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn cancel_pending_task_prevents_execution() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let id = sched.schedule_after(10, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        assert!(sched.cancel(id));
        assert!(!sched.cancel(id)); // 二次取消失败

        for t in 1..=12 {
            sched.tick(t);
        }
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        assert_eq!(sched.pending(), 0);
    }

    #[test]
    fn empty_queue_tick_does_not_panic() {
        let mut sched = TaskScheduler::default();
        sched.tick(0);
        sched.tick(100);
        assert_eq!(sched.pending(), 0);
    }

    #[test]
    fn pending_counts_scheduled_tasks() {
        let mut sched = TaskScheduler::default();
        assert_eq!(sched.pending(), 0);
        let _a = sched.schedule_after(1, || {});
        let b = sched.schedule_after(2, || {});
        assert_eq!(sched.pending(), 2);
        sched.cancel(b);
        assert_eq!(sched.pending(), 1);
    }

    #[test]
    fn task_ids_are_distinct() {
        let mut sched = TaskScheduler::default();
        let a = sched.schedule_after(1, || {});
        let b = sched.schedule_after(1, || {});
        assert_ne!(a, b);
        // TaskId 可哈希、可比较
        let mut set = std::collections::HashSet::new();
        set.insert(a);
        set.insert(b);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn schedule_after_uses_last_seen_tick() {
        // 在 tick(5) 之后调度的任务以 5 为基准
        let mut sched = TaskScheduler::default();
        sched.tick(5);
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        sched.schedule_after(3, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });

        sched.tick(7);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        sched.tick(8);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn run_after_alias_executes_like_schedule_after() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        sched.run_after(4, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });

        for t in 1..=3 {
            sched.tick(t);
            assert_eq!(counter.load(Ordering::Relaxed), 0, "tick {t} 不应执行");
        }
        sched.tick(4);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        sched.tick(5);
        assert_eq!(counter.load(Ordering::Relaxed), 1, "run_after 只执行一次");
    }

    #[test]
    fn run_repeating_alias_executes_like_schedule_repeat() {
        let mut sched = TaskScheduler::default();
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let id = sched.run_repeating(2, move || {
            c.fetch_add(1, Ordering::Relaxed);
        });

        sched.tick(2);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        sched.tick(4);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
        assert!(sched.cancel(id));
        // 取消后不再执行
        sched.tick(6);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
        assert_eq!(sched.pending(), 0);
    }

    // ---- 新增测试：双阶段执行 ----

    /// schedule_next_tick 等价于 schedule_after(1)，应在下一 tick 执行。
    #[test]
    fn schedule_next_tick_executes_on_next_tick() {
        let mut sched = TaskScheduler::default();
        // 先调用一次 tick，确保 current_tick 初始化为 0
        sched.tick(0);
        let executed = Arc::new(AtomicU32::new(0));
        let e = executed.clone();
        // current_tick = 0，schedule_next_tick → due = 0 + 1 = 1
        sched.schedule_next_tick(move || {
            e.fetch_add(1, Ordering::Relaxed);
        });

        // tick(1) 应执行（due=1 <= current_tick=1）
        sched.tick(1);
        assert_eq!(executed.load(Ordering::Relaxed), 1);
        // tick(2) 不应再次执行
        sched.tick(2);
        assert_eq!(executed.load(Ordering::Relaxed), 1, "不应再次执行");
        assert_eq!(sched.pending(), 0);
    }

    /// schedule_end_of_tick 仅在 tick_end 阶段执行，不在 tick_begin 阶段执行。
    #[test]
    fn schedule_end_of_tick_executes_in_tick_end_only() {
        let mut sched = TaskScheduler::default();
        // 先调用 tick(0) 初始化 current_tick = 0
        sched.tick(0);
        let end_executed = Arc::new(AtomicU32::new(0));
        let e = end_executed.clone();
        // schedule_end_of_tick 此时 due = current_tick = 0
        sched.schedule_end_of_tick(move || {
            e.fetch_add(1, Ordering::Relaxed);
        });

        // 仅调 tick_begin(0)：当前 due=0 <= 0，但这是 TickEnd 任务，不被 drain_queue(TickStart) 处理
        sched.tick_begin(0);
        assert_eq!(
            end_executed.load(Ordering::Relaxed),
            0,
            "tick_begin 不应执行 TickEnd 任务"
        );

        // tick_end(0)：due=0 <= current_tick=0，应执行
        sched.tick_end(0);
        assert_eq!(
            end_executed.load(Ordering::Relaxed),
            1,
            "tick_end 应执行 TickEnd 任务"
        );
    }

    /// tick() 合并调用 tick_begin 和 tick_end，两个阶段的到期任务都能执行。
    #[test]
    fn tick_runs_both_phases() {
        let mut sched = TaskScheduler::default();
        let begin_count = Arc::new(AtomicU32::new(0));
        let end_count = Arc::new(AtomicU32::new(0));
        let b = begin_count.clone();
        let e = end_count.clone();

        // 在 tick(0) 之后，current_tick = 0
        sched.tick(0);

        // schedule_after(1) → due = 0 + 1 = 1（TickStart，在 tick_begin(1) 执行）
        sched.schedule_after(1, move || {
            b.fetch_add(1, Ordering::Relaxed);
        });
        // schedule_end_of_tick → due = 0（TickEnd，在 tick_end(1) 执行，因为 0 <= 1）
        sched.schedule_end_of_tick(move || {
            e.fetch_add(1, Ordering::Relaxed);
        });

        // tick(1) = tick_begin(1) + tick_end(1)
        sched.tick(1);
        assert_eq!(
            begin_count.load(Ordering::Relaxed),
            1,
            "TickStart 任务应在 tick(1) 执行"
        );
        assert_eq!(
            end_count.load(Ordering::Relaxed),
            1,
            "TickEnd 任务应在 tick(1) 执行"
        );
    }

    /// tick_begin 和 tick_end 各自独立处理对应类型的任务，互不干扰。
    #[test]
    fn tick_begin_and_tick_end_are_independent() {
        let mut sched = TaskScheduler::default();
        let start_count = Arc::new(AtomicU32::new(0));
        let end_count = Arc::new(AtomicU32::new(0));
        let s = start_count.clone();
        let e = end_count.clone();

        // 在 tick(0) 之后，current_tick = 0
        sched.tick(0);

        // schedule_after(1) → due = 1（TickStart）
        sched.schedule_after(1, move || {
            s.fetch_add(1, Ordering::Relaxed);
        });
        // schedule_end_of_tick → due = 0（TickEnd，因为 current_tick 此时为 0）
        sched.schedule_end_of_tick(move || {
            e.fetch_add(1, Ordering::Relaxed);
        });

        // 仅调 tick_begin(1)：start 任务因 due=1 <= 1 而执行；
        //   end 任务因类型不匹配（TickEnd vs TickStart）而被跳过
        sched.tick_begin(1);
        assert_eq!(
            start_count.load(Ordering::Relaxed),
            1,
            "tick_begin 应执行 TickStart 任务"
        );
        assert_eq!(
            end_count.load(Ordering::Relaxed),
            0,
            "tick_begin 不应执行 TickEnd 任务"
        );

        // 仅调 tick_end(1)：end 任务因 due=0 <= 1 而执行
        sched.tick_end(1);
        assert_eq!(
            end_count.load(Ordering::Relaxed),
            1,
            "tick_end 应执行 TickEnd 任务"
        );
        // start_count 不应再变化
        assert_eq!(
            start_count.load(Ordering::Relaxed),
            1,
            "tick_end 不应影响 TickStart 计数"
        );
    }
}
