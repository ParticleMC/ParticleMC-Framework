//! 对话框（Dialog）API（框架层，见 `.specs/implement-framework-capabilities/` R8）。
//!
//! 提供 [`DialogOption`] 与 [`DialogTree`] 值类型，以及 [`DialogManager`] `Resource`，
//! 管理 NPC 对话框树的显示 / 清除与选项选择回调。状态经 T20 已实现的
//! [`ShowDialog`]（0x8a）/ [`ClearDialog`]（0x89）clientbound 包同步。
//!
//! 选项选择回调与 serverbound `SelectTrade`（0x32）关联：应用侧收到 `SelectTrade`
//! 后，用选项下标调用 [`on_select`](DialogManager::on_select) 触发对应对话框的
//! 回调。包的发送路径由 T28 系统接线接入。

use std::collections::HashMap;

use uuid::Uuid;

use crate::protocol::packets::play::{ClearDialog, ShowDialog};
use crate::text_component::Component;

/// 对话框单个选项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogOption {
    /// 选项文本。
    pub text: String,
    /// 悬停提示（可选）。
    pub tooltip: Option<String>,
}

/// 对话框树：显示信息 + 选项列表 + 选项选择回调。
///
/// `on_select` 在玩家选中某选项（`SelectTrade` 关联）时被调用，参数为选项下标
/// （`u32`）。回调须 `Send + Sync`（存于 旧 ECS 方案 `Resource`）。
pub struct DialogTree {
    /// 对话框显示名（简化 String）。
    pub display_name: String,
    /// 对话框类型（VarInt）。
    pub dialog_type: i32,
    /// 选项列表。
    pub options: Vec<DialogOption>,
    /// 选项选择回调（无则忽略）。
    pub on_select: Option<Box<dyn Fn(u32) + Send + Sync>>,
}

/// 对话框管理器（旧 ECS 方案 `Resource`）。
///
/// 以 [`Uuid`] 为键登记对话框树。由 [`crate::plugin::McServerPlugin`] 装配或在
/// 应用侧自行插入。
#[derive(Default)]
pub struct DialogManager {
    /// 对话框表（UUID → 对话框树）。
    dialogs: HashMap<Uuid, DialogTree>,
}

impl DialogManager {
    /// 登记一个对话框并分配 UUID，返回其标识。
    ///
    /// 应用侧用返回值构造 [`ShowDialog`] / [`ClearDialog`] 包或作为后续回调句柄。
    pub fn show(&mut self, dialog: DialogTree) -> Uuid {
        let id = Uuid::new_v4();
        self.dialogs.insert(id, dialog);
        id
    }

    /// 清除一个对话框，返回是否存在。
    pub fn clear(&mut self, id: Uuid) -> bool {
        self.dialogs.remove(&id).is_some()
    }

    /// 生成 `ShowDialog` 包；对话框不存在返回 `None`。
    ///
    /// 每个选项编码为 `(选项下标, 文本, 提示)` 动作元组——选项下标即后续
    /// `SelectTrade` / [`on_select`](Self::on_select) 使用的索引。
    pub fn show_packet(&self, id: Uuid) -> Option<ShowDialog> {
        let d = self.dialogs.get(&id)?;
        let actions = d
            .options
            .iter()
            .enumerate()
            .map(|(idx, o)| {
                let action_type = i32::try_from(idx).unwrap_or_default();
                (action_type, o.text.clone(), o.tooltip.clone())
            })
            .collect();
        Some(ShowDialog {
            dialog_id: id,
            display_name: Component::text(&d.display_name),
            dialog_type: d.dialog_type,
            actions,
        })
    }

    /// 生成 `ClearDialog` 包；对话框不存在返回 `None`。
    pub fn clear_packet(&self, id: Uuid) -> Option<ClearDialog> {
        if self.dialogs.contains_key(&id) {
            Some(ClearDialog { dialog_id: id })
        } else {
            None
        }
    }

    /// 触发某对话框的选项选择回调；对话框不存在或未注册回调时忽略。
    pub fn on_select(&self, id: Uuid, option_index: u32) {
        let Some(d) = self.dialogs.get(&id) else {
            return;
        };
        if let Some(cb) = &d.on_select {
            cb(option_index);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn tree(callback: Option<Box<dyn Fn(u32) + Send + Sync>>) -> DialogTree {
        DialogTree {
            display_name: "商人".to_string(),
            dialog_type: 0,
            options: vec![
                DialogOption {
                    text: "购买".to_string(),
                    tooltip: Some("花 5 金币".to_string()),
                },
                DialogOption {
                    text: "离开".to_string(),
                    tooltip: None,
                },
            ],
            on_select: callback,
        }
    }

    #[test]
    fn show_clear_and_packets() {
        let mut mgr = DialogManager::default();
        let id = mgr.show(tree(None));

        let show = mgr.show_packet(id).expect("show_packet 应在");
        assert_eq!(show.dialog_id, id);
        assert_eq!(show.display_name, Component::text("商人"));
        assert_eq!(show.dialog_type, 0);
        assert_eq!(show.actions.len(), 2);
        assert_eq!(
            show.actions[0],
            (0, "购买".to_string(), Some("花 5 金币".to_string()))
        );
        assert_eq!(show.actions[1], (1, "离开".to_string(), None));

        let clear = mgr.clear_packet(id).expect("clear_packet 应在");
        assert_eq!(clear.dialog_id, id);

        // 未登记的 id 包为 None
        assert!(mgr.show_packet(Uuid::nil()).is_none());
        assert!(mgr.clear_packet(Uuid::nil()).is_none());

        assert!(mgr.clear(id));
        assert!(!mgr.clear(id));
        assert!(mgr.show_packet(id).is_none());
    }

    #[test]
    fn on_select_triggers_callback_with_index() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let mut mgr = DialogManager::default();
        let id = mgr.show(tree(Some(Box::new(move |idx| {
            c.fetch_add(idx, Ordering::Relaxed);
        }))));
        mgr.on_select(id, 3);
        assert_eq!(counter.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn on_select_ignored_without_callback_or_dialog() {
        let mut mgr = DialogManager::default();
        let id = mgr.show(tree(None));
        // 无回调：不应 panic
        mgr.on_select(id, 0);
        // 未登记对话框：不应 panic
        mgr.on_select(Uuid::nil(), 0);
    }
}
