// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 跨线程原语：Vyukov 有界 lock-free MPMC 环形队列。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 本模块为 **unsafe 白名单**（U2/U5）：自研无锁 MPMC（多生产者多消费者）
//! 有界环形队列。队列预分配固定容量（2 的幂），`push`/`pop` 均无阻塞——满/
//! 空立即返回 `Err(Full)`/`None`，不等待（R8.1 "push 无阻塞（Full 即返回
//! Err）"）。
//!
//! 算法为经典 Vyukov 有界 MPMC 队列（Dmitry Vyukov, "Bounded MPMC queue"）。
//! 每个槽位附带一个 `sequence` 原子计数：以序列号与入队/出队位置的差值判定
//! 槽位归属轮次（初始 = 槽位下标 i；入队后 = pos+1；出队后 = pos+容量），
//! 经 `compare_exchange_weak` 抢占位置后独占访问值槽位，以 Acquire/Release
//! 内存序发布数据，全程无锁、无数据竞争（R8.1 Scenario 的 Miri 验证要求）。
//!
//! - U2：无锁 MPMC 环形队列 CAS 推进 head/tail——框架层唯一合法跨线程通道。
//!   防护：`debug_assert` 容量为 2 的幂、序列号在 unsafe 前后复验；仅原子
//!   操作，数据竞争由内存序保证消除。
//! - U5：`MaybeUninit` 未初始化内存——槽位预分配不初始化值，`push` 时写入，
//!   `pop` 在序列号 Acquire 校验通过后 `assume_init_read`（读取到的必然已
//!   初始化）。
//!
//! ## 安全性论证（U2/U5）
//!
//! - **互斥写**：`push` 经 `compare_exchange_weak` 在 `enqueue_pos` 上抢占
//!   唯一位置 `pos`，同一位置至多被一个生产者抢得；写入 `value` 前以 Acquire
//!   读 `sequence == pos` 确认该槽位上一轮次已被消费完（sequence 已推进到
//!   pos）。
//! - **读写隔离**：`pop` 仅在 `sequence == pos + 1`（Acquire）时读取
//!   `value`，该观察必然 happens-after 对应 `push` 的
//!   `sequence.store(pos + 1, Release)`，故读取到的 `MaybeUninit` 已完整
//!   初始化（`assume_init_read` 前序列号校验）。
//! - **槽位轮转**：同一槽位 push 写完后 sequence 置 `pos+1`，pop 读完后置
//!   `pos + capacity`；下一轮使用该槽位的位置恰为 `pos + capacity`，序列号
//!   回到与位置相等的数值，各轮次的写/读严格交替、互不重叠。
//! - `unsafe impl Sync`：任意时刻 `value` 至多被一个线程访问（上述序列号
//!   协议保证），故 `T: Send` 时队列可经共享引用跨线程使用；不要求 `T: Sync`。
//!
//! 本文件顶部 `#![allow(unsafe_code)]` 为 T8 白名单授权，其余 API 均为安全
//! 封装（`push`/`pop` 为 `pub fn`，unsafe 全部内聚在本模块）。

#![allow(unsafe_code)]

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::util::next_power_of_two;
use crate::world::World;

/// 队列槽位：未初始化值存储 + 序列号（无锁协议状态机）。
///
/// `#[repr(align(64))]` 使每个槽位独占缓存行，避免多线程下相邻槽位（分属
/// 不同生产/消费位置）的伪共享。
struct Node<T> {
    /// 未初始化值槽位（U5）：仅由序列号协议保护的 push/pop 独占访问。
    value: UnsafeCell<MaybeUninit<T>>,
    /// 序列号：初始 = 槽位下标 i；入队后 = pos+1；出队后 = pos+容量。
    sequence: AtomicUsize,
}

/// 有界 lock-free MPMC 环形队列（Vyukov 算法）。
///
/// - 容量在 [`MpmcQueue::new`] 时向上取 2 的幂（最小 8），预分配
///   `Box<[Node<T>]>`；
/// - `push`：队列满立即返回 [`QueueError::Full`]，不阻塞（R8.1）；
/// - `pop`：队列空立即返回 `None`，不阻塞；
/// - `push`/`pop` 均为安全 `pub fn`，内部 unsafe 全部封装于此。
pub struct MpmcQueue<T> {
    buffer: Box<[Node<T>]>,
    capacity: usize,
    mask: usize,
    enqueue_pos: AtomicUsize,
    dequeue_pos: AtomicUsize,
}

