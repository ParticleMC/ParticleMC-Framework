//! 玩家挖掘状态组件。
//!
//! 记录玩家对目标方块的挖掘进度，用于平滑挖掘动画与打断判定。
//! `target_block` 为 `None` 时表示当前未挖掘任何方块。

/// 玩家挖掘状态。
#[derive(Default, Debug, Clone)]
pub struct PlayerDiggingState {
    /// 目标方块的坐标；`None` 表示未挖掘任何方块。
    pub target_block: Option<(i32, i32, i32)>,
    /// 当前挖掘进度，范围 `[0.0, 1.0]`。
    pub progress: f32,
    /// 开始挖掘的时间戳（毫秒）。
    pub start_time_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_no_target_and_zero_progress() {
        let state = PlayerDiggingState::default();
        assert_eq!(state.target_block, None);
        assert_eq!(state.progress, 0.0);
        assert_eq!(state.start_time_ms, 0);
    }

    #[test]
    fn clone_preserves_all_fields() {
        let original = PlayerDiggingState {
            target_block: Some((1, 2, 3)),
            progress: 0.75,
            start_time_ms: 1234,
        };
        let cloned = original.clone();
        assert_eq!(cloned.target_block, Some((1, 2, 3)));
        assert_eq!(cloned.progress, 0.75);
        assert_eq!(cloned.start_time_ms, 1234);
    }

    #[test]
    fn debug_formats_all_fields() {
        let state = PlayerDiggingState {
            target_block: Some((-10, 64, 20)),
            progress: 0.5,
            start_time_ms: 999,
        };
        let formatted = format!("{:?}", state);
        assert!(formatted.contains("target_block"));
        assert!(formatted.contains("Some"));
        assert!(formatted.contains("-10"));
        assert!(formatted.contains("progress"));
        assert!(formatted.contains("start_time_ms"));
    }
}
