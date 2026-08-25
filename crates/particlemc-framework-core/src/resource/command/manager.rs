// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令管理器（框架核心全对齐版，见 `.specs/implement-command-framework/`）。
//!
//! 提供 ParticleMC-Framework `CommandManager` 的完整 API：命令注册/注销/查询、语法驱动的
//! 解析器（`parse_command`）、执行入口（`execute` / `execute_server_command`）、
//! 内置 `help`、未知命令回调，以及结果码（`CommandResult` / `CommandResultType`）。
//!
//! 解析算法忠实 `CommandParser`：trim → 首 token 命令名（大小写不敏感）→ 递归下钻
//! `subcommands` → 遍历叶命令 `syntaxes` 逐一校验参数（顺序 + 可选尾部展开，取匹配
//! 最多者）→ 填 `CommandContext`（含默认值）→ 交由 `execute` 应用条件闸门与执行器。
//!
//! 管理器作为 旧 ECS 方案 `Resource` 注入（见 `plugin.rs` 的 `init_resource::<CommandManager>()`）。

use std::collections::HashMap;

use crate::resource::command::command::{Command, CommandExecutor, CommandSyntax};
use crate::resource::command::context::CommandContext;
use crate::resource::command::sender::{CommandSender, ServerSender};

/// 命令执行结果类型（对齐框架 `CommandResult`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResultType {
    /// 执行成功。
    Success,
    /// 无合法语法匹配（参数数量/类型不符）。
    InvalidSyntax,
    /// 命令条件闸门拒绝（无权限/不可见）。
    Cancelled,
    /// 未知命令（未注册）。
    Unknown,
}

/// 命令执行结果：结果类型 + 原始输入。
///
/// 取代旧版的 `Result<(), CommandError>`，使调用方无需依赖异常即可区分四种结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// 结果类型。
    pub type_: CommandResultType,
    /// 原始命令输入（用于回显/诊断）。
    pub input: String,
}

/// 命令注册错误（名/别名冲突，对齐框架 `IllegalStateException`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError(pub String);

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "命令注册失败：{}", self.0)
    }
}

impl std::error::Error for CommandError {}

/// 未知命令回调：命令未注册时被调用（可经 `emit` 自定义反馈）。
pub trait UnknownCommandCallback: Send + Sync {
    /// 处理未知命令。
    fn apply(&self, sender: &dyn CommandSender, input: &str, emit: &mut dyn FnMut(&str));
}

/// 解析结果（供测试/外部复用，`command`/`syntax` 借用管理器）。
///
/// `command`/`syntax` 为 `Option<&Command>` / `Option<&CommandSyntax>`，生命周期与
/// 来源 `CommandManager` 绑定；`context` 为已解析上下文（拥有所有权）。
pub struct ParseResult<'a> {
    /// 命中的命令（下钻后的叶命令）；未知时为 `None`。
    pub command: Option<&'a Command>,
    /// 命中的语法（叶命令内最匹配者）；无效语法时为 `None`。
    pub syntax: Option<&'a CommandSyntax>,
    /// 已解析上下文（含默认值填充）。
    pub context: CommandContext,
    /// 初步结果类型（`Unknown` / `InvalidSyntax` / `Success`；条件闸门由 `execute` 应用）。
    pub type_: CommandResultType,
}

/// 命令管理器：命令注册表 + 解析器 + 执行入口（旧 ECS 方案 `Resource`）。
///
/// 见 `.specs/implement-command-framework/`。`new()` 内置 `help` 命令；所有持有物
/// 均为 `Send + Sync`，满足 `Resource` 约束。
#[derive(Default)]
pub struct CommandManager {
    /// 已注册命令（主键为命令名原始大小写；查重大小写不敏感）。
    commands: HashMap<String, Command>,
    /// 未知命令回调（可选）。
    unknown_cb: Option<Box<dyn UnknownCommandCallback>>,
}

impl CommandManager {
    /// 构造管理器并注册内置 `help` 命令（见 `list_commands`）。
    pub fn new() -> Self {
        let mut m = CommandManager::default();
        let mut help = Command::new("help", &[]).description("列出所有已注册命令");
        // 列表由 `execute` 直接生成（避免内置执行器回环引用管理器自身）；
        // 此处仅挂一个空执行器以防未命中特殊分支时落入默认路径。
        help.set_default_executor(Box::new(HelpPlaceholderExecutor));
        let _ = m.register(help);
        m
    }