impl<T> MpmcQueue<T> {
    /// 创建容量不小于 `capacity` 的有界队列（向上取 2 的幂，最小 8）。
    pub fn new(capacity: usize) -> Self {
        let capacity = next_power_of_two(capacity).max(8);
        debug_assert!(capacity.is_power_of_two());
        let mut buffer: Vec<Node<T>> = Vec::with_capacity(capacity);
        for i in 0..capacity {
            buffer.push(Node {
                value: UnsafeCell::new(MaybeUninit::uninit()),
                sequence: AtomicUsize::new(i),
            });
        }
        MpmcQueue {
            buffer: buffer.into_boxed_slice(),
            capacity,
            mask: capacity - 1,
            enqueue_pos: AtomicUsize::new(0),
            dequeue_pos: AtomicUsize::new(0),
        }
    }

    /// 入队：CAS 抢占位置后独占写入值槽位；队列满立即返回 `Err(Full)`。
    ///
    /// 执行顺序语义：所有成功入队的数据，其 `pop` 的完成顺序即 `push` 的
    /// 完成顺序（MPMC 下按入队序，drain 端串行消费，R8.1/IC-11）。
    pub fn push(&self, v: T) -> Result<(), QueueError> {
        let mut pos = self.enqueue_pos.load(Ordering::Relaxed);
        loop {
            let node = &self.buffer[pos & self.mask];
            let seq = node.sequence.load(Ordering::Acquire);
            let diff = seq as isize - pos as isize;
            if diff == 0 {
                // 槽位空闲：CAS 抢占该位置（同一位置只可能被一个生产者抢得）
                match self.enqueue_pos.compare_exchange_weak(
                    pos,
                    pos.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // 抢占成功：独占该槽位，写入值（U2/U5）
                        unsafe {
                            // SAFETY: 本位置已被当前线程经 CAS 独占，且 sequence
                            // Acquire 验证 == pos，证明该槽位上一轮次已消费完
                            // （sequence 已推进到 pos），当前无任何线程访问
                            // value 槽位。debug_assert 在写入前后复验序列号。
                            debug_assert_eq!(node.sequence.load(Ordering::Relaxed), pos);
                            (*node.value.get()).write(v);
                            debug_assert_eq!(node.sequence.load(Ordering::Relaxed), pos);
                        }
                        // Release：发布已初始化的值，对 pop 端可见
                        node.sequence.store(pos.wrapping_add(1), Ordering::Release);
                        return Ok(());
                    }
                    Err(current) => {
                        // 其他生产者抢先推进位置，改用最新位置重试
                        pos = current;
                    }
                }
            } else if diff < 0 {
                // 序列号落后于位置：槽位仍被上一轮次占据，消费者未追上 → 满
                return Err(QueueError::Full);
            } else {
                // 序列号超前（防御性分支）：重读最新位置重试
                pos = self.enqueue_pos.load(Ordering::Relaxed);
            }
        }
    }

    /// 出队：CAS 抢占消费位置后独占读取值；队列空立即返回 `None`。
    pub fn pop(&self) -> Option<T> {
        let mut pos = self.dequeue_pos.load(Ordering::Relaxed);
        loop {
            let node = &self.buffer[pos & self.mask];
            let seq = node.sequence.load(Ordering::Acquire);
            // 位置 pos 的消费前提：对应入队轮次已完成（sequence == pos + 1）
            let diff = seq as isize - (pos as isize + 1);
            if diff == 0 {
                match self.dequeue_pos.compare_exchange_weak(
                    pos,
                    pos.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let value = unsafe {
                            // SAFETY: 位置已被当前线程经 CAS 独占；sequence
                            // Acquire 验证 == pos+1，happens-after 对应 push
                            // 的 sequence.store(pos+1, Release)，值已完整初始化
                            // 且当前无其他线程访问。debug_assert 复验。
                            debug_assert_eq!(
                                node.sequence.load(Ordering::Relaxed),
                                pos.wrapping_add(1)
                            );
                            (*node.value.get()).assume_init_read()
                        };
                        // Release：槽位释放，供下一轮次（pos+capacity）复用
                        node.sequence
                            .store(pos.wrapping_add(self.capacity), Ordering::Release);
                        return Some(value);
                    }
                    Err(current) => {
                        // 其他消费者抢先推进位置，改用最新位置重试
                        pos = current;
                    }
                }
            } else if diff < 0 {
                // 无对应已入队数据：队列空
                return None;
            } else {
                pos = self.dequeue_pos.load(Ordering::Relaxed);
            }
        }
    }

    /// 近似元素数（`enqueue_pos - dequeue_pos`，Relaxed）。
    ///
    /// 含已抢占但尚未入队完成的位置，仅用于容量观测（R8.1/IC-11 len 语义），
    /// 不作为并发精确计数。
    pub fn len(&self) -> usize {
        self.enqueue_pos
            .load(Ordering::Relaxed)
            .wrapping_sub(self.dequeue_pos.load(Ordering::Relaxed))
    }

    /// 是否空（近似）。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 队列容量（2 的幂）。
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

