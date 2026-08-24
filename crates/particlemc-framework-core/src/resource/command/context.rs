// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令上下文（见 `.specs/implement-command-framework/`）。

use std::any::Any;
use std::collections::HashMap;

/// 可克隆的类型擦除值：在 `dyn Any` 基础上提供克隆能力，供 `CommandContext` 整体克隆
/// （支持 `Group`/`Loop` 参数将子上下文存入父上下文）。
///
/// 标准模式：`CloneAny` 作为本地 trait， blanket 实现于所有 `T: Any + Send + Sync + Clone`，
/// 并手动实现 `Clone for Box<dyn CloneAny + Send + Sync>`（经 `clone_box`）。见
/// `.specs/implement-command-framework/`。
pub trait CloneAny: Any + Send + Sync {
    /// 克隆为同类型擦除值。
    fn clone_boxed(&self) -> Box<dyn CloneAny + Send + Sync>;
}

impl<T: Any + Send + Sync + Clone> CloneAny for T {
    fn clone_boxed(&self) -> Box<dyn CloneAny + Send + Sync> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn CloneAny + Send + Sync> {
    fn clone(&self) -> Self {
        self.clone_boxed()
    }
}

/// 命令执行上下文：按参数 `id` 类型安全取值。
///
/// 解析成功的值以 `Box<dyn CloneAny>` 存入（按参数 id 键），`get::<T>` 经 `as_any`
/// 下转型；缺参返回 `None`（不 panic，契合章程禁 `unwrap`）。`Group`/`Loop` 参数的值
/// 类型为 `CommandContext` / `Vec<CommandContext>`，同样经 `get` 取出。`CommandContext`
/// 自身可克隆（供 `Group`/`Loop` 子上下文回存）。
#[derive(Default, Clone)]
pub struct CommandContext {
    input: String,
    command_name: String,
    args: HashMap<String, Box<dyn CloneAny + Send + Sync>>,
    raw: HashMap<String, String>,
}

impl std::fmt::Debug for CommandContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandContext")
            .field("input", &self.input)
            .field("command_name", &self.command_name)
            .field("args", &self.args.keys().collect::<Vec<_>>())
            .field("raw", &self.raw)
            .finish()
    }
}

impl CommandContext {
    /// 构造上下文（记录原始输入与命中的命令名）。
    pub fn new(input: &str, command_name: &str) -> Self {
        Self {
            input: input.to_string(),
            command_name: command_name.to_string(),
            args: HashMap::new(),
            raw: HashMap::new(),
        }
    }

    /// 存入一个已解析参数值（内部，供解析器调用）。
    pub(crate) fn set_arg(&mut self, id: &str, v: Box<dyn CloneAny + Send + Sync>, raw: &str) {
        self.args.insert(id.to_string(), v);
        self.raw.insert(id.to_string(), raw.to_string());
    }

    /// 类型安全取值；缺参或类型不符返回 `None`（安全，不 panic）。
    ///
    /// 经 trait 上转型（`&dyn CloneAny` → `&dyn Any`）取得类型对象后下转型，
    /// 该上转型能正确保留具体类型 id（见 `.specs/implement-command-framework/`）。
    pub fn get<T: Any + Send + Sync + 'static>(&self, id: &str) -> Option<&T> {
        self.args.get(id).and_then(|b| {
            let any: &dyn Any = &**b;
            any.downcast_ref::<T>()
        })
    }

    /// 类型安全取值；缺参或类型不符时返回默认值 `d`。
    pub fn get_or<T: Any + Send + Sync + 'static + Clone>(&self, id: &str, d: T) -> T {
        self.get::<T>(id).cloned().unwrap_or(d)
    }

    /// 是否存在该参数。
    pub fn has(&self, id: &str) -> bool {
        self.args.contains_key(id)
    }

    /// 原始字符串取值。
    pub fn get_raw(&self, id: &str) -> Option<&str> {
        self.raw.get(id).map(String::as_str)
    }

    /// 完整原始命令输入。
    pub fn input(&self) -> &str {
        &self.input
    }

    /// 命中的命令名。
    pub fn command_name(&self) -> &str {
        &self.command_name
    }
}
