// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 命令参数体系：类型擦除 `AnyArgument` 与有类型 `Argument<T>`，以及 `ArgumentType` 工厂。
//!
//! 变更标识：`complete-partial-framework-capabilities`（T3：新增 10 种参数类型、
//! `ArgumentParserType` 枚举及参数-解析器类型关联，供后续 DeclareCommands 下发）。
//! 见 `.specs/implement-command-framework/`。

use std::any::Any;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use crate::item_stack::ItemStack;
use crate::protocol::nbt::{NbtTag, decode_root};
use crate::resource::command::context::{CloneAny, CommandContext};
use crate::resource::command::error::ArgumentSyntaxException;
use crate::resource::command::sender::CommandSender;
use crate::resource::command::suggestion::{Suggestion, SuggestionCallback};
use crate::resource::registries::item::ItemRegistry;
use crate::text_component::Component as TextComponent;

/// 参数解析闭包类型（类型擦除，见 `.specs/implement-command-framework/`）。
type ArgumentParser<T> =
    Arc<dyn Fn(&dyn CommandSender, &str) -> Result<T, ArgumentSyntaxException> + Send + Sync>;

/// 参数级回调：解析失败时可经由 `emit` 自定义错误。
pub trait ArgumentCallback: Send + Sync {
    /// 处理解析异常：可经 `emit` 回发自定义错误文本。
    fn apply(
        &self,
        _sender: &dyn CommandSender,
        _context: &CommandContext,
        _exception: &ArgumentSyntaxException,
        _emit: &mut dyn FnMut(&str),
    );
}

/// 类型擦除的参数接口：供解析器按 `id` 取参数、解析为 `Box<dyn Any>`。
pub trait AnyArgument: Send + Sync {
    /// 参数 id（取/存上下文的键）。
    fn id(&self) -> &str;
    /// 是否允许含空格（如 String 取剩余全部 token）。
    fn allow_space(&self) -> bool;
    /// 是否取剩余全部输入（use_remaining）。
    fn use_remaining(&self) -> bool;
    /// 是否可选（有默认值）。
    fn is_optional(&self) -> bool;
    /// 解析输入；失败返回 [`ArgumentSyntaxException`]。
    fn parse_erased(
        &self,
        sender: &dyn CommandSender,
        input: &str,
    ) -> Result<Box<dyn CloneAny + Send + Sync>, ArgumentSyntaxException>;
    /// 默认值（可选参数展开用），按 `id` 存入上下文。
    fn default_erased(&self) -> Option<Box<dyn CloneAny + Send + Sync>>;
    /// 补全类型标识符（框架侧占位，恒为 `None`，网络下发超出范围）。
    fn suggestion_type(&self) -> Option<u8>;
    /// 取补全回调（框架侧）。
    fn suggestion_callback(&self) -> Option<&dyn SuggestionCallback>;
    /// 取参数回调（解析失败时自定义错误）。
    fn argument_callback(&self) -> Option<&dyn ArgumentCallback>;
    /// 克隆为 `Box<dyn AnyArgument>`（供可选参数展开构建语法前缀）。
    fn clone_box(&self) -> Box<dyn AnyArgument>;
    /// 设置参数回调（可变，供 `Command::set_argument_callback` 调用）。
    fn set_callback(&mut self, cb: Arc<dyn ArgumentCallback>);
    /// 设置补全回调（可变）。
    fn set_suggestion_callback(&mut self, cb: Arc<dyn SuggestionCallback>);
}

/// 有类型命令参数。
///
/// 解析逻辑由 `parser` 闭包承载（按参数类型定）；`default` 标记可选性并用于
/// 语法展开。禁用 `as` 缩窄：数字解析统一经 `str::parse::<T>()`（自带 TryFrom 语义）。
#[derive(Clone)]
pub struct Argument<T: Clone + Any + Send + Sync + 'static> {
    /// 参数 id。
    pub id: String,
    /// 是否允许空格。
    pub allow_space: bool,
    /// 是否取剩余全部输入。
    pub use_remaining: bool,
    /// 默认值（标记可选）。
    pub default: Option<T>,
    /// `Word.from` 限制词表。
    pub restrictions: Option<Vec<String>>,
    /// 数字 `set_range` 下限。
    pub min: Option<T>,
    /// 数字 `set_range` 上限。
    pub max: Option<T>,
    /// 参数回调。
    pub callback: Option<Arc<dyn ArgumentCallback>>,
    /// 补全回调。
    pub suggestion: Option<Arc<dyn SuggestionCallback>>,
    /// 网络下发的解析器类型（DeclareCommands 用；默认 `None` 表示无对应类型）。
    pub parser_type: Option<ArgumentParserType>,
    /// 解析闭包。
    parser: ArgumentParser<T>,
}

impl<T: Clone + Any + Send + Sync + 'static> Argument<T> {
    /// 以解析闭包构造参数（其余元数据取默认）。
    pub fn with_parser(id: &str, parser: ArgumentParser<T>) -> Self {
        Self {
            id: id.to_string(),
            allow_space: false,
            use_remaining: false,
            default: None,
            restrictions: None,
            min: None,
            max: None,
            callback: None,
            suggestion: None,
            parser_type: None,
            parser,
        }
    }

    /// 设置网络下发的解析器类型（供 DeclareCommands 下发用）。
    pub fn with_parser_type(mut self, parser_type: ArgumentParserType) -> Self {
        self.parser_type = Some(parser_type);
        self
    }

    /// 设置默认值（可选参数标记）；配合 `add_syntax` 尾部展开。
    pub fn set_default_value(mut self, v: T) -> Self {
        self.default = Some(v);
        self
    }

    /// 设置参数回调（解析失败时自定义错误）。
    pub fn set_callback(mut self, cb: Arc<dyn ArgumentCallback>) -> Self {
        self.callback = Some(cb);
        self
    }

    /// 设置补全回调。
    pub fn set_suggestion_callback(mut self, cb: Arc<dyn SuggestionCallback>) -> Self {
        self.suggestion = Some(cb);
        self
    }
}

impl Argument<String> {
    /// `Word.from(restrictions)`：限制词表，不在表中即抛异常。
    pub fn from(mut self, allowed: &[&str]) -> Self {
        let set: Vec<String> = allowed.iter().map(|s| s.to_string()).collect();
        self.restrictions = Some(set.clone());
        self.parser = Arc::new(move |_s, input| {
            if set.iter().any(|a| a == input) {
                Ok(input.to_string())
            } else {
                Err(ArgumentSyntaxException::new(input, 3, "不在允许词表中"))
            }
        });
        self
    }
}

impl Argument<i32> {
    /// 设置整数允许范围（含端点），越界抛异常。
    pub fn set_range(mut self, min: i32, max: i32) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self.parser = Arc::new(move |_s, input| {
            let v = input
                .parse::<i32>()
                .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是整数"))?;
            if v < min || v > max {
                return Err(ArgumentSyntaxException::new(input, 2, "超出允许范围"));
            }
            Ok(v)
        });
        self
    }
}

impl Argument<i64> {
    /// 设置长整数允许范围（含端点），越界抛异常。
    pub fn set_range(mut self, min: i64, max: i64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self.parser = Arc::new(move |_s, input| {
            let v = input
                .parse::<i64>()
                .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是长整数"))?;
            if v < min || v > max {
                return Err(ArgumentSyntaxException::new(input, 2, "超出允许范围"));
            }
            Ok(v)
        });
        self
    }
}

impl Argument<f32> {
    /// 设置浮点允许范围（含端点），越界抛异常。
    pub fn set_range(mut self, min: f32, max: f32) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self.parser = Arc::new(move |_s, input| {
            let v = input
                .parse::<f32>()
                .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是浮点数"))?;
            if v < min || v > max {
                return Err(ArgumentSyntaxException::new(input, 2, "超出允许范围"));
            }
            Ok(v)
        });
        self
    }
}

