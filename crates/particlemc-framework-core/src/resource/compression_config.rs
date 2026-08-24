// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 网络压缩配置（T7 压缩启用）。
//!
//! 控制服务端是否在登录流程中下发`LoginCompression`包并启用 zlib 压缩。
//! `threshold = 0` 表示禁用压缩，`threshold > 0` 表示包体达到该字节数时压缩。
/// 压缩阈值配置（对应 Java 侧的 `MinecraftServer.getCompressionThreshold()`）。
///
/// - 默认阈值 256 字节（Minecraft 官方默认值）。
/// - 环境变量 `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD` 覆盖；解析失败或为负回退默认。
///   值为 `0` 时禁用压缩（登录流程不发送`LoginCompression`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionConfig {
    /// 压缩阈值（字节）。`0` 表示禁用。
    pub threshold: i32,
}

impl Default for CompressionConfig {
    /// 默认压缩阈值 256 字节（与 Minecraft 官方默认一致）。
    fn default() -> Self {
        Self { threshold: 256 }
    }
}

impl CompressionConfig {
    /// 从环境变量构造配置。
    ///
    /// 读取 `PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD`：合法非负整数直接采用（`0` 禁用），
    /// 未设置/解析失败 / 负值一律回退 [`CompressionConfig::default`]（R56）。
    pub fn from_env() -> Self {
        match std::env::var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD") {
            Ok(v) => match v.trim().parse::<i32>() {
                Ok(threshold) if threshold >= 0 => Self { threshold },
                _ => Self::default(),
            },
            Err(_) => Self::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(unsafe_code)]
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::Mutex;

    /// 环境变量为进程级全局状态：串行化相关测试避免并行污染。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_threshold_is_256() {
        assert_eq!(CompressionConfig::default().threshold, 256);
    }

    #[test]
    fn env_overrides_threshold() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            std::env::set_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", "512");
        }
        assert_eq!(CompressionConfig::from_env().threshold, 512);
        unsafe {
            std::env::remove_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD");
        }
    }

    #[test]
    fn env_zero_disables_compression() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            std::env::set_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", "0");
        }
        assert_eq!(CompressionConfig::from_env().threshold, 0);
        unsafe {
            std::env::remove_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD");
        }
    }

    #[test]
    fn env_invalid_falls_back_to_default() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // 非数字
        unsafe {
            std::env::set_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", "abc");
        }
        assert_eq!(CompressionConfig::from_env(), CompressionConfig::default());
        // 负数
        unsafe {
            std::env::set_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD", "-1");
        }
        assert_eq!(CompressionConfig::from_env(), CompressionConfig::default());
        unsafe {
            std::env::remove_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD");
        }
    }

    #[test]
    fn unset_env_uses_default() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            std::env::remove_var("PARTICLE_MCFRAMEWORK_COMPRESSION_THRESHOLD");
        }
        assert_eq!(CompressionConfig::from_env(), CompressionConfig::default());
    }
}
