// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 控制台 IO 与运维命令分发（T13，`implement-custom-ecs`）。
//!
//! 设计要点（与 spec 13.1 的偏差及理由，见 `tasks.md` T13 注记）：
//!
//! - **stdin 独立线程**：仅负责读取命令行并经通道发送，**绝不直接读写 World**
//!   （避免与游戏主循环（`app.update` / `tick_all`）发生数据竞争）。这是 spec
//!   「禁止直接读写 World」的核心约束。
//! - **命令经主循环分发**：命令行在主线程的 `run()` 主循环中被取出，交由
//!   [`dispatch`] 执行。该执行点位于主 World 上下文，可安全访问主 World 的
//!   `CommandManager`（玩家聊天命令 `command_chat_system` 同构）与 `InstanceScheduler`
//!   （`status` 需要枚举世界 / 读取 tick 统计）。
//! - 内置运维命令 `stop` / `status` / `help`：`help` 已由 `CommandManager` 内置；
//!   `stop` / `status` 在 [`plugin`] 注册以便 `help` 列表与 `command_exists` 一致，
//!   实际行为由 [`dispatch`] 拦截处理。
//!
//! 本模块不引入任何外部依赖，仅使用标准库（`std::sync` / `std::thread` / `std::io`）。

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use particlemc_framework_ecs::scheduler::{InstanceScheduler, TickStats, WorldId, WorldTickStats};
use particlemc_framework_ecs::world::World;

use crate::resource::command::{CommandManager, ConsoleSender};
use crate::system::TickCounter;

/// 控制台共享状态：停机信号与最近一轮 tick 统计快照。
///
/// 由 `run()` 持有并传入 [`dispatch`]；`stop` 命令置位 `shutdown`，主循环据此退出。
pub struct ConsoleState {
    /// 停机信号（`stop` 命令置位，主循环检测后优雅退出）。
    pub shutdown: Arc<AtomicBool>,
    /// 最近一轮 [`TickStats`] 快照（`status` 命令读取；每轮 `tick_all` 后刷新）。
    pub stats: Arc<Mutex<Option<TickStats>>>,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(None)),
        }
    }
}

/// 控制台句柄：持有 stdin 读取线程的输出接收端与共享状态。
pub struct Console {
    /// stdin 线程发送端对应的接收端（主循环据此取出命令行）。
    input_rx: std::sync::mpsc::Receiver<String>,
    /// 共享状态（停机信号 + tick 统计）。
    state: ConsoleState,
}

impl Console {
    /// 启动控制台：派发独立 stdin 读取线程，返回 [`Console`]（持有输出接收端）。
    ///
    /// 读取线程逐行读取标准输入并发送到内部通道；主循环经 [`Console::drain_input`]
    /// 取出。线程不持有任何 World 引用，纯粹转发文本（满足「禁止直接读写 World」）。
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        thread::spawn(move || {
            let stdin = std::io::stdin();
            let reader = BufReader::new(stdin.lock());
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    // 接收端已关闭（服务器退出），结束读取线程。
                    break;
                }
            }
        });
        Console {
            input_rx: rx,
            state: ConsoleState::default(),
        }
    }

    /// 取出当前缓冲的全部控制台输入行（非阻塞）。
    pub fn drain_input(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(line) = self.input_rx.try_recv() {
            out.push(line);
        }
        out
    }

    /// 共享状态引用（停机信号 / tick 统计快照）。
    pub fn state(&self) -> &ConsoleState {
        &self.state
    }

    /// 是否已收到停机信号（`stop` 命令置位）。
    pub fn should_shutdown(&self) -> bool {
        self.state.shutdown.load(Ordering::SeqCst)
    }
}

/// 分发单条控制台命令，返回输出的文本行（不负责打印，由调用方决定输出端）。
///
/// 分派规则：
/// - `stop`：置位停机信号并返回提示。
/// - `status`：自 `InstanceScheduler` 与 [`ConsoleState`] 采集世界数 / 实体数 /
///   tick 耗时 / 内存占用（best-effort）。
/// - 其余命令：经主 World 的 `CommandManager` 解析执行（与玩家聊天命令同构）。
pub fn dispatch(
    line: &str,
    world: &World,
    scheduler: &InstanceScheduler,
    state: &ConsoleState,
) -> Vec<String> {
    let trimmed = line.trim();
    let mut out = Vec::new();

    if trimmed.is_empty() {
        return out;
    }

    // 内置运维命令：stop / status 由控制台直接处理（需调度器与共享状态）。
    if trimmed.eq_ignore_ascii_case("stop") {
        state.shutdown.store(true, Ordering::SeqCst);
        out.push("正在优雅停机……".to_string());
        return out;
    }
    if trimmed.eq_ignore_ascii_case("status") {
        out.extend(report_status(world, scheduler, state));
        return out;
    }

    // 其余命令经主 World 的 CommandManager 解析执行（help 内置其中）。
    if let Some(mgr) = world.resource::<CommandManager>() {
        mgr.execute(trimmed, &ConsoleSender, &mut |msg: &str| {
            out.push(msg.to_string());
        });
    } else {
        out.push("命令管理器不可用（CommandManager 未注入主 World）".to_string());
    }
    out
}

