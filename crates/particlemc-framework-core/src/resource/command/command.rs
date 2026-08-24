// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令构建器（见 `.specs/implement-command-framework/`）。

use std::sync::Arc;

use crate::resource::command::argument::{AnyArgument, ArgumentCallback};
use crate::resource::command::condition::CommandCondition;
use crate::resource::command::context::CommandContext;
use crate::resource::command::sender::CommandSender;

/// 命令执行器（框架层）。
///
/// 执行器经 `emit` 回发文本反馈（与 inventory 反馈风格一致，系统层统一转
/// `SystemChatPacket`）。所有执行器须 `Send + Sync`（作为 `Resource` 的子组件）。
pub trait CommandExecutor: Send + Sync {
    /// 执行命令；`ctx` 为已解析上下文，`emit` 用于回发反馈文本。
    fn execute(&self, sender: &dyn CommandSender, ctx: &CommandContext, emit: &mut dyn FnMut(&str));
}

/// 一条语法：一组有类型参数 + 绑定执行器（可选）+ 条件（可选）。
///
/// 解析器遍历语法参数依次匹配输入 token；可选尾部参数（`default` 标记）由
/// `add_conditional_syntax` 自动展开为多条语法变体。
pub struct CommandSyntax {
    /// 参数序列（类型擦除）。
    pub args: Vec<Box<dyn AnyArgument>>,
    /// 绑定执行器（语法级优先于命令默认执行器）。
    pub executor: Option<Arc<dyn CommandExecutor>>,
    /// 语法级条件闸门。
    pub condition: Option<Arc<dyn CommandCondition>>,
}

/// 命令：多语法、默认执行器、条件、参数回调、子命令。
///
/// 对齐 Minestom `Command`：命令名 + 别名 + 多语法 + 默认执行器 + 条件 + 子命令。
/// 所有持有物均为 `Send + Sync`，故 `Command` 可作为 `CommandManager`（`Resource`）
/// 的子组件。禁用 `as` 缩窄：参数序列访问全用迭代，无裸 `[i]`。
#[derive(Default)]
pub struct Command {
    /// 命令名（主键，大小写不敏感查重）。
    pub name: String,
    /// 别名（与命令名同样参与查重）。
    pub aliases: Vec<String>,
    /// 命令描述（help 列出用）。
    pub description: String,
    /// 命令级条件闸门。
    pub condition: Option<Arc<dyn CommandCondition>>,
    /// 默认执行器（无语法匹配或不提供语法时调用）。
    pub default_executor: Option<Arc<dyn CommandExecutor>>,
    /// 子命令（递归下钻）。
    pub subcommands: Vec<Command>,
    /// 语法集（含可选展开变体）。
    pub syntaxes: Vec<CommandSyntax>,
}

impl Command {
    /// 构造命令（指定名与别名）。
    pub fn new(name: &str, aliases: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            description: String::new(),
            condition: None,
            default_executor: None,
            subcommands: Vec::new(),
            syntaxes: Vec::new(),
        }
    }

    /// 设置命令描述（链式）。
    pub fn description(mut self, d: &str) -> Self {
        self.description = d.to_string();
        self
    }

    /// 注册一条语法（绑定执行器与参数序列）。
    pub fn add_syntax(
        &mut self,
        exec: Box<dyn CommandExecutor>,
        args: &[Box<dyn AnyArgument>],
    ) -> &mut Self {
        self.add_conditional_syntax(None, exec, args)
    }

    /// 注册一条带条件的语法；自动展开可选尾部参数为多条变体。
    ///
    /// 展开规则：每遇到可选参数（有默认值），先提交当前前缀（不含该可选及其之后
    /// 的参数）作为一条语法变体，再把该可选参数纳入前缀继续，从而覆盖
    /// 「提供 / 省略可选参数」两种输入。
    pub fn add_conditional_syntax(
        &mut self,
        cond: Option<Arc<dyn CommandCondition>>,
        exec: Box<dyn CommandExecutor>,
        args: &[Box<dyn AnyArgument>],
    ) -> &mut Self {
        let exec = Arc::from(exec);
        let base: Vec<Box<dyn AnyArgument>> = args.iter().map(|a| a.clone_box()).collect();
        self.syntaxes.push(CommandSyntax {
            args: base,
            executor: Some(Arc::clone(&exec)),
            condition: cond.clone(),
        });
        let mut prefix: Vec<Box<dyn AnyArgument>> = Vec::new();
        for a in args {
            if a.is_optional() {
                let prefix_cloned: Vec<Box<dyn AnyArgument>> =
                    prefix.iter().map(|x| x.clone_box()).collect();
                self.syntaxes.push(CommandSyntax {
                    args: prefix_cloned,
                    executor: Some(Arc::clone(&exec)),
                    condition: cond.clone(),
                });
                prefix.push(a.clone_box());
            } else {
                prefix.push(a.clone_box());
            }
        }
        self
    }

    /// 设置默认执行器（无语法匹配时回退调用）。
    pub fn set_default_executor(&mut self, exec: Box<dyn CommandExecutor>) -> &mut Self {
        self.default_executor = Some(Arc::from(exec));
        self
    }

    /// 设置命令级条件闸门。
    pub fn set_condition(&mut self, c: Box<dyn CommandCondition>) -> &mut Self {
        self.condition = Some(Arc::from(c));
        self
    }

    /// 为指定 `id` 的参数设置回调（递归应用到子命令）；解析失败时经 `emit` 自定义错误。
    pub fn set_argument_callback(&mut self, cb: Box<dyn ArgumentCallback>, id: &str) -> &mut Self {
        let arc: Arc<dyn ArgumentCallback> = Arc::from(cb);
        self.apply_argument_callback(&arc, id);
        self
    }

    /// 添加子命令（递归下钻）。
    pub fn add_subcommand(&mut self, sub: Command) -> &mut Self {
        self.subcommands.push(sub);
        self
    }

    /// 命令名 + 别名（大小写保留用于展示，查重由管理器大小写不敏感处理）。
    pub fn names(&self) -> Vec<String> {
        let mut v = vec![self.name.clone()];
        v.extend(self.aliases.iter().cloned());
        v
    }

    /// 递归应用参数回调（供 `set_argument_callback` 调用）。
    fn apply_argument_callback(&mut self, arc: &Arc<dyn ArgumentCallback>, id: &str) {
        for syn in &mut self.syntaxes {
            for arg in &mut syn.args {
                if arg.id() == id {
                    arg.set_callback(Arc::clone(arc));
                }
            }
        }
        for sub in &mut self.subcommands {
            sub.apply_argument_callback(arc, id);
        }
    }
}