impl Argument<f64> {
    /// 设置双精度浮点允许范围（含端点），越界抛异常。
    pub fn set_range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self.parser = Arc::new(move |_s, input| {
            let v = input
                .parse::<f64>()
                .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是浮点数"))?;
            if v < min || v > max {
                return Err(ArgumentSyntaxException::new(input, 2, "超出允许范围"));
            }
            Ok(v)
        });
        self
    }
}

impl<T: Clone + Any + Send + Sync + 'static> AnyArgument for Argument<T> {
    fn id(&self) -> &str {
        &self.id
    }
    fn allow_space(&self) -> bool {
        self.allow_space
    }
    fn use_remaining(&self) -> bool {
        self.use_remaining
    }
    fn is_optional(&self) -> bool {
        self.default.is_some()
    }
    fn parse_erased(
        &self,
        sender: &dyn CommandSender,
        input: &str,
    ) -> Result<Box<dyn CloneAny + Send + Sync>, ArgumentSyntaxException> {
        let v = (self.parser)(sender, input)?;
        Ok(Box::new(v))
    }
    fn default_erased(&self) -> Option<Box<dyn CloneAny + Send + Sync>> {
        self.default
            .clone()
            .map(|d| Box::new(d) as Box<dyn CloneAny + Send + Sync>)
    }
    fn suggestion_type(&self) -> Option<u8> {
        None
    }
    fn suggestion_callback(&self) -> Option<&dyn SuggestionCallback> {
        self.suggestion.as_deref()
    }
    fn argument_callback(&self) -> Option<&dyn ArgumentCallback> {
        self.callback.as_deref()
    }
    fn clone_box(&self) -> Box<dyn AnyArgument> {
        Box::new(self.clone())
    }
    fn set_callback(&mut self, cb: Arc<dyn ArgumentCallback>) {
        self.callback = Some(cb);
    }
    fn set_suggestion_callback(&mut self, cb: Arc<dyn SuggestionCallback>) {
        self.suggestion = Some(cb);
    }
}

/// 命令参数解析器类型，id 对齐 Java `ArgumentParserType` 的序位（权威源：
/// `java/.../autogenerated/.../ArgumentParserType.java`）。
///
/// 前三个变体 `ROOT`/`LITERAL`/`ARGUMENT` 为 DeclareCommands 的节点类型标记
/// （brigadier 节点 flag），并非真实解析器，id 取保留负值，不占 Java 序位；
/// 其余变体从 `BOOL`(0) 到 `UUID`(56) 与 Java 序位一一对应。
///
/// 变体名刻意沿用 Java 枚举常量命名（大写蛇形，含下划线），故放宽
/// `non_camel_case_types`，与权威源逐名对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ArgumentParserType {
    /// 节点类型：根节点（DeclareCommands 用，非解析器）。
    ROOT,
    /// 节点类型：字面量节点（DeclareCommands 用，非解析器）。
    LITERAL,
    /// 节点类型：参数节点（DeclareCommands 用，非解析器）。
    ARGUMENT,
    /// `brigadier:bool`
    BOOL,
    /// `brigadier:float`
    FLOAT,
    /// `brigadier:double`
    DOUBLE,
    /// `brigadier:integer`
    INTEGER,
    /// `brigadier:long`
    LONG,
    /// `brigadier:string`
    STRING,
    /// `entity`
    ENTITY,
    /// `game_profile`
    GAME_PROFILE,
    /// `block_pos`
    BLOCK_POS,
    /// `column_pos`
    COLUMN_POS,
    /// `vec3`
    VEC3,
    /// `vec2`
    VEC2,
    /// `block_state`
    BLOCK_STATE,
    /// `block_predicate`
    BLOCK_PREDICATE,
    /// `item_stack`
    ITEM_STACK,
    /// `item_predicate`
    ITEM_PREDICATE,
    /// `color`
    COLOR,
    /// `hex_color`
    HEX_COLOR,
    /// `component`
    COMPONENT,
    /// `style`
    STYLE,
    /// `message`
    MESSAGE,
    /// `nbt_compound_tag`
    NBT_COMPOUND_TAG,
    /// `nbt_tag`
    NBT_TAG,
    /// `nbt_path`
    NBT_PATH,
    /// `objective`
    OBJECTIVE,
    /// `objective_criteria`
    OBJECTIVE_CRITERIA,
    /// `operation`
    OPERATION,
    /// `particle`
    PARTICLE,
    /// `angle`
    ANGLE,
    /// `rotation`
    ROTATION,
    /// `scoreboard_slot`
    SCOREBOARD_SLOT,
    /// `score_holder`
    SCORE_HOLDER,
    /// `swizzle`
    SWIZZLE,
    /// `team`
    TEAM,
    /// `item_slot`
    ITEM_SLOT,
    /// `item_slots`
    ITEM_SLOTS,
    /// `resource_location`
    RESOURCE_LOCATION,
    /// `function`
    FUNCTION,
    /// `entity_anchor`
    ENTITY_ANCHOR,
    /// `int_range`
    INT_RANGE,
    /// `float_range`
    FLOAT_RANGE,
    /// `dimension`
    DIMENSION,
    /// `gamemode`
    GAMEMODE,
    /// `time`
    TIME,
    /// `resource_or_tag`
    RESOURCE_OR_TAG,
    /// `resource_or_tag_key`
    RESOURCE_OR_TAG_KEY,
    /// `resource`
    RESOURCE,
    /// `resource_key`
    RESOURCE_KEY,
    /// `resource_selector`
    RESOURCE_SELECTOR,
    /// `template_mirror`
    TEMPLATE_MIRROR,
    /// `template_rotation`
    TEMPLATE_ROTATION,
    /// `heightmap`
    HEIGHTMAP,
    /// `loot_table`
    LOOT_TABLE,
    /// `loot_predicate`
    LOOT_PREDICATE,
    /// `loot_modifier`
    LOOT_MODIFIER,
    /// `dialog`
    DIALOG,
    /// `uuid`
    UUID,
}

impl ArgumentParserType {
    /// 解析器 id：Java 序位（`BOOL`=0 … `UUID`=56）；节点标记为保留负值。
    pub fn id(&self) -> i32 {
        match self {
            ArgumentParserType::ROOT => -3,
            ArgumentParserType::LITERAL => -2,
            ArgumentParserType::ARGUMENT => -1,
            ArgumentParserType::BOOL => 0,
            ArgumentParserType::FLOAT => 1,
            ArgumentParserType::DOUBLE => 2,
            ArgumentParserType::INTEGER => 3,
            ArgumentParserType::LONG => 4,
            ArgumentParserType::STRING => 5,
            ArgumentParserType::ENTITY => 6,
            ArgumentParserType::GAME_PROFILE => 7,
            ArgumentParserType::BLOCK_POS => 8,
            ArgumentParserType::COLUMN_POS => 9,
            ArgumentParserType::VEC3 => 10,
            ArgumentParserType::VEC2 => 11,
            ArgumentParserType::BLOCK_STATE => 12,
            ArgumentParserType::BLOCK_PREDICATE => 13,
            ArgumentParserType::ITEM_STACK => 14,
            ArgumentParserType::ITEM_PREDICATE => 15,
            ArgumentParserType::COLOR => 16,
            ArgumentParserType::HEX_COLOR => 17,
            ArgumentParserType::COMPONENT => 18,
            ArgumentParserType::STYLE => 19,
            ArgumentParserType::MESSAGE => 20,
            ArgumentParserType::NBT_COMPOUND_TAG => 21,
            ArgumentParserType::NBT_TAG => 22,
            ArgumentParserType::NBT_PATH => 23,
            ArgumentParserType::OBJECTIVE => 24,
            ArgumentParserType::OBJECTIVE_CRITERIA => 25,
            ArgumentParserType::OPERATION => 26,
            ArgumentParserType::PARTICLE => 27,
            ArgumentParserType::ANGLE => 28,
            ArgumentParserType::ROTATION => 29,
            ArgumentParserType::SCOREBOARD_SLOT => 30,
            ArgumentParserType::SCORE_HOLDER => 31,
            ArgumentParserType::SWIZZLE => 32,
            ArgumentParserType::TEAM => 33,
            ArgumentParserType::ITEM_SLOT => 34,
            ArgumentParserType::ITEM_SLOTS => 35,
            ArgumentParserType::RESOURCE_LOCATION => 36,
            ArgumentParserType::FUNCTION => 37,
            ArgumentParserType::ENTITY_ANCHOR => 38,
            ArgumentParserType::INT_RANGE => 39,
            ArgumentParserType::FLOAT_RANGE => 40,
            ArgumentParserType::DIMENSION => 41,
            ArgumentParserType::GAMEMODE => 42,
            ArgumentParserType::TIME => 43,
            ArgumentParserType::RESOURCE_OR_TAG => 44,
            ArgumentParserType::RESOURCE_OR_TAG_KEY => 45,
            ArgumentParserType::RESOURCE => 46,
            ArgumentParserType::RESOURCE_KEY => 47,
            ArgumentParserType::RESOURCE_SELECTOR => 48,
            ArgumentParserType::TEMPLATE_MIRROR => 49,
            ArgumentParserType::TEMPLATE_ROTATION => 50,
            ArgumentParserType::HEIGHTMAP => 51,
            ArgumentParserType::LOOT_TABLE => 52,
            ArgumentParserType::LOOT_PREDICATE => 53,
            ArgumentParserType::LOOT_MODIFIER => 54,
            ArgumentParserType::DIALOG => 55,
            ArgumentParserType::UUID => 56,
        }
    }

