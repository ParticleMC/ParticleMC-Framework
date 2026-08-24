//! 速度代理转发配置（Velocity Modern Forwarding）。
//!
//! 对应 Java 侧 `MinecraftServer.getVelocityForwardingSecret()` 行为。
//!
//! 配置来源优先级：环境变量 > 配置文件。
//! - `PARTICLE_MCFRAMEWORK_VELOCITY_SECRET`：覆盖配置文件中的 secret。
//! - `PARTICLE_MCFRAMEWORK_VELOCITY_ENFORCE`：覆盖配置文件中的 enforce。
//!
//! 默认值：
//! - secret: None（未启用转发）
//! - enforce: false（非严格模式）

use std::env;
use std::fs;
use std::path::Path;

/// 速度代理转发配置。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
pub struct VelocityConfig {
    /// 密钥（从环境变量或配置文件读取）。
    pub secret: Option<String>,
    /// 是否严格模式（从环境变量或配置文件读取）。
    #[serde(default, rename = "enforce")]
    pub enforce_proxy: bool,
}

impl VelocityConfig {
    /// 加载配置：优先读取环境变量，其次读取配置文件。
    pub fn load() -> Self {
        // 环境变量优先于配置文件。
        let secret = match env::var("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET") {
            Ok(s) if !s.is_empty() => Some(s),
            _ => None,
        };
        if let Some(secret) = secret {
            return VelocityConfig {
                secret: Some(secret),
                enforce_proxy: env::var("PARTICLE_MCFRAMEWORK_VELOCITY_ENFORCE")
                    .map(|v| v == "true")
                    .unwrap_or(false),
            };
        }

        // 无有效环境变量：回退到配置文件。
        let config_path = Path::new("config").join("velocity.toml");
        if config_path.exists()
            && let Ok(content) = fs::read_to_string(&config_path)
            && let Ok(cfg) = toml::from_str::<VelocityConfig>(&content)
        {
            return cfg;
        }

        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// 环境变量与 cwd 均为全局共享状态：用一把锁串行化相关测试，
    /// 避免 cargo 并行运行测试时相互覆盖。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 清理全部 MINESTOM_VELOCITY_* 环境变量。
    fn clear_env() {
        unsafe {
            env::remove_var("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET");
        }
        unsafe {
            env::remove_var("PARTICLE_MCFRAMEWORK_VELOCITY_ENFORCE");
        }
    }

    /// 临时工作目录守卫：在 `std::env::temp_dir()` 下创建唯一子目录，
    /// 写入 `config/velocity.toml` 并切换到该目录；Drop 时恢复原 cwd 并清理。
    struct TempCwd {
        original: PathBuf,
        tmp: PathBuf,
    }

    impl TempCwd {
        /// 创建临时目录并写入 `config/velocity.toml`（内容为 `content`）。
        fn with_file(content: &str) -> Self {
            let original = std::env::current_dir().unwrap();
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let tmp = std::env::temp_dir().join(format!(
                "mc_velocity_test_{}_{}",
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&tmp).unwrap();
            fs::create_dir_all(tmp.join("config")).unwrap();
            fs::write(tmp.join("config").join("velocity.toml"), content).unwrap();
            env::set_current_dir(&tmp).unwrap();
            Self { original, tmp }
        }

        /// 创建空临时目录（无配置文件）。
        fn empty() -> Self {
            let original = std::env::current_dir().unwrap();
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let tmp = std::env::temp_dir().join(format!(
                "mc_velocity_test_empty_{}_{}",
                std::process::id(),
                unique
            ));
            fs::create_dir_all(&tmp).unwrap();
            env::set_current_dir(&tmp).unwrap();
            Self { original, tmp }
        }
    }

    impl Drop for TempCwd {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.original);
            let _ = fs::remove_dir_all(&self.tmp);
        }
    }

    #[test]
    fn default_when_nothing_set() {
        // 无 env、无文件 → 默认（secret=None, enforce=false）。
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_env();
        let _cwd = TempCwd::empty();

        let cfg = VelocityConfig::load();
        assert_eq!(cfg.secret, None);
        assert!(!cfg.enforce_proxy);
    }

    #[test]
    fn env_secret_used() {
        // 环境变量应覆盖配置文件。
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_env();
        let _cwd = TempCwd::with_file("secret=\"file-secret\"\nenforce=true\n");
        unsafe {
            env::set_var("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET", "env-secret");
        }

        let cfg = VelocityConfig::load();
        assert_eq!(cfg.secret.as_deref(), Some("env-secret"));
        // 未设置 enforce 环境变量：env 分支下不会回退文件，故为 false。
        assert!(!cfg.enforce_proxy);
    }

    #[test]
    fn env_enforce_overrides_file() {
        // enforce 环境变量优先于配置文件。
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_env();
        let _cwd = TempCwd::with_file("secret=\"file-secret\"\nenforce=true\n");
        unsafe {
            env::set_var("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET", "env-secret");
        }
        unsafe {
            env::set_var("PARTICLE_MCFRAMEWORK_VELOCITY_ENFORCE", "false");
        }

        let cfg = VelocityConfig::load();
        assert_eq!(cfg.secret.as_deref(), Some("env-secret"));
        assert!(!cfg.enforce_proxy, "env 中的 false 应覆盖文件中的 true");
    }

    #[test]
    fn file_secret_used_when_env_empty() {
        // env 密钥为空字符串时视为未设置，应回退到配置文件。
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_env();
        let _cwd = TempCwd::with_file("secret=\"file-secret\"\nenforce=true\n");
        unsafe {
            env::set_var("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET", "");
        }

        let cfg = VelocityConfig::load();
        assert_eq!(cfg.secret.as_deref(), Some("file-secret"));
        assert!(cfg.enforce_proxy);
    }

    #[test]
    fn env_empty_secret_without_file_is_default() {
        // env 密钥为空且无文件 → 默认（未启用转发）。
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_env();
        let _cwd = TempCwd::empty();
        unsafe {
            env::set_var("PARTICLE_MCFRAMEWORK_VELOCITY_SECRET", "");
        }

        let cfg = VelocityConfig::load();
        assert_eq!(cfg.secret, None);
        assert!(!cfg.enforce_proxy);
    }
}