    /// 注册命令；名或别名与已注册命令冲突时返回 `Err(CommandError)`。
    ///
    /// 查重大小写不敏感（对齐框架：重复注册抛 `IllegalStateException`）。
    pub fn register(&mut self, cmd: Command) -> Result<(), CommandError> {
        let new_names = cmd.names();
        for existing in self.commands.values() {
            for en in existing.names() {
                for nn in &new_names {
                    if en.eq_ignore_ascii_case(nn) {
                        return Err(CommandError(format!("命令名或别名冲突：{nn}")));
                    }
                }
            }
        }
        self.commands.insert(cmd.name.clone(), cmd);
        Ok(())
    }

    /// 按名或别名（大小写不敏感）注销命令；返回是否成功移除。
    pub fn unregister(&mut self, name: &str) -> bool {
        let key: Option<String> = self
            .commands
            .keys()
            .find(|k| {
                k.eq_ignore_ascii_case(name)
                    || self
                        .commands
                        .get(*k)
                        .is_some_and(|c| c.names().iter().any(|a| a.eq_ignore_ascii_case(name)))
            })
            .cloned();
        match key {
            Some(k) => self.commands.remove(&k).is_some(),
            None => false,
        }
    }

    /// 按名或别名（大小写不敏感）查询命令。
    pub fn get_command(&self, name: &str) -> Option<&Command> {
        self.commands
            .values()
            .find(|c| c.names().iter().any(|n| n.eq_ignore_ascii_case(name)))
    }

    /// 命令是否存在（大小写不敏感）。
    pub fn command_exists(&self, name: &str) -> bool {
        self.get_command(name).is_some()
    }