    /// 由 id 反查解析器类型；未知 id 返回 `None`。
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            -3 => Some(ArgumentParserType::ROOT),
            -2 => Some(ArgumentParserType::LITERAL),
            -1 => Some(ArgumentParserType::ARGUMENT),
            0 => Some(ArgumentParserType::BOOL),
            1 => Some(ArgumentParserType::FLOAT),
            2 => Some(ArgumentParserType::DOUBLE),
            3 => Some(ArgumentParserType::INTEGER),
            4 => Some(ArgumentParserType::LONG),
            5 => Some(ArgumentParserType::STRING),
            6 => Some(ArgumentParserType::ENTITY),
            7 => Some(ArgumentParserType::GAME_PROFILE),
            8 => Some(ArgumentParserType::BLOCK_POS),
            9 => Some(ArgumentParserType::COLUMN_POS),
            10 => Some(ArgumentParserType::VEC3),
            11 => Some(ArgumentParserType::VEC2),
            12 => Some(ArgumentParserType::BLOCK_STATE),
            13 => Some(ArgumentParserType::BLOCK_PREDICATE),
            14 => Some(ArgumentParserType::ITEM_STACK),
            15 => Some(ArgumentParserType::ITEM_PREDICATE),
            16 => Some(ArgumentParserType::COLOR),
            17 => Some(ArgumentParserType::HEX_COLOR),
            18 => Some(ArgumentParserType::COMPONENT),
            19 => Some(ArgumentParserType::STYLE),
            20 => Some(ArgumentParserType::MESSAGE),
            21 => Some(ArgumentParserType::NBT_COMPOUND_TAG),
            22 => Some(ArgumentParserType::NBT_TAG),
            23 => Some(ArgumentParserType::NBT_PATH),
            24 => Some(ArgumentParserType::OBJECTIVE),
            25 => Some(ArgumentParserType::OBJECTIVE_CRITERIA),
            26 => Some(ArgumentParserType::OPERATION),
            27 => Some(ArgumentParserType::PARTICLE),
            28 => Some(ArgumentParserType::ANGLE),
            29 => Some(ArgumentParserType::ROTATION),
            30 => Some(ArgumentParserType::SCOREBOARD_SLOT),
            31 => Some(ArgumentParserType::SCORE_HOLDER),
            32 => Some(ArgumentParserType::SWIZZLE),
            33 => Some(ArgumentParserType::TEAM),
            34 => Some(ArgumentParserType::ITEM_SLOT),
            35 => Some(ArgumentParserType::ITEM_SLOTS),
            36 => Some(ArgumentParserType::RESOURCE_LOCATION),
            37 => Some(ArgumentParserType::FUNCTION),
            38 => Some(ArgumentParserType::ENTITY_ANCHOR),
            39 => Some(ArgumentParserType::INT_RANGE),
            40 => Some(ArgumentParserType::FLOAT_RANGE),
            41 => Some(ArgumentParserType::DIMENSION),
            42 => Some(ArgumentParserType::GAMEMODE),
            43 => Some(ArgumentParserType::TIME),
            44 => Some(ArgumentParserType::RESOURCE_OR_TAG),
            45 => Some(ArgumentParserType::RESOURCE_OR_TAG_KEY),
            46 => Some(ArgumentParserType::RESOURCE),
            47 => Some(ArgumentParserType::RESOURCE_KEY),
            48 => Some(ArgumentParserType::RESOURCE_SELECTOR),
            49 => Some(ArgumentParserType::TEMPLATE_MIRROR),
            50 => Some(ArgumentParserType::TEMPLATE_ROTATION),
            51 => Some(ArgumentParserType::HEIGHTMAP),
            52 => Some(ArgumentParserType::LOOT_TABLE),
            53 => Some(ArgumentParserType::LOOT_PREDICATE),
            54 => Some(ArgumentParserType::LOOT_MODIFIER),
            55 => Some(ArgumentParserType::DIALOG),
            56 => Some(ArgumentParserType::UUID),
            _ => None,
        }
    }

    /// 解析器标识串（与 Java `ArgumentParserType` 构造参数一致）。
    pub fn key(&self) -> &'static str {
        match self {
            ArgumentParserType::ROOT => "root",
            ArgumentParserType::LITERAL => "literal",
            ArgumentParserType::ARGUMENT => "argument",
            ArgumentParserType::BOOL => "brigadier:bool",
            ArgumentParserType::FLOAT => "brigadier:float",
            ArgumentParserType::DOUBLE => "brigadier:double",
            ArgumentParserType::INTEGER => "brigadier:integer",
            ArgumentParserType::LONG => "brigadier:long",
            ArgumentParserType::STRING => "brigadier:string",
            ArgumentParserType::ENTITY => "entity",
            ArgumentParserType::GAME_PROFILE => "game_profile",
            ArgumentParserType::BLOCK_POS => "block_pos",
            ArgumentParserType::COLUMN_POS => "column_pos",
            ArgumentParserType::VEC3 => "vec3",
            ArgumentParserType::VEC2 => "vec2",
            ArgumentParserType::BLOCK_STATE => "block_state",
            ArgumentParserType::BLOCK_PREDICATE => "block_predicate",
            ArgumentParserType::ITEM_STACK => "item_stack",
            ArgumentParserType::ITEM_PREDICATE => "item_predicate",
            ArgumentParserType::COLOR => "color",
            ArgumentParserType::HEX_COLOR => "hex_color",
            ArgumentParserType::COMPONENT => "component",
            ArgumentParserType::STYLE => "style",
            ArgumentParserType::MESSAGE => "message",
            ArgumentParserType::NBT_COMPOUND_TAG => "nbt_compound_tag",
            ArgumentParserType::NBT_TAG => "nbt_tag",
            ArgumentParserType::NBT_PATH => "nbt_path",
            ArgumentParserType::OBJECTIVE => "objective",
            ArgumentParserType::OBJECTIVE_CRITERIA => "objective_criteria",
            ArgumentParserType::OPERATION => "operation",
            ArgumentParserType::PARTICLE => "particle",
            ArgumentParserType::ANGLE => "angle",
            ArgumentParserType::ROTATION => "rotation",
            ArgumentParserType::SCOREBOARD_SLOT => "scoreboard_slot",
            ArgumentParserType::SCORE_HOLDER => "score_holder",
            ArgumentParserType::SWIZZLE => "swizzle",
            ArgumentParserType::TEAM => "team",
            ArgumentParserType::ITEM_SLOT => "item_slot",
            ArgumentParserType::ITEM_SLOTS => "item_slots",
            ArgumentParserType::RESOURCE_LOCATION => "resource_location",
            ArgumentParserType::FUNCTION => "function",
            ArgumentParserType::ENTITY_ANCHOR => "entity_anchor",
            ArgumentParserType::INT_RANGE => "int_range",
            ArgumentParserType::FLOAT_RANGE => "float_range",
            ArgumentParserType::DIMENSION => "dimension",
            ArgumentParserType::GAMEMODE => "gamemode",
            ArgumentParserType::TIME => "time",
            ArgumentParserType::RESOURCE_OR_TAG => "resource_or_tag",
            ArgumentParserType::RESOURCE_OR_TAG_KEY => "resource_or_tag_key",
            ArgumentParserType::RESOURCE => "resource",
            ArgumentParserType::RESOURCE_KEY => "resource_key",
            ArgumentParserType::RESOURCE_SELECTOR => "resource_selector",
            ArgumentParserType::TEMPLATE_MIRROR => "template_mirror",
            ArgumentParserType::TEMPLATE_ROTATION => "template_rotation",
            ArgumentParserType::HEIGHTMAP => "heightmap",
            ArgumentParserType::LOOT_TABLE => "loot_table",
            ArgumentParserType::LOOT_PREDICATE => "loot_predicate",
            ArgumentParserType::LOOT_MODIFIER => "loot_modifier",
            ArgumentParserType::DIALOG => "dialog",
            ArgumentParserType::UUID => "uuid",
        }
    }
}

