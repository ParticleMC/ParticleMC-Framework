//! 线程模型抽象：`ThreadProvider` trait 占位 + [`StdThreadProvider`] 实现。
//!
//! **边界说明**：Rust 侧以 `std::thread` + tokio（异步运行时）为准，**不复制
//! Java `ThreadProvider` 的线程分派实现**（Java 的 chunk 线程亲和 / 刷新策略在
//! Rust 中由 tokio 与 旧 ECS 方案 schedule 承担）。本 trait 仅供应用自定义线程策略时
//! 注入使用，默认实现 [`StdThreadProvider`] 直接委托 `std::thread`。
//!
//! 变更标识符：`complete-missing-subsystems`（R15 utils/coordinate/thread 工具层）。
//! 见 `.specs/complete-missing-subsystems/spec.md`。

use std::time::Duration;

/// 应用可注入的线程策略接口。
///
/// - [`spawn`](Self::spawn)：在一个新线程中执行闭包（不等待其完成）。
/// - [`sleep`](Self::sleep)：阻塞当前线程指定时长。
///
/// 该 trait 是**接口占位**：Rust 生态默认使用 `std::thread` 与 tokio，本 trait
/// 仅为需要自定义线程策略（如接入自定义线程池 / 命名线程）的应用提供扩展点。
/// 相比 Java `ThreadProvider`（按分区分配线程并支持刷新），此处有意省略
/// 线程亲和与刷新语义。
pub trait ThreadProvider: Send + Sync {
    /// 在后台线程中执行 `f`，调用方不等待其完成。
    fn spawn<F: FnOnce() + Send + 'static>(&self, f: F);

    /// 阻塞当前线程 `duration` 时长。
    fn sleep(&self, duration: Duration);
}

/// 标准线程策略：`spawn` 用 `std::thread::spawn`，`sleep` 用
/// `std::thread::sleep`。
pub struct StdThreadProvider;

impl ThreadProvider for StdThreadProvider {
    fn spawn<F: FnOnce() + Send + 'static>(&self, f: F) {
        // 线程句柄由系统接管，框架不 join，与 Java 的 fire-and-forget 语义一致。
        std::thread::spawn(f);
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    #[test]
    fn std_provider_spawn_executes_closure() {
        let provider = StdThreadProvider;
        let (tx, rx) = mpsc::channel();
        provider.spawn(move || {
            // 闭包在新线程中执行并回传结果。
            tx.send(42).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), 42);
    }

    #[test]
    fn std_provider_sleep_blocks_for_duration() {
        let provider = StdThreadProvider;
        let start = Instant::now();
        provider.sleep(Duration::from_millis(15));
        assert!(start.elapsed() >= Duration::from_millis(15));
    }
}