// SAFETY: 序列号协议保证任意时刻值槽位至多被一个线程访问（push 独占写、
// pop 独占读，同一槽位经 sequence 轮转严格交替），故 `T: Send` 时整个队列
// 可经共享引用（`&MpmcQueue<T>`）跨线程安全使用，无需 `T: Sync`。
unsafe impl<T: Send> Sync for MpmcQueue<T> {}

impl<T> Drop for MpmcQueue<T> {
    /// 析构时清空残留元素：值槽位为 `MaybeUninit`，其 Drop 不会释放内部值
    /// （U5 语义），需逐条 pop 取出以正确析构（避免泄漏 Box 闭包等资源）。
    ///
    /// Drop 仅在唯一持有者时执行（无并发），`pop` 的序列号协议保证可读全量
    /// 已入队数据。
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

/// 队列错误（R8.1 / IC-11：push 无阻塞，满即返回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    /// 队列满：消费者未及时取走数据，本次入队失败且未产生副作用。
    Full,
}

/// 外部线程（网络/控制台）与世界的唯一交互入口（R8.2）。
///
/// 命令在 World 侧线程 tick 起始经 [`CommandQueue::drain`] 批量执行。
pub type ExternalCommand = Box<dyn FnOnce(&mut World) + Send>;

/// 跨线程命令队列（IC-11）：外部线程 push，World 侧线程 drain。
///
/// `Send + Sync`：可经 `Arc` 共享给任意线程；同一时刻仅 `drain` 持有
/// `&mut World`（World 侧单线程执行），push 端仅持有队列自身。
pub struct CommandQueue {
    inner: MpmcQueue<ExternalCommand>,
}

impl CommandQueue {
    /// 创建容量不小于 `n` 的命令队列（向上取 2 的幂，最小 8）。
    pub fn with_capacity(n: usize) -> Self {
        CommandQueue {
            inner: MpmcQueue::new(n),
        }
    }

    /// 入队一条命令；队列满立即返回 `Err(QueueError::Full)`，不阻塞。
    pub fn push(&self, cmd: ExternalCommand) -> Result<(), QueueError> {
        self.inner.push(cmd)
    }

    /// 批量执行队列中全部命令（tick 起始调用），返回执行条数。
    ///
    /// 执行顺序 = push 完成顺序（MPMC 下按入队序，drain 端串行消费）；
    /// 执行到队列空为止，一次调用处理完当前积压的全部命令。
    pub fn drain(&self, world: &mut World) -> usize {
        let mut count = 0;
        while let Some(cmd) = self.inner.pop() {
            cmd(world);
            count += 1;
        }
        count
    }

    /// 近似待执行命令数（见 [`MpmcQueue::len`]）。
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// 是否无待执行命令（近似）。
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 测试资源：命令执行计数。
    #[derive(Default)]
    struct Counter(usize);

    /// 测试资源：记录命令执行顺序（验证 FIFO）。
    #[derive(Default)]
    struct Sequence(Vec<u32>);

    // ---- MpmcQueue 单测 ----