/// 实体选择器目标类型（v1 子集，见 [`EntitySelector`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntitySelectorType {
    /// `@p`：最近玩家。
    NearestPlayer,
    /// `@a`：全部玩家。
    AllPlayers,
    /// `@e`：全部实体。
    AllEntities,
    /// 裸玩家名（无 `@` 前缀）。
    ByUsername,
}

/// 实体选择器（v1 简化：仅 `@p`/`@a`/`@e` 与 `name=` 过滤子集）。
///
/// 对齐 Java `ArgumentEntity` 的 `EntityFinder` 语义；v1 不解析
/// `type=`/`limit=` 等其余过滤项。`name` 为 `[name=...]` 提供的过滤名
/// （忽略 `!` 排除前缀的语义，仅记录目标名）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitySelector {
    /// 目标选择类型。
    pub target_type: EntitySelectorType,
    /// 名称过滤（`[name=...]`；无过滤时为 `None`）。
    pub name: Option<String>,
}

/// 三维相对/绝对坐标（`~`/`^` 相对语义，对齐 Java `ArgumentRelativeVec3`）。
///
/// `values` 为各分量数值（`~`/`^` 无数字时取 0.0）；`relative` 标记该分量
/// 是否为相对坐标（`~`/`^` 前缀）。v1 不区分世界相对（`~`）与本地相对（`^`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelativeVec3 {
    /// 各分量数值。
    pub values: [f64; 3],
    /// 各分量是否为相对坐标。
    pub relative: [bool; 3],
}

/// 将一组子参数从 token 序列解析进子上下文（供 `Group`/`Loop` 复用）。
///
/// 返回 `(子上下文, 已消费 token 数)`；消费数用于 `Loop` 推进。禁用裸 `[i]`：
/// 顺序参数访问一律用迭代 + 索引推进。
fn parse_subargs(
    sender: &dyn CommandSender,
    tokens: &[&str],
    args: &[Box<dyn AnyArgument>],
) -> Result<(CommandContext, usize), ArgumentSyntaxException> {
    let mut ctx = CommandContext::new("", "");
    let mut idx = 0usize;
    for arg in args {
        if idx >= tokens.len() {
            if let Some(def) = arg.default_erased() {
                ctx.set_arg(arg.id(), def, "");
                continue;
            }
            return Err(ArgumentSyntaxException::new("", 1, "子参数不足"));
        }
        if arg.use_remaining() {
            let rest = tokens[idx..].join(" ");
            let v = arg.parse_erased(sender, &rest)?;
            ctx.set_arg(arg.id(), v, &rest);
            idx = tokens.len();
        } else {
            let tok = tokens[idx];
            let v = arg.parse_erased(sender, tok)?;
            ctx.set_arg(arg.id(), v, tok);
            idx += 1;
        }
    }
    Ok((ctx, idx))
}

/// 有类型参数工厂（对齐框架 `ArgumentType`）。
///
/// 工厂函数沿用框架 `ArgumentType` 的 PascalCase 命名（如 `Word`/`Integer`/
/// `Boolean`/`Enum`/`Literal`/`StringArray`/`Group`/`Loop`/`Command`），故对模块与
/// 函数放宽 `non_snake_case`（见 `.specs/implement-command-framework/`）。
#[allow(non_snake_case)]
pub mod ArgumentType {
    use super::*;

    /// 单 token 词（默认不限制；`.from(restrictions)` 限制词表）。
    pub fn Word(id: &str) -> Argument<String> {
        Argument::with_parser(id, Arc::new(|_s, input| Ok(input.to_string())))
    }

    /// 允许空格（use_remaining）的字符串。
    pub fn String(id: &str) -> Argument<String> {
        let mut a = Argument::with_parser(id, Arc::new(|_s, input| Ok(input.to_string())))
            .with_parser_type(ArgumentParserType::STRING);
        a.allow_space = true;
        a.use_remaining = true;
        a
    }