    /// 遍历全部已注册命令。
    pub fn get_commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.values()
    }

    /// 设置未知命令回调（链式）。
    pub fn set_unknown_command_callback(
        &mut self,
        cb: Box<dyn UnknownCommandCallback>,
    ) -> &mut Self {
        self.unknown_cb = Some(cb);
        self
    }

    /// 取未知命令回调（若存在）。
    pub fn get_unknown_command_callback(&self) -> Option<&dyn UnknownCommandCallback> {
        self.unknown_cb.as_deref()
    }

    /// 解析命令（纯解析，不含条件与应用副作用）：返回 [`ParseResult`]。
    ///
    /// 流程：trim → 首 token 命令名 → 递归下钻 `subcommands` → 遍历叶命令 `syntaxes`
    /// 取匹配最多者（参数数量优先）。未知命令返回 `Unknown`；无合法语法返回
    /// `InvalidSyntax`；命中返回 `Success`（条件闸门由 `execute` 应用）。
    #[allow(clippy::type_complexity)]
    pub fn parse_command(&self, input: &str, sender: &dyn CommandSender) -> ParseResult<'_> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return ParseResult {
                command: None,
                syntax: None,
                context: CommandContext::new(input, ""),
                type_: CommandResultType::Unknown,
            };
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        let first = tokens[0];
        let command = match self.get_command(first) {
            Some(c) => c,
            None => {
                return ParseResult {
                    command: None,
                    syntax: None,
                    context: CommandContext::new(input, ""),
                    type_: CommandResultType::Unknown,
                };
            }
        };
        // 递归下钻子命令：下一 token 命中子命令名/别名则 shift。
        let mut current: &Command = command;
        let mut idx = 1usize;
        while idx < tokens.len() {
            let next = tokens[idx];
            let found = current
                .subcommands
                .iter()
                .find(|s| s.names().iter().any(|n| n.eq_ignore_ascii_case(next)));
            match found {
                Some(sub) => {
                    current = sub;
                    idx += 1;
                }
                None => break,
            }
        }
        let leaf = current;
        let remaining: &[&str] = &tokens[idx..];

        // 遍历叶语法，取匹配参数最多者（findMostCorrectSyntax）。
        let mut best: Option<(CommandContext, usize, usize, &CommandSyntax)> = None;
        for syn in &leaf.syntaxes {
            if let Some((ctx, mt, ma)) = evaluate_syntax(sender, syn, remaining, input, &leaf.name)
            {
                let better = match &best {
                    None => true,
                    Some((_, bmt, bma, _)) => (ma, mt) > (*bma, *bmt),
                };
                if better {
                    best = Some((ctx, mt, ma, syn));
                }
            }
        }

        match best {
            Some((ctx, _mt, _ma, syn)) => ParseResult {
                command: Some(leaf),
                syntax: Some(syn),
                context: ctx,
                type_: CommandResultType::Success,
            },
            None => ParseResult {
                command: Some(leaf),
                syntax: None,
                context: CommandContext::new(input, &leaf.name),
                type_: CommandResultType::InvalidSyntax,
            },
        }
    }

    /// 执行命令：解析后应用条件闸门与执行器，并经 `emit` 回发反馈。
    ///
    /// 内置 `help` 由本方法直接生成列表（不经过语法解析），以避免执行器回环引用。
    /// 结果码经 [`CommandResult`] 返回：未知 / 无效语法 / 取消 / 成功。
    pub fn execute(
        &self,
        input: &str,
        sender: &dyn CommandSender,
        emit: &mut dyn FnMut(&str),
    ) -> CommandResult {
        let trimmed = input.trim();
        // 内置 help：直接由管理器列出命令（避免执行器回环引用）。
        if let Some(first) = trimmed.split_whitespace().next()
            && first.eq_ignore_ascii_case("help")
        {
            self.list_commands(sender, emit);
            return CommandResult {
                type_: CommandResultType::Success,
                input: input.to_string(),
            };
        }

        let parsed = self.parse_command(input, sender);
        match parsed.type_ {
            CommandResultType::Unknown => {
                emit(&format!("未知命令：{trimmed}"));
                if let Some(cb) = &self.unknown_cb {
                    cb.apply(sender, trimmed, emit);
                }
                CommandResult {
                    type_: CommandResultType::Unknown,
                    input: input.to_string(),
                }
            }
            CommandResultType::InvalidSyntax => {
                // 有默认执行器则调用（即便语法无效），否则回发错误文本。
                match parsed.command {
                    Some(cmd) => {
                        if let Some(de) = &cmd.default_executor {
                            de.execute(sender, &parsed.context, emit);
                        } else {
                            emit(&format!("命令语法无效：{trimmed}"));
                        }
                    }
                    None => emit(&format!("命令语法无效：{trimmed}")),
                }
                CommandResult {
                    type_: CommandResultType::InvalidSyntax,
                    input: input.to_string(),
                }
            }
            CommandResultType::Success => {
                let cmd = match parsed.command {
                    Some(c) => c,
                    None => {
                        return CommandResult {
                            type_: CommandResultType::InvalidSyntax,
                            input: input.to_string(),
                        };
                    }
                };
                // 命令级条件闸门。
                let mut cancelled = false;
                if let Some(cond) = &cmd.condition
                    && !cond.can_use(sender, Some(trimmed))
                {
                    cancelled = true;
                }
                // 语法级条件闸门。
                if !cancelled
                    && let Some(syn) = parsed.syntax
                    && let Some(cond) = &syn.condition
                    && !cond.can_use(sender, Some(trimmed))
                {
                    cancelled = true;
                }
                if cancelled {
                    emit("无权限执行该命令");
                    return CommandResult {
                        type_: CommandResultType::Cancelled,
                        input: input.to_string(),
                    };
                }
                // 执行器：语法级优先，回退命令默认执行器。
                let exec = parsed
                    .syntax
                    .and_then(|s| s.executor.clone())
                    .or_else(|| cmd.default_executor.clone());
                if let Some(e) = exec {
                    e.execute(sender, &parsed.context, emit);
                }
                CommandResult {
                    type_: CommandResultType::Success,
                    input: input.to_string(),
                }
            }
            CommandResultType::Cancelled => CommandResult {
                type_: CommandResultType::Cancelled,
                input: input.to_string(),
            },
        }
    }

    /// 以服务器来源（`ServerSender`）执行命令；反馈不回发网络（`emit` 为空操作）。
    pub fn execute_server_command(&self, input: &str) -> CommandResult {
        let mut emit = |_msg: &str| {};
        self.execute(input, &ServerSender, &mut emit)
    }

    /// 生成命令列表（`help` 用）：按字母序列出 `name - description`，排除自身。
    fn list_commands(&self, _sender: &dyn CommandSender, emit: &mut dyn FnMut(&str)) {
        let mut list: Vec<&Command> = self.get_commands().filter(|c| c.name != "help").collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        for c in list {
            let desc = if c.description.is_empty() {
                "(无描述)"
            } else {
                c.description.as_str()
            };
            emit(&format!("{} - {}", c.name, desc));
        }
    }
}