/// 生成 `status` 命令输出：世界数、全局 tick 计数、各世界 tick 耗时 / 实体数 /
/// 实体按类型分布 / 组件列容量、内存占用（best-effort）。
///
/// - 实体与组件统计跨实例 World 经 [`InstanceScheduler::with_world`] 只读读取（14.2/14.3）；
/// - tick 耗时 / 实体数复用 R9 的 [`TickStats`] 快照（T13 每轮 `tick_all` 后刷新，14.1）。
fn report_status(
    world: &World,
    scheduler: &InstanceScheduler,
    state: &ConsoleState,
) -> Vec<String> {
    let mut lines = Vec::new();
    let world_ids: Vec<WorldId> = scheduler.world_ids();
    lines.push(format!("世界数：{}", world_ids.len()));

    // 全局 tick 计数（TickCounter，TaskScheduler 时钟源，14.1）。
    if let Some(tc) = world.resource::<TickCounter>() {
        lines.push(format!("全局 tick 计数：{}", tc.0));
    }

    // 复制 tick 统计快照后立即释放锁（避免与下方 with_world 的 scheduler 锁长期重叠）。
    let worlds_stats: Vec<(WorldId, WorldTickStats)> = match state.stats.lock() {
        Ok(guard) => guard.as_ref().map(|s| s.worlds.clone()).unwrap_or_default(),
        // 锁被毒化（极端情况）：降级为无统计，不 panic。
        Err(_) => Vec::new(),
    };

    for id in &world_ids {
        let tick = worlds_stats
            .iter()
            .find(|(wid, _)| wid == id)
            .map(|(_, w)| *w);
        // 跨 World 只读读取实体 / 组件统计（14.2/14.3）。
        let live = scheduler.with_world(*id, |w| {
            (
                w.entity_count(),
                w.entities_by_kind(),
                w.component_capacity(),
            )
        });
        match live {
            Some((ent, by_kind, cap)) => {
                if let Some(ws) = tick {
                    lines.push(format!(
                        "  世界 {:?}：实体 {}，tick 耗时 {:?}，组件容量 {}",
                        id, ws.entity_count, ws.elapsed, cap
                    ));
                } else {
                    lines.push(format!("  世界 {:?}：实体 {}，组件容量 {}", id, ent, cap));
                }
                // 按类型细分（取前 16 种，避免输出过长）。
                for (kind, count) in by_kind.iter().take(16) {
                    lines.push(format!("    类型 {}：{} 个", kind.0, count));
                }
                if by_kind.len() > 16 {
                    lines.push(format!("    …（共 {} 种类型）", by_kind.len()));
                }
            }
            None => lines.push(format!("  世界 {:?}：只读访问失败", id)),
        }
    }
    if world_ids.is_empty() {
        lines.push("  （无实例世界）".to_string());
    }

    match memory_bytes() {
        Some(bytes) => lines.push(format!("内存占用：{} bytes", bytes)),
        None => lines.push("内存占用：当前平台不可用（需平台 API）".to_string()),
    }
    lines
}

/// 进程常驻内存占用（best-effort）。
///
/// Linux 读取 `/proc/self/status` 的 `VmRSS`；其余平台（如 Windows / macOS）标准库
/// 无对应接口，返回 `None`，由 `status` 输出标注「不可用」。
#[cfg(target_os = "linux")]
fn memory_bytes() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let field = rest.split_whitespace().next()?;
            let kb: u64 = field.parse().ok()?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

/// 非 Linux 平台无标准库内存探测接口，返回 `None`（见 [`memory_bytes`] 文档）。
#[cfg(not(target_os = "linux"))]
fn memory_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::resource::command::Command;

    /// 构造一个含 CommandManager 与主 World 等价资源的最小 World（用于分发测试）。
    fn test_world() -> World {
        let mut world = World::new();
        world.init_resource::<CommandManager>();
        // 注册 stop / status，使 help 列表与 command_exists 一致（与 plugin.rs 对齐）。
        if let Some(mgr) = world.resource_mut::<CommandManager>() {
            let _ = mgr.register(Command::new("stop", &[]).description("优雅停机服务器"));
            let _ = mgr.register(Command::new("status", &[]).description("显示运行状态"));
        }
        world
    }

    #[test]
    fn dispatch_help_lists_builtin_commands() {
        let world = test_world();
        let sched = InstanceScheduler::default();
        let state = ConsoleState::default();
        let out = dispatch("help", &world, &sched, &state);
        let joined = out.join("\n");
        assert!(joined.contains("stop"), "help 应列出 stop");
        assert!(joined.contains("status"), "help 应列出 status");
    }

    #[test]
    fn dispatch_stop_sets_shutdown_flag() {
        let world = test_world();
        let sched = InstanceScheduler::default();
        let state = ConsoleState::default();
        let out = dispatch("stop", &world, &sched, &state);
        assert!(state.shutdown.load(Ordering::SeqCst), "stop 应置位停机信号");
        assert!(out.iter().any(|l| l.contains("停机")), "stop 应输出提示");
    }

    #[test]
    fn dispatch_status_reports_world_count() {
        let world = test_world();
        let sched = InstanceScheduler::default();
        let state = ConsoleState::default();
        let out = dispatch("status", &world, &sched, &state);
        let joined = out.join("\n");
        assert!(joined.contains("世界数"), "status 应报告世界数");
        assert!(joined.contains("内存占用"), "status 应报告内存占用");
    }

    #[test]
    fn dispatch_unknown_command_reports_error() {
        let world = test_world();
        let sched = InstanceScheduler::default();
        let state = ConsoleState::default();
        let out = dispatch("definitely_not_a_command", &world, &sched, &state);
        assert!(
            out.iter()
                .any(|l| l.contains("未知命令") || l.contains("无效")),
            "未知命令应被报告：{:?}",
            out
        );
    }

    #[test]
    fn dispatch_empty_line_is_noop() {
        let world = test_world();
        let sched = InstanceScheduler::default();
        let state = ConsoleState::default();
        let out = dispatch("   ", &world, &sched, &state);
        assert!(out.is_empty(), "空输入不应产生输出");
    }
}