    /// 32 位整数；`.set_range(min,max)` 限制范围。
    pub fn Integer(id: &str) -> Argument<i32> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                input
                    .parse::<i32>()
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是整数"))
            }),
        )
        .with_parser_type(ArgumentParserType::INTEGER)
    }

    /// 64 位整数；`.set_range(min,max)` 限制范围。
    pub fn Long(id: &str) -> Argument<i64> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                input
                    .parse::<i64>()
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是长整数"))
            }),
        )
        .with_parser_type(ArgumentParserType::LONG)
    }

    /// 32 位浮点；`.set_range(min,max)` 限制范围。
    pub fn Float(id: &str) -> Argument<f32> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                input
                    .parse::<f32>()
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是浮点数"))
            }),
        )
        .with_parser_type(ArgumentParserType::FLOAT)
    }

    /// 64 位浮点；`.set_range(min,max)` 限制范围。
    pub fn Double(id: &str) -> Argument<f64> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                input
                    .parse::<f64>()
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "不是浮点数"))
            }),
        )
        .with_parser_type(ArgumentParserType::DOUBLE)
    }

    /// 布尔（仅接受 `true`/`false`，大小写不敏感）。
    pub fn Boolean(id: &str) -> Argument<bool> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| match input.to_ascii_lowercase().as_str() {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(ArgumentSyntaxException::new(
                    input,
                    4,
                    "不是布尔值(true/false)",
                )),
            }),
        )
        .with_parser_type(ArgumentParserType::BOOL)
    }

    /// 枚举（经 `FromStr` 解析；`_vals` 为允许值列表，供调用方参考）。
    pub fn Enum<E: Display + FromStr + Clone + Send + Sync + 'static>(
        id: &str,
        _vals: &[E],
    ) -> Argument<E>
    where
        E::Err: std::fmt::Display,
    {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                E::from_str(input).map_err(|e| {
                    ArgumentSyntaxException::new(input, 5, &format!("枚举解析失败：{e}"))
                })
            }),
        )
    }

    /// 字面量：输入须精确等于 `id`。
    pub fn Literal(id: &str) -> Argument<String> {
        let lit = id.to_string();
        Argument::with_parser(
            id,
            Arc::new(move |_s, input| {
                if input == lit {
                    Ok(input.to_string())
                } else {
                    Err(ArgumentSyntaxException::new(input, 6, "字面量不匹配"))
                }
            }),
        )
    }

    /// 字符串数组（按空白切分为 `Vec<String>`，use_remaining）。
    pub fn StringArray(id: &str) -> Argument<Vec<String>> {
        let mut a = Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                Ok(input
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>())
            }),
        );
        a.allow_space = true;
        a.use_remaining = true;
        a
    }

    /// 复合参数：解析为子 [`CommandContext`]（use_remaining）。
    pub fn Group(id: &str, args: &[Box<dyn AnyArgument>]) -> Argument<CommandContext> {
        let subargs: Vec<Box<dyn AnyArgument>> = args.iter().map(|a| a.clone_box()).collect();
        let mut a = Argument::with_parser(
            id,
            Arc::new(move |s, input| {
                let tokens: Vec<&str> = input.split_whitespace().collect();
                let (ctx, _consumed) = parse_subargs(s, &tokens, &subargs)?;
                Ok(ctx)
            }),
        );
        a.allow_space = true;
        a.use_remaining = true;
        a
    }

    /// 重复参数（1..n 组）：解析为 `Vec<CommandContext>`（use_remaining）。
    pub fn Loop(id: &str, args: &[Box<dyn AnyArgument>]) -> Argument<Vec<CommandContext>> {
        let subargs: Vec<Box<dyn AnyArgument>> = args.iter().map(|a| a.clone_box()).collect();
        let mut a = Argument::with_parser(
            id,
            Arc::new(move |s, input| {
                let tokens: Vec<&str> = input.split_whitespace().collect();
                if tokens.is_empty() {
                    return Err(ArgumentSyntaxException::new("", 1, "Loop 至少需要一组参数"));
                }
                let mut idx = 0usize;
                let mut collected = Vec::new();
                while idx < tokens.len() {
                    let (ctx, consumed) = parse_subargs(s, &tokens[idx..], &subargs)?;
                    if consumed == 0 {
                        break;
                    }
                    idx += consumed;
                    collected.push(ctx);
                }
                Ok(collected)
            }),
        );
        a.allow_space = true;
        a.use_remaining = true;
        a
    }

    /// 子命令桥（首 token = 子命令名）；实际下钻由管理器 `resolve_leaf` 处理。
    pub fn Command(id: &str) -> Argument<String> {
        Argument::with_parser(id, Arc::new(|_s, input| Ok(input.to_string())))
    }

    // ── 以下为 T3 新增（变更标识：complete-partial-framework-capabilities）──

    /// 实体选择器（v1 子集：`@p`/`@a`/`@e` 与 `[name=...]` 过滤；裸玩家名）。
    pub fn Entity(id: &str) -> Argument<EntitySelector> {
        Argument::with_parser(id, Arc::new(|_s, input| parse_entity_selector(input)))
            .with_parser_type(ArgumentParserType::ENTITY)
    }

    /// 玩家名（1..=16 位字母数字或下划线）。
    pub fn Player(id: &str) -> Argument<String> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                if input.is_empty() {
                    return Err(ArgumentSyntaxException::new(input, 1, "玩家名不能为空"));
                }
                let valid = input.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                let len = input.chars().count();
                if !valid || len > 16 {
                    return Err(ArgumentSyntaxException::new(
                        input,
                        1,
                        "玩家名须为 1..=16 位字母数字或下划线",
                    ));
                }
                Ok(input.to_string())
            }),
        )
        .with_parser_type(ArgumentParserType::GAME_PROFILE)
    }

    /// 方块坐标：`x y z` 三个整数分量，`~` 视为 0（v1 简化）。
    pub fn BlockPos(id: &str) -> Argument<[i32; 3]> {
        Argument::with_parser(id, Arc::new(|_s, input| parse_block_pos(input)))
            .with_parser_type(ArgumentParserType::BLOCK_POS)
    }

    /// 物品栈：按 `ItemRegistry` 查物品名（无命名空间时补 `minecraft:` 前缀），
    /// 返回 amount=1 的 [`ItemStack`]；未提供注册表或物品未知时报错。
    pub fn ItemStack(id: &str, registry: Option<ItemRegistry>) -> Argument<ItemStack> {
        Argument::with_parser(
            id,
            Arc::new(move |_s, input| {
                let Some(reg) = registry.as_ref() else {
                    return Err(ArgumentSyntaxException::new(
                        input,
                        1,
                        "未提供物品注册表，无法解析物品",
                    ));
                };
                let material = lookup_material(reg, input)
                    .ok_or_else(|| ArgumentSyntaxException::new(input, 3, "未知物品"))?;
                Ok(ItemStack::new(material, 1))
            }),
        )
        .with_parser_type(ArgumentParserType::ITEM_STACK)
    }

    /// NBT 标签：复用 `protocol::nbt` 的二进制解码入口（`decode_root`），
    /// 将输入字符串的字节视作二进制 NBT 解析。
    pub fn NbtTag(id: &str) -> Argument<NbtTag> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                decode_root(input.as_bytes())
                    .map(|(_root_name, tag)| tag)
                    .map_err(|_| {
                        ArgumentSyntaxException::new(input, 1, "无效的 NBT（需为二进制 NBT 编码）")
                    })
            }),
        )
        .with_parser_type(ArgumentParserType::NBT_TAG)
    }

    /// 文本组件：复用 `protocol::nbt` 的二进制解码入口（`decode_root`）后经
    /// `text_component::Component::from_nbt` 还原；输入为二进制 NBT 编码。
    pub fn Component(id: &str) -> Argument<TextComponent> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                let (_root_name, tag) = decode_root(input.as_bytes())
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "无效的文本组件 NBT"))?;
                TextComponent::from_nbt(&tag)
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "NBT 无法解析为文本组件"))
            }),
        )
        .with_parser_type(ArgumentParserType::COMPONENT)
    }

    /// 资源定位符 `namespace:path`；无冒号时补 `minecraft:` 前缀，校验字符集。
    pub fn ResourceLocation(id: &str) -> Argument<String> {
        Argument::with_parser(id, Arc::new(|_s, input| parse_resource_location(input)))
            .with_parser_type(ArgumentParserType::RESOURCE_LOCATION)
    }

    /// UUID（经 `uuid` crate 的 `Uuid::parse_str`）。
    pub fn Uuid(id: &str) -> Argument<Uuid> {
        Argument::with_parser(
            id,
            Arc::new(|_s, input| {
                input
                    .parse::<Uuid>()
                    .map_err(|_| ArgumentSyntaxException::new(input, 1, "无效的 UUID"))
            }),
        )
        .with_parser_type(ArgumentParserType::UUID)
    }

    /// 时间（tick 数）：支持 `t`/`s`/`d` 后缀（1s=20tick，1d=24000tick）
    /// 与无后缀（默认 tick）；v1 返回 i32 tick 数。
    pub fn Time(id: &str) -> Argument<i32> {
        Argument::with_parser(id, Arc::new(|_s, input| parse_time(input)))
            .with_parser_type(ArgumentParserType::TIME)
    }

    /// 三维相对/绝对坐标（`~`/`^` 支持，v1 仅记录相对标志）。
    pub fn RelativeVec3(id: &str) -> Argument<RelativeVec3> {
        Argument::with_parser(id, Arc::new(|_s, input| parse_relative_vec3(input)))
            .with_parser_type(ArgumentParserType::VEC3)
    }
}

/// 物品注册表中按名称查物品 id（无命名空间时补 `minecraft:` 前缀）。
fn lookup_material(reg: &ItemRegistry, input: &str) -> Option<u32> {
    if let Some(id) = reg.0.get_id(input) {
        return Some(id);
    }
    if !input.contains(':') {
        let full = format!("minecraft:{input}");
        return reg.0.get_id(&full);
    }
    None
}