/// 内置 help 命令的占位执行器（列表由 `CommandManager::execute` 直接生成）。
struct HelpPlaceholderExecutor;

impl CommandExecutor for HelpPlaceholderExecutor {
    fn execute(
        &self,
        _sender: &dyn CommandSender,
        _ctx: &CommandContext,
        _emit: &mut dyn FnMut(&str),
    ) {
        // 实际列表由管理器在 `execute` 中生成，此处不做事。
    }
}

/// 评估单条语法对剩余 token 的匹配度。
///
/// 返回 `(已填上下文, 已消费 token 数, 已匹配参数数)`；不匹配返回 `None`。
/// 规则：顺序匹配；`use_remaining` 参数吞掉剩余全部 token（须为尾部）；可选参数
/// （有默认值）在 token 不足时以默认值填充；必填参数缺失或 token 过剩则失败。
/// 禁用裸 `[i]`：参数顺序访问一律用迭代 + 索引推进。
fn evaluate_syntax(
    sender: &dyn CommandSender,
    syntax: &CommandSyntax,
    tokens: &[&str],
    input: &str,
    command_name: &str,
) -> Option<(CommandContext, usize, usize)> {
    let mut ctx = CommandContext::new(input, command_name);
    let mut matched_tokens = 0usize;
    let mut matched_args = 0usize;
    for (i, arg) in syntax.args.iter().enumerate() {
        if i < tokens.len() {
            let tok = tokens[i];
            if arg.use_remaining() {
                let rest = tokens[i..].join(" ");
                match arg.parse_erased(sender, &rest) {
                    Ok(v) => {
                        ctx.set_arg(arg.id(), v, &rest);
                        matched_tokens = tokens.len();
                        matched_args += 1;
                    }
                    Err(_) => return None,
                }
                // use_remaining 须为尾部参数：消费剩余后立即结束。
                break;
            } else {
                match arg.parse_erased(sender, tok) {
                    Ok(v) => {
                        ctx.set_arg(arg.id(), v, tok);
                        matched_tokens = i + 1;
                        matched_args += 1;
                    }
                    Err(_) => return None,
                }
            }
        } else if arg.is_optional() {
            // token 不足：可选参数以默认值填充（is_optional 即 default.is_some()）。
            let def = arg.default_erased()?;
            ctx.set_arg(arg.id(), def, "");
            matched_args += 1;
        } else {
            // 必填参数缺失：语法无效。
            return None;
        }
    }
    // token 过剩（非 use_remaining 吞掉剩余的情况）视为无效。
    if matched_tokens < tokens.len() {
        return None;
    }
    Some((ctx, matched_tokens, matched_args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::command::argument::ArgumentType;
    use crate::resource::command::condition::CommandCondition;
    use crate::resource::command::sender::ConsoleSender;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    struct FlagExec(Arc<AtomicBool>);
    impl CommandExecutor for FlagExec {
        fn execute(
            &self,
            _s: &dyn CommandSender,
            _c: &CommandContext,
            _emit: &mut dyn FnMut(&str),
        ) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct RecordExec(Arc<Mutex<Vec<String>>>);
    impl CommandExecutor for RecordExec {
        fn execute(&self, _s: &dyn CommandSender, c: &CommandContext, _emit: &mut dyn FnMut(&str)) {
            if let Some(t) = c.get::<String>("target")
                && let Ok(mut g) = self.0.lock()
            {
                g.push(t.clone());
            }
        }
    }

    struct NoopExec;
    impl CommandExecutor for NoopExec {
        fn execute(
            &self,
            _s: &dyn CommandSender,
            _c: &CommandContext,
            _emit: &mut dyn FnMut(&str),
        ) {
        }
    }

    struct DenyCond;
    impl CommandCondition for DenyCond {
        fn can_use(&self, _s: &dyn CommandSender, _c: Option<&str>) -> bool {
            false
        }
    }

    struct RecUnknown(Arc<Mutex<Vec<String>>>);
    impl UnknownCommandCallback for RecUnknown {
        fn apply(&self, _s: &dyn CommandSender, input: &str, _emit: &mut dyn FnMut(&str)) {
            if let Ok(mut g) = self.0.lock() {
                g.push(input.to_string());
            }
        }
    }

    fn console() -> ConsoleSender {
        ConsoleSender
    }

    #[test]
    fn parse_multi_syntax() {
        let mut m = CommandManager::new();
        let mut tp = Command::new("tp", &[]);
        tp.add_syntax(
            Box::new(NoopExec),
            &[Box::new(ArgumentType::Word("target"))],
        );
        tp.add_syntax(
            Box::new(NoopExec),
            &[
                Box::new(ArgumentType::Word("target")),
                Box::new(ArgumentType::Integer("y")),
            ],
        );
        assert!(m.register(tp).is_ok());

        let r = m.parse_command("tp Steve", &console());
        assert_eq!(r.type_, CommandResultType::Success);
        assert_eq!(
            r.context.get::<String>("target"),
            Some(&"Steve".to_string())
        );

        let r = m.parse_command("tp Steve 64", &console());
        assert_eq!(r.type_, CommandResultType::Success);
        assert_eq!(r.context.get::<i32>("y"), Some(&64));
    }

    #[test]
    fn parse_optional_expansion() {
        let mut m = CommandManager::new();
        let mut cmd = Command::new("msg", &[]);
        cmd.add_syntax(
            Box::new(NoopExec),
            &[
                Box::new(ArgumentType::Word("msg")),
                Box::new(ArgumentType::Integer("count").set_default_value(1)),
            ],
        );
        assert!(m.register(cmd).is_ok());

        let r = m.parse_command("msg hi", &console());
        assert_eq!(r.type_, CommandResultType::Success);
        assert_eq!(r.context.get::<String>("msg"), Some(&"hi".to_string()));
        assert_eq!(r.context.get::<i32>("count"), Some(&1));

        let r = m.parse_command("msg hi 3", &console());
        assert_eq!(r.type_, CommandResultType::Success);
        assert_eq!(r.context.get::<i32>("count"), Some(&3));
    }

    #[test]
    fn parse_subcommand() {
        let mut m = CommandManager::new();
        let mut add = Command::new("add", &[]);
        add.add_syntax(Box::new(NoopExec), &[Box::new(ArgumentType::Word("name"))]);
        let mut warp = Command::new("warp", &[]);
        warp.add_subcommand(add);
        assert!(m.register(warp).is_ok());

        let r = m.parse_command("warp add hub", &console());
        assert_eq!(r.type_, CommandResultType::Success);
        assert_eq!(r.context.get::<String>("name"), Some(&"hub".to_string()));
    }

    #[test]
    fn parse_unknown() {
        let m = CommandManager::new();
        let r = m.parse_command("nonexistent", &console());
        assert_eq!(r.type_, CommandResultType::Unknown);
        assert!(r.command.is_none());
    }

    #[test]
    fn parse_invalid_syntax_with_default_executor() {
        let mut m = CommandManager::new();
        let called = Arc::new(AtomicBool::new(false));
        let mut cmd = Command::new("num", &[]);
        cmd.add_syntax(Box::new(NoopExec), &[Box::new(ArgumentType::Integer("n"))]);
        cmd.set_default_executor(Box::new(FlagExec(called.clone())));
        assert!(m.register(cmd).is_ok());

        let r = m.parse_command("num abc", &console());
        assert_eq!(r.type_, CommandResultType::InvalidSyntax);
    }

    #[test]
    fn register_conflict() {
        let mut m = CommandManager::new();
        assert!(m.register(Command::new("tp", &[])).is_ok());
        assert!(m.register(Command::new("tp", &[])).is_err());
        assert!(m.register(Command::new("TP", &[])).is_err());
        assert!(m.register(Command::new("teleport", &["tp"])).is_err());
    }

    #[test]
    fn get_command_case_insensitive() {
        let mut m = CommandManager::new();
        assert!(m.register(Command::new("Teleport", &["tp"])).is_ok());
        assert!(m.get_command("teleport").is_some());
        assert!(m.command_exists("TP"));
    }

    #[test]
    fn unregister_works() {
        let mut m = CommandManager::new();
        assert!(m.register(Command::new("foo", &["f"])).is_ok());
        assert!(m.unregister("FOO"));
        assert!(!m.command_exists("f"));
        assert!(!m.unregister("foo"));
    }

    #[test]
    fn execute_success_runs_executor() {
        let mut m = CommandManager::new();
        let rec = Arc::new(Mutex::new(Vec::new()));
        let mut cmd = Command::new("tp", &[]).description("传送");
        cmd.add_syntax(
            Box::new(RecordExec(rec.clone())),
            &[Box::new(ArgumentType::Word("target"))],
        );
        assert!(m.register(cmd).is_ok());

        let mut received = Vec::new();
        let mut emit = |msg: &str| {
            received.push(msg.to_string());
        };
        let r = m.execute("tp Steve", &console(), &mut emit);
        assert_eq!(r.type_, CommandResultType::Success);
        let n = if let Ok(g) = rec.lock() { g.len() } else { 0 };
        assert_eq!(n, 1);
        assert!(received.is_empty());
    }

    #[test]
    fn execute_invalid_syntax_calls_default() {
        let mut m = CommandManager::new();
        let called = Arc::new(AtomicBool::new(false));
        let mut cmd = Command::new("num", &[]);
        cmd.add_syntax(Box::new(NoopExec), &[Box::new(ArgumentType::Integer("n"))]);
        cmd.set_default_executor(Box::new(FlagExec(called.clone())));
        assert!(m.register(cmd).is_ok());

        let mut received = Vec::new();
        let mut emit = |msg: &str| {
            received.push(msg.to_string());
        };
        let r = m.execute("num abc", &console(), &mut emit);
        assert_eq!(r.type_, CommandResultType::InvalidSyntax);
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn execute_condition_cancelled() {
        let mut m = CommandManager::new();
        let called = Arc::new(AtomicBool::new(false));
        let mut cmd = Command::new("admin", &[]);
        cmd.set_condition(Box::new(DenyCond));
        cmd.add_syntax(
            Box::new(FlagExec(called.clone())),
            &[Box::new(ArgumentType::Word("x"))],
        );
        assert!(m.register(cmd).is_ok());

        let mut received = Vec::new();
        let mut emit = |msg: &str| {
            received.push(msg.to_string());
        };
        let r = m.execute("admin foo", &console(), &mut emit);
        assert_eq!(r.type_, CommandResultType::Cancelled);
        assert!(!called.load(Ordering::SeqCst));
        assert!(received.iter().any(|s| s.contains("权限")));
    }

    #[test]
    fn execute_unknown_calls_callback() {
        let mut m = CommandManager::new();
        let rec = Arc::new(Mutex::new(Vec::new()));
        m.set_unknown_command_callback(Box::new(RecUnknown(rec.clone())));

        let mut received = Vec::new();
        let mut emit = |msg: &str| {
            received.push(msg.to_string());
        };
        let r = m.execute("foobar", &console(), &mut emit);
        assert_eq!(r.type_, CommandResultType::Unknown);
        assert!(received.iter().any(|s| s.contains("foobar")));
        let n = if let Ok(g) = rec.lock() { g.len() } else { 0 };
        assert_eq!(n, 1);
    }

    #[test]
    fn help_lists_commands() {
        let mut m = CommandManager::new();
        let warp = Command::new("warp", &[]).description("传送点");
        let tp = Command::new("tp", &[]).description("传送");
        assert!(m.register(warp).is_ok());
        assert!(m.register(tp).is_ok());

        let mut received = Vec::new();
        let mut emit = |msg: &str| {
            received.push(msg.to_string());
        };
        let r = m.execute("help", &console(), &mut emit);
        assert_eq!(r.type_, CommandResultType::Success);
        let joined = received.join("\n");
        assert!(joined.contains("warp"));
        assert!(joined.contains("tp"));
        // 字母序：tp 在 warp 之前。
        let tp_idx = joined.find("tp");
        let warp_idx = joined.find("warp");
        assert!(tp_idx < warp_idx);
    }

    #[test]
    fn server_command_success() {
        let mut m = CommandManager::new();
        let called = Arc::new(AtomicBool::new(false));
        let mut cmd = Command::new("reload", &[]);
        cmd.add_syntax(Box::new(FlagExec(called.clone())), &[]);
        assert!(m.register(cmd).is_ok());

        let r = m.execute_server_command("reload");
        assert_eq!(r.type_, CommandResultType::Success);
        assert!(called.load(Ordering::SeqCst));
    }
}
