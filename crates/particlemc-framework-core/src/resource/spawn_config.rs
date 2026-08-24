//! 出生配置：玩家进入 Play 时的默认出生坐标。
//!
//! 框架不生成任何出生平台（世界内容由应用提供），仅提供可配置的默认出生点，
//! 供登录流程发送 `Position` 包使用。

/// 出生配置。
#[derive(Debug, Clone, Copy)]
pub struct SpawnConfig {
    /// 出生 X 坐标。
    pub x: f64,
    /// 出生 Y 坐标。
    pub y: f64,
    /// 出生 Z 坐标。
    pub z: f64,
    /// 出生偏航角（度）。
    pub yaw: f32,
    /// 出生俯仰角（度）。
    pub pitch: f32,
}

impl Default for SpawnConfig {
    /// 默认出生点 (8, 64, 8)，朝向 0。
    fn default() -> Self {
        Self {
            x: 8.0,
            y: 64.0,
            z: 8.0,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

impl SpawnConfig {
    /// 返回出生点坐标三元组。
    pub fn position(&self) -> (f64, f64, f64) {
        (self.x, self.y, self.z)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_spawn_is_8_64_8() {
        let config = SpawnConfig::default();
        assert_eq!(config.position(), (8.0, 64.0, 8.0));
        assert_eq!(config.yaw, 0.0);
        assert_eq!(config.pitch, 0.0);
    }
}