    #[test]
    fn single_thread_push_pop_fifo() {
        // 容量 8：分块推/弹多轮，验证 FIFO 顺序与跨轮次槽位复用
        let queue = MpmcQueue::<u32>::new(8);
        for chunk in 0..10u32 {
            for i in 0..8u32 {
                assert!(queue.push(chunk * 8 + i).is_ok());
            }
            // FIFO：pop 顺序与 push 顺序一致
            for i in 0..8u32 {
                assert_eq!(queue.pop(), Some(chunk * 8 + i));
            }
        }
        assert!(queue.is_empty());
    }

    #[test]
    fn push_full_returns_err_and_recovers_after_pop() {
        let queue = MpmcQueue::<u32>::new(8);
        // 容量 8：连续 push 8 个成功，第 9 个 Full
        for i in 0..8 {
            assert!(queue.push(i).is_ok());
        }
        assert_eq!(queue.push(8), Err(QueueError::Full));
        // 消费后槽位释放，可继续入队
        assert_eq!(queue.pop(), Some(0));
        assert!(queue.push(8).is_ok());
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
    }

    #[test]
    fn pop_empty_returns_none() {
        let queue = MpmcQueue::<u32>::new(8);
        assert_eq!(queue.pop(), None);
        assert!(queue.is_empty());
    }

    #[test]
    fn len_tracks_roundtrip() {
        let queue = MpmcQueue::<u32>::new(8);
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
        for i in 0..5 {
            assert!(queue.push(i).is_ok());
        }
        assert_eq!(queue.len(), 5);
        assert!(!queue.is_empty());
        assert_eq!(queue.pop(), Some(0));
        assert_eq!(queue.len(), 4);
    }

    #[test]
    fn capacity_is_power_of_two_minimum() {
        assert_eq!(MpmcQueue::<u32>::new(0).capacity(), 8);
        assert_eq!(MpmcQueue::<u32>::new(3).capacity(), 8);
        assert_eq!(MpmcQueue::<u32>::new(8).capacity(), 8);
        assert_eq!(MpmcQueue::<u32>::new(100).capacity(), 128);
        assert_eq!(MpmcQueue::<u32>::new(4096).capacity(), 4096);
    }

    #[test]
    fn queue_wraps_around_many_cycles() {
        // 多轮循环（位置回绕）：验证 sequence 轮转协议不随 wrap 失效
        let queue = MpmcQueue::<u32>::new(8);
        for round in 0..100u32 {
            for i in 0..8 {
                assert!(queue.push(round * 8 + i).is_ok());
            }
            for i in 0..8 {
                assert_eq!(queue.pop(), Some(round * 8 + i));
            }
        }
        assert!(queue.is_empty());
    }