/// 解析实体选择器（v1 子集）。
fn parse_entity_selector(input: &str) -> Result<EntitySelector, ArgumentSyntaxException> {
    let mut chars = input.chars();
    let first = chars.next();
    if first == Some('@') {
        let target = match chars.next() {
            Some('p') => EntitySelectorType::NearestPlayer,
            Some('a') => EntitySelectorType::AllPlayers,
            Some('e') => EntitySelectorType::AllEntities,
            _ => {
                return Err(ArgumentSyntaxException::new(
                    input,
                    1,
                    "不支持的实体选择器变量（v1 支持 @p/@a/@e）",
                ));
            }
        };
        let rest: String = chars.collect();
        let name = if rest.is_empty() {
            None
        } else {
            let inner = rest
                .strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .ok_or_else(|| {
                    ArgumentSyntaxException::new(input, 1, "选择器结构需以 [..] 包裹")
                })?;
            parse_name_filter(input, inner)?
        };
        Ok(EntitySelector {
            target_type: target,
            name,
        })
    } else {
        // 裸玩家名（对齐 Java 的 username 分支）
        let valid = input.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        let len = input.chars().count();
        if !valid || len == 0 || len > 16 {
            return Err(ArgumentSyntaxException::new(
                input,
                1,
                "玩家名须为 1..=16 位字母数字或下划线",
            ));
        }
        Ok(EntitySelector {
            target_type: EntitySelectorType::ByUsername,
            name: Some(input.to_string()),
        })
    }
}

/// 解析选择器括号内的过滤项（v1 仅接受 `name=`，其余键报错）。
fn parse_name_filter(input: &str, inner: &str) -> Result<Option<String>, ArgumentSyntaxException> {
    let mut name: Option<String> = None;
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = part.split_once('=').ok_or_else(|| {
            ArgumentSyntaxException::new(input, 1, "选择器参数需为 key=value 形式")
        })?;
        if key.trim() != "name" {
            return Err(ArgumentSyntaxException::new(
                input,
                1,
                &format!("v1 仅支持 name 过滤，不支持 {}", key.trim()),
            ));
        }
        let mut value = value.trim().to_string();
        if value.starts_with('!') {
            // v1 忽略排除语义，仅记录目标名
            value.remove(0);
        }
        if value.is_empty() {
            return Err(ArgumentSyntaxException::new(
                input,
                1,
                "name 过滤值不能为空",
            ));
        }
        name = Some(value);
    }
    Ok(name)
}

/// 解析单分量方块坐标：`~` 视为 0，否则须为整数。
fn parse_block_component(input: &str, tok: Option<&str>) -> Result<i32, ArgumentSyntaxException> {
    let tok = tok
        .ok_or_else(|| ArgumentSyntaxException::new(input, 1, "BlockPos 需要恰好 3 个坐标分量"))?;
    if tok == "~" {
        Ok(0)
    } else {
        tok.parse::<i32>()
            .map_err(|_| ArgumentSyntaxException::new(input, 1, "坐标分量不是整数"))
    }
}

/// 解析方块坐标 `x y z`。
fn parse_block_pos(input: &str) -> Result<[i32; 3], ArgumentSyntaxException> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.len() != 3 {
        return Err(ArgumentSyntaxException::new(
            input,
            1,
            "BlockPos 需要恰好 3 个坐标分量",
        ));
    }
    let mut iter = tokens.into_iter();
    let x = parse_block_component(input, iter.next())?;
    let y = parse_block_component(input, iter.next())?;
    let z = parse_block_component(input, iter.next())?;
    Ok([x, y, z])
}

/// 解析单分量相对坐标：`~`/`^` 前缀标记相对，数字部分为空视为 0.0。
fn parse_rel_component(
    input: &str,
    tok: Option<&str>,
) -> Result<(f64, bool), ArgumentSyntaxException> {
    let tok = tok.ok_or_else(|| {
        ArgumentSyntaxException::new(input, 1, "RelativeVec3 需要恰好 3 个坐标分量")
    })?;
    if tok.starts_with('~') || tok.starts_with('^') {
        let num_part: String = tok.chars().skip(1).collect();
        let v = if num_part.is_empty() {
            0.0
        } else {
            num_part
                .parse::<f64>()
                .map_err(|_| ArgumentSyntaxException::new(input, 1, "相对坐标数字非法"))?
        };
        Ok((v, true))
    } else {
        let v = tok
            .parse::<f64>()
            .map_err(|_| ArgumentSyntaxException::new(input, 1, "坐标分量不是数字"))?;
        Ok((v, false))
    }
}

/// 解析三维相对/绝对坐标。
fn parse_relative_vec3(input: &str) -> Result<RelativeVec3, ArgumentSyntaxException> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.len() != 3 {
        return Err(ArgumentSyntaxException::new(
            input,
            1,
            "RelativeVec3 需要恰好 3 个坐标分量",
        ));
    }
    let mut iter = tokens.into_iter();
    let (x, rx) = parse_rel_component(input, iter.next())?;
    let (y, ry) = parse_rel_component(input, iter.next())?;
    let (z, rz) = parse_rel_component(input, iter.next())?;
    Ok(RelativeVec3 {
        values: [x, y, z],
        relative: [rx, ry, rz],
    })
}

/// 去掉字符串最后一个字符（UTF-8 安全：在末字符起始边界切分）。
fn strip_last_char(s: &str) -> &str {
    match s.char_indices().last() {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// 解析时间：`t`/`s`/`d` 后缀或无后缀（tick），返回 tick 数。
fn parse_time(input: &str) -> Result<i32, ArgumentSyntaxException> {
    let (num_part, multiplier) = match input.chars().next_back() {
        Some('t') => (strip_last_char(input), 1i64),
        Some('s') => (strip_last_char(input), 20),
        Some('d') => (strip_last_char(input), 24_000),
        Some(c) if c.is_ascii_digit() => (input, 1),
        _ => {
            return Err(ArgumentSyntaxException::new(
                input,
                1,
                "时间需为数字，可带 t/s/d 后缀或无后缀（tick）",
            ));
        }
    };
    let value = num_part
        .parse::<i64>()
        .map_err(|_| ArgumentSyntaxException::new(input, 1, "时间需为数字"))?;
    let ticks = value
        .checked_mul(multiplier)
        .ok_or_else(|| ArgumentSyntaxException::new(input, 1, "时间超出 tick 范围"))?;
    i32::try_from(ticks).map_err(|_| ArgumentSyntaxException::new(input, 1, "时间超出 tick 范围"))
}

/// 命名空间字符集（`a-z0-9._-`）。
fn is_valid_namespace(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.' || c == '-'
        })
}

/// 路径字符集（`a-z0-9._-/`）。
fn is_valid_path(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '_'
                || c == '.'
                || c == '-'
                || c == '/'
        })
}

/// 解析资源定位符：无冒号补 `minecraft:` 前缀，校验命名空间与路径。
fn parse_resource_location(input: &str) -> Result<String, ArgumentSyntaxException> {
    let full: String = if input.contains(':') {
        input.to_string()
    } else {
        format!("minecraft:{input}")
    };
    let (ns, path) = full
        .split_once(':')
        .ok_or_else(|| ArgumentSyntaxException::new(input, 1, "资源定位符需含命名空间与路径"))?;
    if !is_valid_namespace(ns) {
        return Err(ArgumentSyntaxException::new(
            input,
            1,
            "资源定位符命名空间非法",
        ));
    }
    if !is_valid_path(path) {
        return Err(ArgumentSyntaxException::new(input, 1, "资源定位符路径非法"));
    }
    Ok(full)
}

