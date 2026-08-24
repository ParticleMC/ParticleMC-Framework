// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 通用工具：缓存行对齐包装与扩容辅助。
//!
//! 变更标识符：`implement-custom-ecs`

/// 缓存行对齐包装：将值按 64 字节对齐存放。
///
/// 供后续 SoA 列存储 / 热数据使用（R1.3/R3.1），避免多线程下相邻槽位的
/// 伪共享（false sharing）。
#[repr(align(64))]
pub struct Align64<T>(pub T);

impl<T> Align64<T> {
    /// 便捷构造。
    pub fn new(value: T) -> Self {
        Align64(value)
    }
}

/// 返回大于等于 `n` 的最小 2 的幂；`n == 0` 时返回 1（避免 0 容量）。
pub(crate) fn next_power_of_two(n: usize) -> usize {
    let mut p = 1usize;
    while p < n {
        p = match p.checked_mul(2) {
            Some(next) => next,
            // n 接近 usize::MAX 时 2 的幂超出表示范围，饱和返回（扩容辅助的调用方容量不可能如此大）
            None => return usize::MAX,
        };
    }
    p
}

/// 将**当前 OS 线程**绑定到 `core` 号逻辑核心（T16.4 / R9.5 线程级亲和）。
///
/// 零依赖实现：Windows 经 `kernel32!SetThreadAffinityMask`、Linux 经 glibc
/// `sched_setaffinity`（std 二进制恒链接 libc，无需额外依赖）。macOS / 其他
/// 平台静默返回 `false`（亲和能力缺失，不报错，符合"平台边界注明"原则）。
///
/// 返回 `true` 表示绑定成功。失败（核心越界 / 权限不足 / 平台不支持）返回
/// `false`，调用方据此降级为"不绑定"（不影响 tick 正确性）。
///
/// # 平台说明
///
/// - 绑定作用于 OS 线程，非 CPU 物理核；NUMA 感知需操作系统级策略配合
///   （见 `docs/affinity-numa.md`）。
/// - 本调用不改变调度器逻辑，仅影响线程可运行的核心集合。
#[allow(unsafe_code)]
pub fn set_current_thread_affinity(core: usize) -> bool {
    #[cfg(windows)]
    {
        type Handle = *mut core::ffi::c_void;
        unsafe extern "system" {
            fn GetCurrentThread() -> Handle;
            fn SetThreadAffinityMask(hThread: Handle, dwThreadAffinityMask: usize) -> usize;
        }
        // SAFETY: 由 kernel32 提供的稳定 ABI；入参为当前线程句柄与合法掩码
        let handle = unsafe { GetCurrentThread() };
        let mask: usize = 1usize.checked_shl(core as u32).unwrap_or(0);
        if mask == 0 {
            return false; // 核心索引超出 usize 位宽
        }
        // SAFETY: handle 有效、掩码合法；失败时返回 0，不触发异常
        let ret = unsafe { SetThreadAffinityMask(handle, mask) };
        ret != 0
    }
    #[cfg(all(unix, target_os = "linux"))]
    {
        extern "C" {
            fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u8) -> i32;
        }
        let mut mask: u64 = 1u64.checked_shl(core as u32).unwrap_or(0);
        if mask == 0 {
            return false;
        }
        // SAFETY: pid=0 表示当前线程；cpusetsize 为 u64 大小；mask 指向有效
        // 栈变量，调用期间存活；返回 0 表示成功
        let ret = unsafe {
            sched_setaffinity(
                0,
                core::mem::size_of::<u64>(),
                &mask as *const u64 as *const u8,
            )
        };
        ret == 0
    }
    #[cfg(not(any(windows, all(unix, target_os = "linux"))))]
    {
        let _ = core;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align64_layout() {
        assert_eq!(std::mem::align_of::<Align64<u32>>(), 64);
        // 小尺寸值按对齐填充，size 至少为一个缓存行
        assert_eq!(std::mem::size_of::<Align64<u32>>(), 64);
    }

    #[test]
    fn align64_new_holds_value() {
        let a = Align64::new(7u32);
        assert_eq!(a.0, 7);
        let b = Align64::new("x");
        assert_eq!(b.0, "x");
    }

    #[test]
    fn next_power_of_two_values() {
        assert_eq!(next_power_of_two(0), 1);
        assert_eq!(next_power_of_two(1), 1);
        assert_eq!(next_power_of_two(2), 2);
        assert_eq!(next_power_of_two(3), 4);
        assert_eq!(next_power_of_two(5), 8);
        assert_eq!(next_power_of_two(100), 128);
        // 溢出饱和分支
        assert_eq!(next_power_of_two(usize::MAX), usize::MAX);
    }
}