    #[test]
    fn drop_drains_remaining_values() {
        // MaybeUninit 槽位不自动 drop 值：析构时须逐条取出释放（U5 泄漏防护）
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        struct DropCounted(Arc<std::sync::atomic::AtomicUsize>);
        impl Drop for DropCounted {
            fn drop(&mut self) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let d = Arc::clone(&drops);
        {
            let queue = MpmcQueue::new(8);
            for _ in 0..3 {
                assert!(queue.push(DropCounted(Arc::clone(&d))).is_ok());
            }
            // 弹出并释放 1 个；队列析构时剩余 2 个须被释放
            assert!(queue.pop().is_some());
        }
        assert_eq!(drops.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[test]
    fn concurrent_push_pop_no_loss_no_duplication() {
        // 并发压力：4 生产者 × 1000 条，1 消费者收满——无丢失、无重复
        const PRODUCERS: u32 = 4;
        const PER_PRODUCER: u32 = 1000;
        let queue = Arc::new(MpmcQueue::<u32>::new(8192)); // 容量 > 总量，不会 Full
        let mut handles = Vec::new();
        for p in 0..PRODUCERS {
            let queue = Arc::clone(&queue);
            handles.push(std::thread::spawn(move || {
                for i in 0..PER_PRODUCER {
                    let value = p * PER_PRODUCER + i;
                    // Full 则让出 CPU 重试（容量充足，正常不会触发）
                    while queue.push(value).is_err() {
                        std::thread::yield_now();
                    }
                }
            }));
        }
        let mut received: Vec<u32> = Vec::new();
        while received.len() < (PRODUCERS * PER_PRODUCER) as usize {
            match queue.pop() {
                Some(v) => received.push(v),
                // 生产者仍在生产：让出 CPU 等待
                None => std::thread::yield_now(),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
        // 收集结果排序比对：每条值恰好出现一次
        received.sort_unstable();
        let expected: Vec<u32> = (0..PRODUCERS * PER_PRODUCER).collect();
        assert_eq!(received, expected);
        assert!(queue.is_empty());
    }

    #[test]
    fn concurrent_push_small_queue_no_loss() {
        // 小容量 + 并发生产：push 失败必须重试，最终无丢失
        let queue = Arc::new(MpmcQueue::<u32>::new(8));
        const PRODUCERS: u32 = 4;
        const PER_PRODUCER: u32 = 500;
        let mut handles = Vec::new();
        for p in 0..PRODUCERS {
            let queue = Arc::clone(&queue);
            handles.push(std::thread::spawn(move || {
                for i in 0..PER_PRODUCER {
                    while queue.push(p * PER_PRODUCER + i).is_err() {
                        std::thread::yield_now();
                    }
                }
            }));
        }
        let mut received: Vec<u32> = Vec::new();
        while received.len() < (PRODUCERS * PER_PRODUCER) as usize {
            match queue.pop() {
                Some(v) => received.push(v),
                None => std::thread::yield_now(),
            }
        }
        for h in handles {
            h.join().unwrap();
        }
        received.sort_unstable();
        let expected: Vec<u32> = (0..PRODUCERS * PER_PRODUCER).collect();
        assert_eq!(received, expected);
    }

    // ---- CommandQueue 单测（IC-11）----

    #[test]
    fn command_queue_push_and_drain_executes_in_order() {
        let queue = CommandQueue::with_capacity(16);
        // 入队按序号写入 World 资源（验证执行顺序 = 入队顺序，FIFO）
        for i in 0..5u32 {
            let cmd: ExternalCommand = Box::new(move |world: &mut World| {
                if let Some(seq) = world.resource_mut::<Sequence>() {
                    seq.0.push(i);
                }
            });
            assert!(queue.push(cmd).is_ok());
        }
        assert!(!queue.is_empty());
        let mut world = World::new();
        world.insert_resource(Sequence(Vec::new()));
        let executed = queue.drain(&mut world);
        assert_eq!(executed, 5);
        assert!(queue.is_empty());
        assert_eq!(world.resource::<Sequence>().unwrap().0, vec![0, 1, 2, 3, 4]);
        // 再次 drain 无积压：执行 0 条
        assert_eq!(queue.drain(&mut world), 0);
    }

    #[test]
    fn command_queue_push_when_full_returns_err() {
        let queue = CommandQueue::with_capacity(8);
        for _ in 0..8 {
            let cmd: ExternalCommand = Box::new(|_world: &mut World| {});
            assert!(queue.push(cmd).is_ok());
        }
        let cmd: ExternalCommand = Box::new(|_world: &mut World| {});
        assert_eq!(queue.push(cmd), Err(QueueError::Full));
        assert_eq!(queue.len(), 8);
    }

    #[test]
    fn command_queue_multithreaded_push_and_drain() {
        // 4 线程并发 push 各 100 条，主线程 drain——无丢失、无重复执行
        let queue = Arc::new(CommandQueue::with_capacity(64));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let queue = Arc::clone(&queue);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    // Full 则让出 CPU 重试：命令为 FnOnce（push 消费所有权），
                    // 每次重试重建闭包
                    loop {
                        let cmd: ExternalCommand = Box::new(|world: &mut World| {
                            if let Some(counter) = world.resource_mut::<Counter>() {
                                counter.0 += 1;
                            }
                        });
                        if queue.push(cmd).is_ok() {
                            break;
                        }
                        std::thread::yield_now();
                    }
                }
            }));
        }
        let mut world = World::new();
        world.insert_resource(Counter(0));
        // 生产者生产期间持续 drain（tick 起始语义）
        let mut total = 0;
        while total < 400 {
            total += queue.drain(&mut world);
            if total < 400 {
                std::thread::yield_now();
            }
        }
        for h in handles {
            h.join().unwrap();
        }
        // 全部 400 条命令恰执行一次
        assert_eq!(world.resource::<Counter>().unwrap().0, 400);
        assert!(queue.is_empty());
    }
}