/// 收集某参数的补全候选（框架侧；网络下发超出范围）。
///
/// 若该参数注册了 `suggestion_callback`，调用之填充 [`Suggestion`]；否则返回空。
pub fn collect_suggestion(
    arg: &dyn AnyArgument,
    sender: &dyn CommandSender,
    context: &CommandContext,
) -> Suggestion {
    let mut suggestion = Suggestion::empty();
    if let Some(cb) = arg.suggestion_callback() {
        cb.apply(sender, context, &mut suggestion, &mut |_msg: &str| {});
    }
    suggestion
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::resource::command::sender::ConsoleSender;
    use crate::resource::registries::{ItemDefinition, Registry};

    /// 测试辅助：解析成功并下转为具体类型。
    fn parse_ok<T>(arg: &Argument<T>, input: &str) -> T
    where
        T: Clone + Any + Send + Sync + PartialEq + std::fmt::Debug,
    {
        let sender = ConsoleSender;
        let boxed = arg.parse_erased(&sender, input).expect("解析应成功");
        let any: &dyn Any = &*boxed;
        any.downcast_ref::<T>().expect("类型下转").clone()
    }

    /// 测试辅助：解析失败并返回异常。
    fn parse_err<T>(arg: &Argument<T>, input: &str) -> ArgumentSyntaxException
    where
        T: Clone + Any + Send + Sync + 'static,
    {
        let sender = ConsoleSender;
        match arg.parse_erased(&sender, input) {
            Err(e) => e,
            Ok(_) => panic!("解析应失败"),
        }
    }

    /// 最小物品注册表（测试用）。
    fn sample_item_registry() -> ItemRegistry {
        let toml = r#"
            [[entry]]
            id = 264
            name = "minecraft:diamond"

            [[entry]]
            id = 1
            name = "minecraft:stone"
        "#;
        ItemRegistry(Registry::<ItemDefinition>::from_toml_str(toml).unwrap())
    }

    // ── ArgumentParserType ──

    #[test]
    fn parser_type_ids_aligned_with_java() {
        // 关键序位对齐 Java ArgumentParserType（0..=56）
        assert_eq!(ArgumentParserType::BOOL.id(), 0);
        assert_eq!(ArgumentParserType::INTEGER.id(), 3);
        assert_eq!(ArgumentParserType::STRING.id(), 5);
        assert_eq!(ArgumentParserType::ENTITY.id(), 6);
        assert_eq!(ArgumentParserType::BLOCK_POS.id(), 8);
        assert_eq!(ArgumentParserType::ITEM_STACK.id(), 14);
        assert_eq!(ArgumentParserType::COMPONENT.id(), 18);
        assert_eq!(ArgumentParserType::NBT_TAG.id(), 22);
        assert_eq!(ArgumentParserType::SCORE_HOLDER.id(), 31);
        assert_eq!(ArgumentParserType::TIME.id(), 43);
        assert_eq!(ArgumentParserType::RESOURCE_OR_TAG.id(), 44);
        assert_eq!(ArgumentParserType::RESOURCE_OR_TAG_KEY.id(), 45);
        assert_eq!(ArgumentParserType::RESOURCE.id(), 46);
        assert_eq!(ArgumentParserType::RESOURCE_KEY.id(), 47);
        assert_eq!(ArgumentParserType::UUID.id(), 56);
        // 节点标记为保留负值
        assert_eq!(ArgumentParserType::ROOT.id(), -3);
        assert_eq!(ArgumentParserType::LITERAL.id(), -2);
        assert_eq!(ArgumentParserType::ARGUMENT.id(), -1);
    }

    #[test]
    fn parser_type_from_id_roundtrip_and_unknown() {
        for t in [
            ArgumentParserType::BOOL,
            ArgumentParserType::DOUBLE,
            ArgumentParserType::LONG,
            ArgumentParserType::ENTITY,
            ArgumentParserType::COLUMN_POS,
            ArgumentParserType::VEC3,
            ArgumentParserType::NBT_COMPOUND_TAG,
            ArgumentParserType::RESOURCE_LOCATION,
            ArgumentParserType::GAMEMODE,
            ArgumentParserType::UUID,
            ArgumentParserType::ROOT,
            ArgumentParserType::LITERAL,
            ArgumentParserType::ARGUMENT,
        ] {
            assert_eq!(
                ArgumentParserType::from_id(t.id()),
                Some(t),
                "id {}",
                t.id()
            );
        }
        assert_eq!(ArgumentParserType::from_id(57), None);
        assert_eq!(ArgumentParserType::from_id(999), None);
        assert_eq!(ArgumentParserType::from_id(-4), None);
    }

    #[test]
    fn factory_parser_type_association() {
        // 既有工厂：映射到对应解析器类型
        assert_eq!(ArgumentType::Word("w").parser_type, None);
        assert_eq!(
            ArgumentType::String("s").parser_type,
            Some(ArgumentParserType::STRING)
        );
        assert_eq!(
            ArgumentType::Integer("i").parser_type,
            Some(ArgumentParserType::INTEGER)
        );
        assert_eq!(
            ArgumentType::Long("l").parser_type,
            Some(ArgumentParserType::LONG)
        );
        assert_eq!(
            ArgumentType::Float("f").parser_type,
            Some(ArgumentParserType::FLOAT)
        );
        assert_eq!(
            ArgumentType::Double("d").parser_type,
            Some(ArgumentParserType::DOUBLE)
        );
        assert_eq!(
            ArgumentType::Boolean("b").parser_type,
            Some(ArgumentParserType::BOOL)
        );
        // 新工厂
        assert_eq!(
            ArgumentType::Entity("e").parser_type,
            Some(ArgumentParserType::ENTITY)
        );
        assert_eq!(
            ArgumentType::Player("p").parser_type,
            Some(ArgumentParserType::GAME_PROFILE)
        );
        assert_eq!(
            ArgumentType::BlockPos("bp").parser_type,
            Some(ArgumentParserType::BLOCK_POS)
        );
        assert_eq!(
            ArgumentType::ItemStack("is", None).parser_type,
            Some(ArgumentParserType::ITEM_STACK)
        );
        assert_eq!(
            ArgumentType::NbtTag("n").parser_type,
            Some(ArgumentParserType::NBT_TAG)
        );
        assert_eq!(
            ArgumentType::Component("c").parser_type,
            Some(ArgumentParserType::COMPONENT)
        );
        assert_eq!(
            ArgumentType::ResourceLocation("r").parser_type,
            Some(ArgumentParserType::RESOURCE_LOCATION)
        );
        assert_eq!(
            ArgumentType::Uuid("u").parser_type,
            Some(ArgumentParserType::UUID)
        );
        assert_eq!(
            ArgumentType::Time("t").parser_type,
            Some(ArgumentParserType::TIME)
        );
        assert_eq!(
            ArgumentType::RelativeVec3("v").parser_type,
            Some(ArgumentParserType::VEC3)
        );
    }

    // ── Entity ──

    #[test]
    fn entity_selector_variants_success() {
        let arg = ArgumentType::Entity("e");
        assert_eq!(
            parse_ok(&arg, "@p"),
            EntitySelector {
                target_type: EntitySelectorType::NearestPlayer,
                name: None,
            }
        );
        assert_eq!(
            parse_ok(&arg, "@a"),
            EntitySelector {
                target_type: EntitySelectorType::AllPlayers,
                name: None,
            }
        );
        assert_eq!(
            parse_ok(&arg, "@e[name=zombie]"),
            EntitySelector {
                target_type: EntitySelectorType::AllEntities,
                name: Some("zombie".to_string()),
            }
        );
        assert_eq!(
            parse_ok(&arg, "Steve"),
            EntitySelector {
                target_type: EntitySelectorType::ByUsername,
                name: Some("Steve".to_string()),
            }
        );
    }

    #[test]
    fn entity_selector_failures() {
        let arg = ArgumentType::Entity("e");
        assert!(parse_err(&arg, "@x").message.contains("选择器变量"));
        assert!(parse_err(&arg, "@e[type=zombie]").message.contains("name"));
        assert!(parse_err(&arg, "@e[name=]").message.contains("空"));
        assert!(parse_err(&arg, "@e(name=zombie)").message.contains("包裹"));
        assert!(parse_err(&arg, "").message.contains("玩家名"));
    }

    // ── Player ──

    #[test]
    fn player_name_success() {
        assert_eq!(parse_ok(&ArgumentType::Player("p"), "Steve"), "Steve");
        assert_eq!(parse_ok(&ArgumentType::Player("p"), "a_b_c"), "a_b_c");
    }

    #[test]
    fn player_name_failures() {
        assert!(
            parse_err(&ArgumentType::Player("p"), "")
                .message
                .contains("空")
        );
        assert!(
            parse_err(&ArgumentType::Player("p"), "名字带中文")
                .message
                .contains("字母")
        );
        assert!(
            parse_err(&ArgumentType::Player("p"), "this_name_is_too_long_ok")
                .message
                .contains("字母")
        );
    }

    // ── BlockPos ──

    #[test]
    fn block_pos_success() {
        assert_eq!(parse_ok(&ArgumentType::BlockPos("bp"), "1 2 3"), [1, 2, 3]);
        assert_eq!(parse_ok(&ArgumentType::BlockPos("bp"), "~ 5 ~"), [0, 5, 0]);
        assert_eq!(
            parse_ok(&ArgumentType::BlockPos("bp"), "-1 0 7"),
            [-1, 0, 7]
        );
    }

    #[test]
    fn block_pos_failures() {
        assert!(
            parse_err(&ArgumentType::BlockPos("bp"), "1 2")
                .message
                .contains("3")
        );
        assert!(
            parse_err(&ArgumentType::BlockPos("bp"), "1 x 3")
                .message
                .contains("整数")
        );
        assert!(
            parse_err(&ArgumentType::BlockPos("bp"), "1 2 3 4")
                .message
                .contains("3")
        );
    }

    // ── ItemStack ──

    #[test]
    fn item_stack_success() {
        let reg = sample_item_registry();
        let arg = ArgumentType::ItemStack("is", Some(reg));
        let item = parse_ok(&arg, "minecraft:diamond");
        assert_eq!(item.material, 264);
        assert_eq!(item.amount, 1);
        // 无命名空间时补 minecraft: 前缀
        let item2 = parse_ok(&arg, "stone");
        assert_eq!(item2.material, 1);
        assert_eq!(item2.amount, 1);
    }

    #[test]
    fn item_stack_failures() {
        let reg = sample_item_registry();
        let arg = ArgumentType::ItemStack("is", Some(reg));
        assert!(
            parse_err(&arg, "minecraft:bedrock")
                .message
                .contains("未知")
        );
        // 未提供注册表
        let none = ArgumentType::ItemStack("is", None);
        assert!(parse_err(&none, "diamond").message.contains("注册表"));
    }

    // ── NbtTag ──

    #[test]
    fn nbt_tag_success() {
        // 二进制 NBT：根 Compound（0x0a）+ 空根名（长度 0），条目 {b: Int(7)}
        // 注意：NBT 字符串长度为 VarInt，"b" 的长度 1 编码为单字节 0x01
        let bytes = "\u{0a}\u{00}\u{03}\u{01}b\u{00}\u{00}\u{00}\u{07}\u{00}";
        let tag = parse_ok(&ArgumentType::NbtTag("n"), bytes);
        assert_eq!(tag, NbtTag::Compound(vec![("b".into(), NbtTag::Int(7))]));
    }

    #[test]
    fn nbt_tag_failures() {
        // 截断的二进制 NBT（根 Compound 后无 payload）
        assert!(
            parse_err(&ArgumentType::NbtTag("n"), "\u{0a}\u{00}")
                .message
                .contains("NBT")
        );
        assert!(
            parse_err(&ArgumentType::NbtTag("n"), "")
                .message
                .contains("NBT")
        );
    }

    // ── Component（经 text_component）──

    #[test]
    fn component_text_success() {
        // 二进制 NBT：根 Compound（0x0a）+ 空根名（0x00），条目 text:"hi"
        // 字符串长度均为 VarInt，单字节编码（"text"=0x04、"hi"=0x02）
        let bytes = "\u{0a}\u{00}\u{08}\u{04}text\u{02}hi\u{00}";
        let c = parse_ok(&ArgumentType::Component("c"), bytes);
        assert_eq!(c, TextComponent::text("hi"));
    }

    #[test]
    fn component_failures() {
        // 截断的 NBT（根 Compound 后无 payload）
        assert!(
            parse_err(&ArgumentType::Component("c"), "\u{0a}\u{00}")
                .message
                .contains("文本组件")
        );
        // NBT 合法但非组件结构（text 键类型为 Int 而非 String）
        let bad = "\u{0a}\u{00}\u{03}\u{04}text\u{00}\u{00}\u{00}\u{07}\u{00}";
        assert!(
            parse_err(&ArgumentType::Component("c"), bad)
                .message
                .contains("文本组件")
        );
    }

    // ── ResourceLocation ──

    #[test]
    fn resource_location_success() {
        assert_eq!(
            parse_ok(&ArgumentType::ResourceLocation("r"), "minecraft:stone"),
            "minecraft:stone"
        );
        // 无冒号补 minecraft: 前缀
        assert_eq!(
            parse_ok(&ArgumentType::ResourceLocation("r"), "diamond"),
            "minecraft:diamond"
        );
        assert_eq!(
            parse_ok(
                &ArgumentType::ResourceLocation("r"),
                "minecraft:foo/bar_baz"
            ),
            "minecraft:foo/bar_baz"
        );
    }

    #[test]
    fn resource_location_failures() {
        assert!(
            parse_err(&ArgumentType::ResourceLocation("r"), ":stone")
                .message
                .contains("命名空间")
        );
        assert!(
            parse_err(&ArgumentType::ResourceLocation("r"), "minecraft:")
                .message
                .contains("路径")
        );
        assert!(
            parse_err(&ArgumentType::ResourceLocation("r"), "Minecraft:Stone")
                .message
                .contains("非法")
        );
    }

    // ── Uuid ──

    #[test]
    fn uuid_success() {
        let parsed = parse_ok(
            &ArgumentType::Uuid("u"),
            "550e8400-e29b-41d4-a716-446655440000",
        );
        assert_eq!(
            parsed,
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
        );
    }

    #[test]
    fn uuid_failures() {
        assert!(
            parse_err(&ArgumentType::Uuid("u"), "not-a-uuid")
                .message
                .contains("UUID")
        );
        assert!(
            parse_err(&ArgumentType::Uuid("u"), "550e8400-e29b")
                .message
                .contains("UUID")
        );
    }

    // ── Time ──

    #[test]
    fn time_success() {
        assert_eq!(parse_ok(&ArgumentType::Time("t"), "75"), 75);
        assert_eq!(parse_ok(&ArgumentType::Time("t"), "75t"), 75);
        assert_eq!(parse_ok(&ArgumentType::Time("t"), "25s"), 500);
        assert_eq!(parse_ok(&ArgumentType::Time("t"), "50d"), 1_200_000);
        assert_eq!(parse_ok(&ArgumentType::Time("t"), "-5s"), -100);
    }

    #[test]
    fn time_failures() {
        assert!(
            parse_err(&ArgumentType::Time("t"), "abc")
                .message
                .contains("数字")
        );
        assert!(
            parse_err(&ArgumentType::Time("t"), "75x")
                .message
                .contains("后缀")
        );
        // 溢出 tick 范围
        assert!(
            parse_err(&ArgumentType::Time("t"), "999999999999d")
                .message
                .contains("超出")
        );
    }

    // ── RelativeVec3 ──

    #[test]
    fn relative_vec3_success() {
        let v = parse_ok(&ArgumentType::RelativeVec3("v"), "-1.2 ~ 5");
        assert_eq!(v.values, [-1.2, 0.0, 5.0]);
        assert_eq!(v.relative, [false, true, false]);

        let v2 = parse_ok(&ArgumentType::RelativeVec3("v"), "^ ^1.5 ~");
        assert_eq!(v2.values, [0.0, 1.5, 0.0]);
        assert_eq!(v2.relative, [true, true, true]);
    }

    #[test]
    fn relative_vec3_failures() {
        assert!(
            parse_err(&ArgumentType::RelativeVec3("v"), "1 2")
                .message
                .contains("3")
        );
        assert!(
            parse_err(&ArgumentType::RelativeVec3("v"), "1 ~ x")
                .message
                .contains("数字")
        );
        assert!(
            parse_err(&ArgumentType::RelativeVec3("v"), "1.2.3 ~ 0")
                .message
                .contains("数字")
        );
    }
}
