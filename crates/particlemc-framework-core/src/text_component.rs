// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Minecraft 文本组件（adventure 语义对齐，含 NBT 序列化）。
//!
//! 本模块实现聊天 / 物品文本组件的 adventure 等价子集：`Empty`、`Text`、
//! `Translatable`、`Keybind`、`Scoreboard`、`Selector`、`Nbt`、`ClickEvent`
//! 与 `HoverEvent`，配套 [`Style`] 完整样式与 [`Color`] 值类型，并提供与 NBT 互逆的
//! [`Component::to_nbt`] / [`Component::from_nbt`] 序列化，供物品组件
//! `custom_name`(6) / `lore`(11) 等以 NBT 承载（见 [`crate::item_stack`]）。
//!
//! NBT 线格式对齐框架的 `ComponentNetworkBufferTypeImpl`
//! （写 `TAG_COMPOUND`(0x0a) 后接 anonymous Compound payload）与 adventure 的
//! NBT 序列化约定：
//!
//! - `Text` → `Compound { text, color?, italic?, bold?, underlined?, strikethrough?,
//!   obfuscated?, insertion?, click_event?, hover_event? }`（`color` 为 ARGB `u32`，
//!   以 `TAG_Int` 位模式承载；italic/bold/underlined/strikethrough/obfuscated 仅真
//!   值写入 `TAG_Byte(1)`，缺省为 false；`insertion` 写 `TAG_String`；
//!   `click_event` / `hover_event` 嵌套 Compound 见下）；
//! - `ClickEvent` → `Compound { click_event: Compound { action, value } }`；
//! - `HoverEvent` → `Compound { hover_event: Compound { action, value } }`；
//! - `Translatable` → `Compound { translate, fallback?, with? }`（`with` 为子组件
//!   `TAG_List`）；
//! - `Keybind` → `Compound { keybind }`；
//! - `Scoreboard` → `Compound { score: Compound { name, objective } }`；
//! - `Selector` → `Compound { selector, separator? }`；
//! - `Nbt` → `Compound { nbt: Compound { nbt_path, interpret?, separator? } }`
//!   （`interpret` 仅真值写 `TAG_Byte(1)`）；
//! - `Empty` → 空 `Compound`。
//!
//! `Text` 既有字段字节序保持 `text` → `color?` → `italic?` → `bold?` 不变，新增
//! 样式字段在尾部追加，保证旧承载（如既有 custom_name/lore 的 NBT 字节）向后兼容。
//!
//! 变更标识符：`complete-missing-subsystems`（R1 adventure 文本组件扩展），
//! 见 `.specs/complete-missing-subsystems/spec.md`。

use crate::protocol::nbt::{NbtError, NbtTag};

/// ARGB 颜色值类型（对齐 Minecraft / adventure 的 `NamedTextColor` 与 RGB 颜色）。
///
/// 高位字节为 alpha（通常 `0xFF` 不透明），低 24 位为 RGB。颜色名解析
/// [`Color::from_name`] 对齐 Minecraft 内置 16 色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    /// 按 Minecraft 内置 16 色名解析颜色（如 `"red"` → `Color(0xFFFF5555)`）。
    ///
    /// 未知名称返回 `None`。名称值对齐 Minecraft 聊天格式的 `Formatting` 命名色
    /// （`black` 至 `white`，含 dark_* / light_purple / aqua 等 16 色）。
    pub fn from_name(name: &str) -> Option<Color> {
        let rgb = match name {
            "black" => 0x00_00_00,
            "dark_blue" => 0x00_00_AA,
            "dark_green" => 0x00_AA_00,
            "dark_aqua" => 0x00_AA_AA,
            "dark_red" => 0xAA_00_00,
            "dark_purple" => 0xAA_00_AA,
            "gold" => 0xFF_AA_00,
            "gray" => 0xAA_AA_AA,
            "dark_gray" => 0x55_55_55,
            "blue" => 0x55_55_FF,
            "green" => 0x55_FF_55,
            "aqua" => 0x55_FF_FF,
            "red" => 0xFF_55_55,
            "light_purple" => 0xFF_55_FF,
            "yellow" => 0xFF_FF_55,
            "white" => 0xFF_FF_FF,
            _ => return None,
        };
        Some(Color(0xFF_00_00_00 | rgb))
    }

    /// 序列化为 NBT（`TAG_Int`，ARGB 位模式）。
    pub fn to_nbt(&self) -> NbtTag {
        NbtTag::Int(i32::from_be_bytes(self.0.to_be_bytes()))
    }

    /// 从 NBT 反序列化（`TAG_Int`，ARGB 位模式）。
    ///
    /// 非 `TAG_Int` 输入返回 [`NbtError::InvalidListType`]。
    pub fn from_nbt(tag: &NbtTag) -> Result<Self, NbtError> {
        match tag {
            NbtTag::Int(i) => Ok(Color(u32::from_be_bytes(i.to_be_bytes()))),
            _ => Err(NbtError::InvalidListType),
        }
    }
}

/// 文本样式（adventure 样式子集）。
///
/// `Default` 为全缺省：无颜色、全部布尔样式为 false、无插入文本、无点击/悬停事件。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Style {
    /// ARGB 颜色（`u32`，如 `0xFF_FF0000` 为不透明红；缺省无颜色）。
    pub color: Option<u32>,
    /// 是否加粗。
    pub bold: bool,
    /// 是否斜体。
    pub italic: bool,
    /// 是否下划线。
    pub underlined: bool,
    /// 是否删除线。
    pub strikethrough: bool,
    /// 是否混淆（随机字符）。
    pub obfuscated: bool,
    /// 点击时插入到聊天输入框的文本。
    pub insertion: Option<String>,
    /// 点击事件（`None` 表示无事件）。
    pub click_event: Option<ClickEvent>,
    /// 悬停事件（`None` 表示无事件）。
    pub hover_event: Option<HoverEvent>,
}

impl Style {
    /// 以指定 ARGB 颜色构造样式（其余字段缺省）。
    pub fn with_color(color: u32) -> Self {
        Style {
            color: Some(color),
            ..Style::default()
        }
    }

    /// 以指定插入文本构造样式（其余字段缺省）。
    pub fn with_insertion(insertion: String) -> Self {
        Style {
            insertion: Some(insertion),
            ..Style::default()
        }
    }
}

/// 点击事件动作（对齐 Minecraft 协议规范中的 i32 编码）。
///
/// - `RunCommand`（0）：执行命令。
/// - `OpenUrl`（1）：在浏览器中打开 URL。
/// - `SuggestCommand`（2）：将命令填入聊天输入框（不执行）。
/// - `ChangePage`（3）：切换到书的指定页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    /// 执行命令（i32 = 0）。
    RunCommand,
    /// 打开 URL（i32 = 1）。
    OpenUrl,
    /// 建议命令（i32 = 2）。
    SuggestCommand,
    /// 翻页（i32 = 3）。
    ChangePage,
}

impl ClickAction {
    /// 返回协议 i32 编码。
    pub fn code(&self) -> i32 {
        match self {
            ClickAction::RunCommand => 0,
            ClickAction::OpenUrl => 1,
            ClickAction::SuggestCommand => 2,
            ClickAction::ChangePage => 3,
        }
    }

    /// 按协议 i32 解码；非法值返回 `None`。
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(ClickAction::RunCommand),
            1 => Some(ClickAction::OpenUrl),
            2 => Some(ClickAction::SuggestCommand),
            3 => Some(ClickAction::ChangePage),
            _ => None,
        }
    }
}

/// 悬停事件动作（对齐 Minecraft 协议规范中的 i32 编码）。
///
/// - `ShowText`（0）：展示文本。
/// - `ShowEntity`（1）：展示实体信息。
/// - `ShowItem`（2）：展示物品信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverAction {
    /// 展示文本（i32 = 0）。
    ShowText,
    /// 展示实体信息（i32 = 1）。
    ShowEntity,
    /// 展示物品信息（i32 = 2）。
    ShowItem,
}

impl HoverAction {
    /// 返回协议 i32 编码。
    pub fn code(&self) -> i32 {
        match self {
            HoverAction::ShowText => 0,
            HoverAction::ShowEntity => 1,
            HoverAction::ShowItem => 2,
        }
    }

    /// 按协议 i32 解码；非法值返回 `None`。
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(HoverAction::ShowText),
            1 => Some(HoverAction::ShowEntity),
            2 => Some(HoverAction::ShowItem),
            _ => None,
        }
    }
}

/// 点击事件，可嵌入 [`Style::click_event`] 或作为独立 [`Component`] 变体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickEvent {
    /// 点击动作。
    pub action: ClickAction,
    /// 动作参数（命令 / URL / 页码等）。
    pub value: String,
}

/// 悬停事件，可嵌入 [`Style::hover_event`] 或作为独立 [`Component`] 变体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverEvent {
    /// 悬停动作。
    pub action: HoverAction,
    /// 动作参数（文本 / 实体 NBT / 物品 NBT 等）。
    pub value: String,
}

/// 文本组件值（adventure 等价子集）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Component {
    /// 空文本（序列化为空 `Compound`）。
    Empty,
    /// 普通文本：`text` + 完整样式。
    Text {
        /// 文本内容。
        text: String,
        /// 样式（颜色 / 布尔样式 / 插入文本）。
        style: Style,
    },
    /// 翻译文本：`key` + 可选 fallback + 参数子组件（对应 `%s` 等占位符）。
    Translatable {
        /// 翻译键。
        key: String,
        /// 备用文本（翻译键缺失时展示）。
        fallback: Option<String>,
        /// 翻译参数（子组件）。
        args: Vec<Component>,
    },
    /// 按键绑定：渲染为对应键位名称（如 `"key.attack"`）。
    Keybind {
        /// 按键绑定键（`key.xxx`）。
        key: String,
    },
    /// 计分板分数：渲染为某计分板目标下指定实体/玩家的分数。
    Scoreboard {
        /// 实体选择名（玩家名或实体 UUID 等）。
        name: String,
        /// 计分板目标名。
        objective: String,
    },
    /// 实体选择器：渲染为选择器匹配的实体展示名列表。
    Selector {
        /// 选择器字符串（如 `"@e[type=minecraft:cow]"`）。
        pattern: String,
        /// 多个实体展示名之间的分隔符。
        separator: Option<String>,
    },
    /// NBT 值：渲染为实体 / 方块实体 / 存储的指定 NBT 路径值。
    Nbt {
        /// NBT 路径（如 `"Pos[0]"`）。
        nbt_path: String,
        /// 是否按解释（interpret）方式解析路径（`false` 按字面处理）。
        interpret: bool,
        /// 列表值元素之间的分隔符。
        separator: Option<String>,
    },
    /// 点击事件：用户点击文本时触发指定动作。
    ClickEvent {
        /// 点击动作类型。
        action: ClickAction,
        /// 动作参数（命令 / URL / 页码等）。
        value: String,
    },
    /// 悬停事件：鼠标悬停文本时展示对应内容。
    HoverEvent {
        /// 悬停动作类型。
        action: HoverAction,
        /// 动作参数（文本 / 实体 NBT / 物品 NBT 等）。
        value: String,
    },
}

impl Component {
    /// 以普通文本构造（无颜色、无样式）。
    ///
    /// 等价于 `Text { text, style: Style::default() }`，与 T1 前的字节输出一致。
    pub fn text(s: &str) -> Self {
        Component::Text {
            text: s.to_owned(),
            style: Style::default(),
        }
    }

    /// 以点击事件构造组件。
    pub fn click_event(action: ClickAction, value: &str) -> Self {
        Component::ClickEvent {
            action,
            value: value.to_owned(),
        }
    }

    /// 以悬停事件构造组件。
    pub fn hover_event(action: HoverAction, value: &str) -> Self {
        Component::HoverEvent {
            action,
            value: value.to_owned(),
        }
    }

    /// 序列化为 NBT（返回 Compound tag）。
    ///
    /// 各类型映射见模块文档；`Text` 既有字段字节序（`text`/`color`/`italic`/`bold`）
    /// 保持不变，新增样式字段在尾部追加。该函数与 [`Component::from_nbt`] 互逆
    /// （roundtrip 无损）。
    pub fn to_nbt(&self) -> NbtTag {
        match self {
            Component::Empty => NbtTag::Compound(Vec::new()),
            Component::Text { text, style } => {
                let mut entries = vec![("text".to_string(), NbtTag::String(text.clone()))];
                if let Some(c) = style.color {
                    // u32 → i32 用位模式转换（非 `as` 缩窄），保证 ARGB 无损。
                    entries.push((
                        "color".to_string(),
                        NbtTag::Int(i32::from_be_bytes(c.to_be_bytes())),
                    ));
                }
                // 布尔样式仅真值写入 `TAG_Byte(1)`（缺省即不写，向后兼容）。
                if style.italic {
                    entries.push(("italic".to_string(), NbtTag::Byte(1)));
                }
                if style.bold {
                    entries.push(("bold".to_string(), NbtTag::Byte(1)));
                }
                if style.underlined {
                    entries.push(("underlined".to_string(), NbtTag::Byte(1)));
                }
                if style.strikethrough {
                    entries.push(("strikethrough".to_string(), NbtTag::Byte(1)));
                }
                if style.obfuscated {
                    entries.push(("obfuscated".to_string(), NbtTag::Byte(1)));
                }
                if let Some(ins) = &style.insertion {
                    entries.push(("insertion".to_string(), NbtTag::String(ins.clone())));
                }
                if let Some(ce) = &style.click_event {
                    entries.push((
                        "click_event".to_string(),
                        NbtTag::Compound(vec![
                            ("action".to_string(), NbtTag::Int(ce.action.code())),
                            ("value".to_string(), NbtTag::String(ce.value.clone())),
                        ]),
                    ));
                }
                if let Some(he) = &style.hover_event {
                    entries.push((
                        "hover_event".to_string(),
                        NbtTag::Compound(vec![
                            ("action".to_string(), NbtTag::Int(he.action.code())),
                            ("value".to_string(), NbtTag::String(he.value.clone())),
                        ]),
                    ));
                }
                NbtTag::Compound(entries)
            }
            Component::Translatable {
                key,
                fallback,
                args,
            } => {
                let mut entries = vec![("translate".to_string(), NbtTag::String(key.clone()))];
                if let Some(fb) = fallback {
                    entries.push(("fallback".to_string(), NbtTag::String(fb.clone())));
                }
                if !args.is_empty() {
                    entries.push((
                        "with".to_string(),
                        NbtTag::List(args.iter().map(|a| a.to_nbt()).collect()),
                    ));
                }
                NbtTag::Compound(entries)
            }
            Component::Keybind { key } => {
                NbtTag::Compound(vec![("keybind".to_string(), NbtTag::String(key.clone()))])
            }
            Component::Scoreboard { name, objective } => NbtTag::Compound(vec![(
                "score".to_string(),
                NbtTag::Compound(vec![
                    ("name".to_string(), NbtTag::String(name.clone())),
                    ("objective".to_string(), NbtTag::String(objective.clone())),
                ]),
            )]),
            Component::Selector { pattern, separator } => {
                let mut entries = vec![("selector".to_string(), NbtTag::String(pattern.clone()))];
                if let Some(sep) = separator {
                    entries.push(("separator".to_string(), NbtTag::String(sep.clone())));
                }
                NbtTag::Compound(entries)
            }
            Component::Nbt {
                nbt_path,
                interpret,
                separator,
            } => {
                let mut inner = vec![("nbt_path".to_string(), NbtTag::String(nbt_path.clone()))];
                if *interpret {
                    inner.push(("interpret".to_string(), NbtTag::Byte(1)));
                }
                if let Some(sep) = separator {
                    inner.push(("separator".to_string(), NbtTag::String(sep.clone())));
                }
                NbtTag::Compound(vec![("nbt".to_string(), NbtTag::Compound(inner))])
            }
            Component::ClickEvent { action, value } => NbtTag::Compound(vec![(
                "click_event".to_string(),
                NbtTag::Compound(vec![
                    ("action".to_string(), NbtTag::Int(action.code())),
                    ("value".to_string(), NbtTag::String(value.clone())),
                ]),
            )]),
            Component::HoverEvent { action, value } => NbtTag::Compound(vec![(
                "hover_event".to_string(),
                NbtTag::Compound(vec![
                    ("action".to_string(), NbtTag::Int(action.code())),
                    ("value".to_string(), NbtTag::String(value.clone())),
                ]),
            )]),
        }
    }

    /// 从 NBT 反序列化。
    ///
    /// 判定规则（按键名依次匹配）：`keybind` → `Keybind`；`score` → `Scoreboard`；
    /// `selector` → `Selector`；`nbt` → `Nbt`；`translate` → `Translatable`；
    /// `text` → `Text`；否则（空 Compound）→ `Empty`。键存在但类型不符（如
    /// `text` 为 `TAG_Int`）或输入非 Compound 时返回 [`NbtError::InvalidListType`]。
    pub fn from_nbt(tag: &NbtTag) -> Result<Self, NbtError> {
        let entries = match tag {
            NbtTag::Compound(e) => e.as_slice(),
            _ => return Err(NbtError::InvalidListType),
        };
        if entries.iter().any(|(k, _)| k.as_str() == "keybind") {
            let key = entry_str(entries, "keybind")?.ok_or(NbtError::InvalidListType)?;
            Ok(Component::Keybind {
                key: key.to_owned(),
            })
        } else if entries.iter().any(|(k, _)| k.as_str() == "score") {
            let inner = entry_compound(entries, "score")?.ok_or(NbtError::InvalidListType)?;
            let name = entry_str(inner, "name")?.ok_or(NbtError::InvalidListType)?;
            let objective = entry_str(inner, "objective")?.ok_or(NbtError::InvalidListType)?;
            Ok(Component::Scoreboard {
                name: name.to_owned(),
                objective: objective.to_owned(),
            })
        } else if entries.iter().any(|(k, _)| k.as_str() == "selector") {
            let pattern = entry_str(entries, "selector")?.ok_or(NbtError::InvalidListType)?;
            let separator = entry_str(entries, "separator")?.map(str::to_owned);
            Ok(Component::Selector {
                pattern: pattern.to_owned(),
                separator,
            })
        } else if entries.iter().any(|(k, _)| k.as_str() == "nbt") {
            let inner = entry_compound(entries, "nbt")?.ok_or(NbtError::InvalidListType)?;
            let nbt_path = entry_str(inner, "nbt_path")?.ok_or(NbtError::InvalidListType)?;
            let interpret = entry_byte(inner, "interpret")?.is_some_and(|b| b != 0);
            let separator = entry_str(inner, "separator")?.map(str::to_owned);
            Ok(Component::Nbt {
                nbt_path: nbt_path.to_owned(),
                interpret,
                separator,
            })
        } else if entries.iter().any(|(k, _)| k.as_str() == "click_event")
            && !entries.iter().any(|(k, _)| k.as_str() == "text")
        {
            // 独立 ClickEvent 组件（非 Text 样式内的 click_event 字段）；
            // 若同时含 `text` 则属于 Text 样式，由下方 Text 分支处理。
            let inner = entry_compound(entries, "click_event")?.ok_or(NbtError::InvalidListType)?;
            let code = entry_int(inner, "action")?.ok_or(NbtError::InvalidListType)?;
            let action = ClickAction::from_code(code).ok_or(NbtError::InvalidListType)?;
            let value = entry_str(inner, "value")?
                .ok_or(NbtError::InvalidListType)?
                .to_owned();
            Ok(Component::ClickEvent { action, value })
        } else if entries.iter().any(|(k, _)| k.as_str() == "hover_event")
            && !entries.iter().any(|(k, _)| k.as_str() == "text")
        {
            // 独立 HoverEvent 组件（非 Text 样式内的 hover_event 字段）。
            let inner = entry_compound(entries, "hover_event")?.ok_or(NbtError::InvalidListType)?;
            let code = entry_int(inner, "action")?.ok_or(NbtError::InvalidListType)?;
            let action = HoverAction::from_code(code).ok_or(NbtError::InvalidListType)?;
            let value = entry_str(inner, "value")?
                .ok_or(NbtError::InvalidListType)?
                .to_owned();
            Ok(Component::HoverEvent { action, value })
        } else if entries.iter().any(|(k, _)| k.as_str() == "translate") {
            let key = entry_str(entries, "translate")?.ok_or(NbtError::InvalidListType)?;
            let fallback = entry_str(entries, "fallback")?.map(str::to_owned);
            let args = entry_component_list(entries, "with")?.unwrap_or_default();
            Ok(Component::Translatable {
                key: key.to_owned(),
                fallback,
                args,
            })
        } else if entries.iter().any(|(k, _)| k.as_str() == "text") {
            let text = entry_str(entries, "text")?.ok_or(NbtError::InvalidListType)?;
            let style = Style {
                color: entry_int(entries, "color")?.map(|c| u32::from_be_bytes(c.to_be_bytes())),
                bold: entry_byte(entries, "bold")?.is_some_and(|b| b != 0),
                italic: entry_byte(entries, "italic")?.is_some_and(|b| b != 0),
                underlined: entry_byte(entries, "underlined")?.is_some_and(|b| b != 0),
                strikethrough: entry_byte(entries, "strikethrough")?.is_some_and(|b| b != 0),
                obfuscated: entry_byte(entries, "obfuscated")?.is_some_and(|b| b != 0),
                insertion: entry_str(entries, "insertion")?.map(str::to_owned),
                click_event: extract_click_event(entries)?,
                hover_event: extract_hover_event(entries)?,
            };
            Ok(Component::Text {
                text: text.to_owned(),
                style,
            })
        } else {
            Ok(Component::Empty)
        }
    }

    /// 拼接纯文本（应用 / 调试用）。
    ///
    /// `Empty` → 空串；`Text` → `text`；`Translatable` → `fallback`（缺省用 `key`）
    /// 后接各参数子组件的纯文本；`Keybind` → 占位串 `"{key}"`；`Scoreboard` →
    /// `name`；`Selector` → `pattern`；`Nbt` → 空串；`ClickEvent` / `HoverEvent`
    /// → 空串。
    ///
    /// 占位语义：`Keybind` 的实际展示依赖客户端按键绑定表（无翻译上下文），此处以
    /// `"{key}"` 占位；`Nbt` 的实际值依赖运行时实体/方块 NBT 数据，此处以空串占位。
    pub fn plain_text(&self) -> String {
        match self {
            Component::Empty => String::new(),
            Component::Text { text, .. } => text.clone(),
            Component::Translatable {
                key,
                fallback,
                args,
            } => {
                let mut s = fallback.clone().unwrap_or_else(|| key.clone());
                for a in args {
                    s.push_str(&a.plain_text());
                }
                s
            }
            Component::Keybind { .. } => "{key}".to_string(),
            Component::Scoreboard { name, .. } => name.clone(),
            Component::Selector { pattern, .. } => pattern.clone(),
            Component::Nbt { .. } => String::new(),
            Component::ClickEvent { .. } => String::new(),
            Component::HoverEvent { .. } => String::new(),
        }
    }
}

/// 持有文本组件的对象（供 R12/R13/R14 的聊天/消息承载实现）。
///
/// `components()` 收集对象持有的所有组件；`copy_with_operator` 对每个持有的组件
/// 应用 `op` 后返回复制结果。本模块为 [`Component`] 自身实现：单个组件即视为持有
/// 自身一个组件。
pub trait ComponentHolder: Sized {
    /// 收集持有的所有组件。
    fn components(&self) -> Vec<Component>;

    /// 对每个持有的组件应用 `op`，返回应用后的复制结果。
    fn copy_with_operator(&self, op: &dyn Fn(&Component) -> Component) -> Self;
}

impl ComponentHolder for Component {
    fn components(&self) -> Vec<Component> {
        vec![self.clone()]
    }

    fn copy_with_operator(&self, op: &dyn Fn(&Component) -> Component) -> Self {
        op(self)
    }
}

/// 提取 Compound 中指定字符串键；键缺失返回 `Ok(None)`，存在但类型不符返回结构错误。
fn entry_str<'a>(entries: &'a [(String, NbtTag)], key: &str) -> Result<Option<&'a str>, NbtError> {
    match entries.iter().find(|(k, _)| k.as_str() == key) {
        None => Ok(None),
        Some((_, NbtTag::String(s))) => Ok(Some(s)),
        Some(_) => Err(NbtError::InvalidListType),
    }
}

/// 提取 Compound 中指定 `TAG_Int` 键。
fn entry_int(entries: &[(String, NbtTag)], key: &str) -> Result<Option<i32>, NbtError> {
    match entries.iter().find(|(k, _)| k.as_str() == key) {
        None => Ok(None),
        Some((_, NbtTag::Int(i))) => Ok(Some(*i)),
        Some(_) => Err(NbtError::InvalidListType),
    }
}

/// 提取 Compound 中指定 `TAG_Byte` 键。
fn entry_byte(entries: &[(String, NbtTag)], key: &str) -> Result<Option<i8>, NbtError> {
    match entries.iter().find(|(k, _)| k.as_str() == key) {
        None => Ok(None),
        Some((_, NbtTag::Byte(b))) => Ok(Some(*b)),
        Some(_) => Err(NbtError::InvalidListType),
    }
}

/// 提取 Compound 中指定嵌套 `TAG_Compound` 键（返回其 entries）。
fn entry_compound<'a>(
    entries: &'a [(String, NbtTag)],
    key: &str,
) -> Result<Option<&'a [(String, NbtTag)]>, NbtError> {
    match entries.iter().find(|(k, _)| k.as_str() == key) {
        None => Ok(None),
        Some((_, NbtTag::Compound(inner))) => Ok(Some(inner.as_slice())),
        Some(_) => Err(NbtError::InvalidListType),
    }
}

/// 提取 Compound 中指定 `TAG_List` 键，并逐元素解析为子组件。
fn entry_component_list(
    entries: &[(String, NbtTag)],
    key: &str,
) -> Result<Option<Vec<Component>>, NbtError> {
    match entries.iter().find(|(k, _)| k.as_str() == key) {
        None => Ok(None),
        Some((_, NbtTag::List(items))) => {
            let mut comps = Vec::with_capacity(items.len());
            for item in items {
                comps.push(Component::from_nbt(item)?);
            }
            Ok(Some(comps))
        }
        Some(_) => Err(NbtError::InvalidListType),
    }
}

/// 从 Compound entries 中提取 `click_event` 字段（若有）。
fn extract_click_event(entries: &[(String, NbtTag)]) -> Result<Option<ClickEvent>, NbtError> {
    let inner = match entry_compound(entries, "click_event")? {
        Some(inner) => inner,
        None => return Ok(None),
    };
    let code = entry_int(inner, "action")?.ok_or(NbtError::InvalidListType)?;
    let action = ClickAction::from_code(code).ok_or(NbtError::InvalidListType)?;
    let value = entry_str(inner, "value")?
        .ok_or(NbtError::InvalidListType)?
        .to_owned();
    Ok(Some(ClickEvent { action, value }))
}

/// 从 Compound entries 中提取 `hover_event` 字段（若有）。
fn extract_hover_event(entries: &[(String, NbtTag)]) -> Result<Option<HoverEvent>, NbtError> {
    let inner = match entry_compound(entries, "hover_event")? {
        Some(inner) => inner,
        None => return Ok(None),
    };
    let code = entry_int(inner, "action")?.ok_or(NbtError::InvalidListType)?;
    let action = HoverAction::from_code(code).ok_or(NbtError::InvalidListType)?;
    let value = entry_str(inner, "value")?
        .ok_or(NbtError::InvalidListType)?
        .to_owned();
    Ok(Some(HoverEvent { action, value }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 构造一个带完整样式的 Text 组件。
    fn styled_text() -> Component {
        Component::Text {
            text: "你好世界".to_string(),
            style: Style {
                color: Some(0xFF_00_00_FF),
                bold: true,
                italic: true,
                underlined: true,
                strikethrough: false,
                obfuscated: false,
                insertion: Some("插入".to_string()),
                click_event: None,
                hover_event: None,
            },
        }
    }

    /// 构造一个带嵌套参数与 fallback 的 Translatable 组件。
    fn translatable() -> Component {
        Component::Translatable {
            key: "item.test.sword".to_string(),
            fallback: Some("测试剑".to_string()),
            args: vec![
                Component::text("甲"),
                Component::Text {
                    text: "乙".to_string(),
                    style: Style::with_color(0xFF_11_22_33),
                },
            ],
        }
    }

    #[test]
    fn empty_roundtrip() {
        let c = Component::Empty;
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
        assert_eq!(c.to_nbt(), NbtTag::Compound(vec![]));
    }

    #[test]
    fn text_roundtrip() {
        let c = styled_text();
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
        // ARGB 颜色必须无损：0xFF0000FF 经 NBT Int 位模式往返后仍相等。
        assert_eq!(back, styled_text());
    }

    #[test]
    fn text_all_style_fields_roundtrip() {
        // 覆盖 Style 全部 9 个字段的 roundtrip。
        let c = Component::Text {
            text: "全样式".to_string(),
            style: Style {
                color: Some(0x80_12_34_56),
                bold: true,
                italic: true,
                underlined: true,
                strikethrough: true,
                obfuscated: true,
                insertion: Some("shift-click".to_string()),
                click_event: Some(ClickEvent {
                    action: ClickAction::RunCommand,
                    value: "/give @p diamond".to_string(),
                }),
                hover_event: Some(HoverEvent {
                    action: HoverAction::ShowText,
                    value: "悬停提示".to_string(),
                }),
            },
        };
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn text_without_style_roundtrip() {
        let c = Component::text("hello");
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn legacy_text_bytes_compatible() {
        // 旧版 `Component::text("x")` 的 NBT 与 T1 版一致：仅 `{text: "x"}`。
        let nbt = Component::text("x").to_nbt();
        assert_eq!(
            nbt,
            NbtTag::Compound(vec![("text".to_string(), NbtTag::String("x".to_string()))])
        );
        // 带 color + italic + bold 的旧式 Text 字节序列保持 `text`→`color`→`italic`
        // →`bold` 顺序不变（新增样式字段仅在尾部追加）。
        let styled = Component::Text {
            text: "x".to_string(),
            style: Style {
                color: Some(0xFF_00_00_FF),
                italic: true,
                bold: true,
                ..Style::default()
            },
        };
        let expected = NbtTag::Compound(vec![
            ("text".to_string(), NbtTag::String("x".to_string())),
            ("color".to_string(), NbtTag::Int(-16_776_961)), // 0xFF0000FF 位模式
            ("italic".to_string(), NbtTag::Byte(1)),
            ("bold".to_string(), NbtTag::Byte(1)),
        ]);
        assert_eq!(styled.to_nbt(), expected);
    }

    #[test]
    fn translatable_roundtrip() {
        let c = translatable();
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn translatable_without_fallback_roundtrip() {
        let c = Component::Translatable {
            key: "gui.merchant".to_string(),
            fallback: None,
            args: vec![],
        };
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn keybind_roundtrip() {
        let c = Component::Keybind {
            key: "key.attack".to_string(),
        };
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
        assert_eq!(
            c.to_nbt(),
            NbtTag::Compound(vec![(
                "keybind".to_string(),
                NbtTag::String("key.attack".into())
            )])
        );
    }

    #[test]
    fn scoreboard_roundtrip() {
        let c = Component::Scoreboard {
            name: "Steve".to_string(),
            objective: "kills".to_string(),
        };
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn selector_roundtrip() {
        let with_sep = Component::Selector {
            pattern: "@e[type=minecraft:cow]".to_string(),
            separator: Some(", ".to_string()),
        };
        let back = Component::from_nbt(&with_sep.to_nbt()).unwrap();
        assert_eq!(back, with_sep);

        let no_sep = Component::Selector {
            pattern: "@p".to_string(),
            separator: None,
        };
        let back = Component::from_nbt(&no_sep.to_nbt()).unwrap();
        assert_eq!(back, no_sep);
    }

    #[test]
    fn nbt_component_roundtrip() {
        let interpret_true = Component::Nbt {
            nbt_path: "Pos[0]".to_string(),
            interpret: true,
            separator: Some("/".to_string()),
        };
        let back = Component::from_nbt(&interpret_true.to_nbt()).unwrap();
        assert_eq!(back, interpret_true);

        let literal = Component::Nbt {
            nbt_path: "custom:data".to_string(),
            interpret: false,
            separator: None,
        };
        let back = Component::from_nbt(&literal.to_nbt()).unwrap();
        assert_eq!(back, literal);
    }

    #[test]
    fn plain_text_concat() {
        // Translatable：fallback + 各参数纯文本拼接。
        assert_eq!(translatable().plain_text(), "测试剑甲乙");
        // 无 fallback 时退回翻译键。
        let no_fb = Component::Translatable {
            key: "item.test".to_string(),
            fallback: None,
            args: vec![Component::text("!"), Component::text("?")],
        };
        assert_eq!(no_fb.plain_text(), "item.test!?");
        assert_eq!(Component::text("仅文本").plain_text(), "仅文本");
        assert_eq!(Component::Empty.plain_text(), "");
    }

    #[test]
    fn plain_text_placeholders() {
        // Keybind → 占位 `{key}`（无翻译上下文，文档注明占位语义）。
        assert_eq!(
            Component::Keybind {
                key: "key.attack".to_string()
            }
            .plain_text(),
            "{key}"
        );
        // Scoreboard → name。
        assert_eq!(
            Component::Scoreboard {
                name: "Steve".to_string(),
                objective: "kills".to_string(),
            }
            .plain_text(),
            "Steve"
        );
        // Selector → pattern。
        assert_eq!(
            Component::Selector {
                pattern: "@a".to_string(),
                separator: None,
            }
            .plain_text(),
            "@a"
        );
        // Nbt → 空串（依赖运行时数据，占位）。
        assert_eq!(
            Component::Nbt {
                nbt_path: "Pos[0]".to_string(),
                interpret: false,
                separator: None,
            }
            .plain_text(),
            ""
        );
        // 嵌套拼接：Translatable 参数中含 Keybind / Selector。
        let nested = Component::Translatable {
            key: "x".to_string(),
            fallback: Some("前".to_string()),
            args: vec![
                Component::Keybind {
                    key: "key.jump".to_string(),
                },
                Component::Selector {
                    pattern: "@e".to_string(),
                    separator: None,
                },
                Component::text("后"),
            ],
        };
        assert_eq!(nested.plain_text(), "前{key}@e后");
    }

    #[test]
    fn color_from_name_16_colors() {
        let cases = [
            ("black", 0xFF_00_00_00),
            ("dark_blue", 0xFF_00_00_AA),
            ("dark_green", 0xFF_00_AA_00),
            ("dark_aqua", 0xFF_00_AA_AA),
            ("dark_red", 0xFF_AA_00_00),
            ("dark_purple", 0xFF_AA_00_AA),
            ("gold", 0xFF_FF_AA_00),
            ("gray", 0xFF_AA_AA_AA),
            ("dark_gray", 0xFF_55_55_55),
            ("blue", 0xFF_55_55_FF),
            ("green", 0xFF_55_FF_55),
            ("aqua", 0xFF_55_FF_FF),
            ("red", 0xFF_FF_55_55),
            ("light_purple", 0xFF_FF_55_FF),
            ("yellow", 0xFF_FF_FF_55),
            ("white", 0xFF_FF_FF_FF),
        ];
        for (name, argb) in cases {
            let c = Color::from_name(name).unwrap_or_else(|| panic!("颜色名 {name} 未识别"));
            assert_eq!(c.0, argb, "颜色名 {name} 的 ARGB 不符");
        }
        assert_eq!(Color::from_name("not_a_color"), None);
        assert_eq!(Color::from_name(""), None);
    }

    #[test]
    fn color_nbt_roundtrip() {
        let c = Color(0x80_FF_00_FF);
        let back = Color::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
        // 非 TAG_Int 输入报错。
        assert_eq!(
            Color::from_nbt(&NbtTag::String("x".into())),
            Err(NbtError::InvalidListType)
        );
    }

    #[test]
    fn style_default_and_with_color() {
        assert_eq!(
            Style::default(),
            Style {
                color: None,
                bold: false,
                italic: false,
                underlined: false,
                strikethrough: false,
                obfuscated: false,
                insertion: None,
                click_event: None,
                hover_event: None,
            }
        );
        assert_eq!(
            Style::with_color(0xFF_00_FF_00),
            Style {
                color: Some(0xFF_00_FF_00),
                ..Style::default()
            }
        );
        assert_eq!(
            Style::with_insertion("i".to_string()),
            Style {
                insertion: Some("i".to_string()),
                ..Style::default()
            }
        );
    }

    #[test]
    fn component_holder_basic() {
        let c = Component::text("hi");
        assert_eq!(c.components(), vec![c.clone()]);
        let out = c.copy_with_operator(&|x| Component::text(&format!("[{}]", x.plain_text())));
        assert_eq!(out, Component::text("[hi]"));
    }

    #[test]
    fn malformed_nbt_rejected() {
        // 非 Compound 输入 → 结构错误。
        assert_eq!(
            Component::from_nbt(&NbtTag::Int(5)),
            Err(NbtError::InvalidListType)
        );
        assert_eq!(
            Component::from_nbt(&NbtTag::String("x".into())),
            Err(NbtError::InvalidListType)
        );
        // `text` 键存在但类型不是 String → 结构错误。
        let bad_text = NbtTag::Compound(vec![("text".into(), NbtTag::Int(7))]);
        assert_eq!(
            Component::from_nbt(&bad_text),
            Err(NbtError::InvalidListType)
        );
        // `translate` 键存在但 `with` 不是 List → 结构错误。
        let bad_with = NbtTag::Compound(vec![
            ("translate".into(), NbtTag::String("k".into())),
            ("with".into(), NbtTag::Int(1)),
        ]);
        assert_eq!(
            Component::from_nbt(&bad_with),
            Err(NbtError::InvalidListType)
        );
        // `color` 键存在但类型不是 Int → 结构错误。
        let bad_color = NbtTag::Compound(vec![
            ("text".into(), NbtTag::String("x".into())),
            ("color".into(), NbtTag::Byte(1)),
        ]);
        assert_eq!(
            Component::from_nbt(&bad_color),
            Err(NbtError::InvalidListType)
        );
        // `keybind` 键存在但类型不是 String → 结构错误。
        let bad_keybind = NbtTag::Compound(vec![("keybind".into(), NbtTag::Int(1))]);
        assert_eq!(
            Component::from_nbt(&bad_keybind),
            Err(NbtError::InvalidListType)
        );
        // `score` 键不是 Compound → 结构错误。
        let bad_score = NbtTag::Compound(vec![("score".into(), NbtTag::Int(1))]);
        assert_eq!(
            Component::from_nbt(&bad_score),
            Err(NbtError::InvalidListType)
        );
        // `score` 为 Compound 但缺 `objective` → 结构错误。
        let score_missing = NbtTag::Compound(vec![(
            "score".into(),
            NbtTag::Compound(vec![("name".into(), NbtTag::String("Steve".into()))]),
        )]);
        assert_eq!(
            Component::from_nbt(&score_missing),
            Err(NbtError::InvalidListType)
        );
        // `selector` 键存在但 `separator` 不是 String → 结构错误。
        let bad_separator = NbtTag::Compound(vec![
            ("selector".into(), NbtTag::String("@e".into())),
            ("separator".into(), NbtTag::Int(1)),
        ]);
        assert_eq!(
            Component::from_nbt(&bad_separator),
            Err(NbtError::InvalidListType)
        );
        // `nbt` 键不是 Compound → 结构错误。
        let bad_nbt = NbtTag::Compound(vec![("nbt".into(), NbtTag::String("Pos".into()))]);
        assert_eq!(
            Component::from_nbt(&bad_nbt),
            Err(NbtError::InvalidListType)
        );
        // `nbt` 为 Compound 但缺 `nbt_path` → 结构错误。
        let nbt_missing = NbtTag::Compound(vec![(
            "nbt".into(),
            NbtTag::Compound(vec![("interpret".into(), NbtTag::Byte(1))]),
        )]);
        assert_eq!(
            Component::from_nbt(&nbt_missing),
            Err(NbtError::InvalidListType)
        );
    }

    #[test]
    fn with_list_element_invalid_rejected() {
        // `with` 列表中混入非组件结构 → 结构错误。
        let bad = NbtTag::Compound(vec![
            ("translate".into(), NbtTag::String("k".into())),
            (
                "with".into(),
                NbtTag::List(vec![NbtTag::Int(1), NbtTag::String("x".into())]),
            ),
        ]);
        assert_eq!(Component::from_nbt(&bad), Err(NbtError::InvalidListType));
    }

    #[test]
    fn click_event_roundtrip() {
        let c = Component::click_event(ClickAction::RunCommand, "/give @p diamond");
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
        assert_eq!(
            c.to_nbt(),
            NbtTag::Compound(vec![(
                "click_event".to_string(),
                NbtTag::Compound(vec![
                    ("action".to_string(), NbtTag::Int(0)),
                    (
                        "value".to_string(),
                        NbtTag::String("/give @p diamond".to_string())
                    ),
                ])
            )])
        );
    }

    #[test]
    fn hover_event_roundtrip() {
        let c = Component::hover_event(HoverAction::ShowText, "悬停提示文本");
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
        assert_eq!(
            c.to_nbt(),
            NbtTag::Compound(vec![(
                "hover_event".to_string(),
                NbtTag::Compound(vec![
                    ("action".to_string(), NbtTag::Int(0)),
                    (
                        "value".to_string(),
                        NbtTag::String("悬停提示文本".to_string())
                    ),
                ])
            )])
        );
    }

    #[test]
    fn click_hover_all_actions_roundtrip() {
        for action in [
            ClickAction::RunCommand,
            ClickAction::OpenUrl,
            ClickAction::SuggestCommand,
            ClickAction::ChangePage,
        ] {
            let c = Component::click_event(action, "value");
            let back = Component::from_nbt(&c.to_nbt()).unwrap();
            assert_eq!(back, c, "ClickAction {:?} roundtrip failed", action);
        }
        for action in [
            HoverAction::ShowText,
            HoverAction::ShowEntity,
            HoverAction::ShowItem,
        ] {
            let c = Component::hover_event(action, "value");
            let back = Component::from_nbt(&c.to_nbt()).unwrap();
            assert_eq!(back, c, "HoverAction {:?} roundtrip failed", action);
        }
    }

    #[test]
    fn nested_click_event_in_translatable_roundtrip() {
        let c = Component::Translatable {
            key: "chat.link.confirm".to_string(),
            fallback: None,
            args: vec![Component::Text {
                text: "点击确认".to_string(),
                style: Style {
                    click_event: Some(ClickEvent {
                        action: ClickAction::OpenUrl,
                        value: "https://example.com".to_string(),
                    }),
                    ..Style::default()
                },
            }],
        };
        let back = Component::from_nbt(&c.to_nbt()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn malformed_click_hover_nbt_rejected() {
        // `click_event` 键不是 Compound → 结构错误。
        let bad_ce = NbtTag::Compound(vec![(
            "click_event".into(),
            NbtTag::String("invalid".into()),
        )]);
        assert_eq!(Component::from_nbt(&bad_ce), Err(NbtError::InvalidListType));
        // `hover_event` 键不是 Compound → 结构错误。
        let bad_he = NbtTag::Compound(vec![(
            "hover_event".into(),
            NbtTag::String("invalid".into()),
        )]);
        assert_eq!(Component::from_nbt(&bad_he), Err(NbtError::InvalidListType));
        // `click_event` 为 Compound 但缺 `action` → 结构错误。
        let ce_missing_action = NbtTag::Compound(vec![(
            "click_event".into(),
            NbtTag::Compound(vec![("value".into(), NbtTag::String("x".into()))]),
        )]);
        assert_eq!(
            Component::from_nbt(&ce_missing_action),
            Err(NbtError::InvalidListType)
        );
        // `click_event` 为 Compound 但 `action` 非法 i32 → 结构错误。
        let ce_bad_action = NbtTag::Compound(vec![(
            "click_event".into(),
            NbtTag::Compound(vec![
                ("action".into(), NbtTag::Int(99)),
                ("value".into(), NbtTag::String("x".into())),
            ]),
        )]);
        assert_eq!(
            Component::from_nbt(&ce_bad_action),
            Err(NbtError::InvalidListType)
        );
        // `hover_event` 为 Compound 但缺 `value` → 结构错误。
        let he_missing_value = NbtTag::Compound(vec![(
            "hover_event".into(),
            NbtTag::Compound(vec![("action".into(), NbtTag::Int(0))]),
        )]);
        assert_eq!(
            Component::from_nbt(&he_missing_value),
            Err(NbtError::InvalidListType)
        );
        // Text 中 `click_event` 键存在但类型不是 Compound → 结构错误。
        let bad_text_ce = NbtTag::Compound(vec![
            ("text".into(), NbtTag::String("x".into())),
            ("click_event".into(), NbtTag::String("y".into())),
        ]);
        assert_eq!(
            Component::from_nbt(&bad_text_ce),
            Err(NbtError::InvalidListType)
        );
    }

    #[test]
    fn click_hover_plain_text() {
        assert_eq!(
            Component::click_event(ClickAction::RunCommand, "/cmd").plain_text(),
            ""
        );
        assert_eq!(
            Component::hover_event(HoverAction::ShowText, "hint").plain_text(),
            ""
        );
    }

    #[test]
    fn click_action_from_code() {
        assert_eq!(ClickAction::from_code(0), Some(ClickAction::RunCommand));
        assert_eq!(ClickAction::from_code(1), Some(ClickAction::OpenUrl));
        assert_eq!(ClickAction::from_code(2), Some(ClickAction::SuggestCommand));
        assert_eq!(ClickAction::from_code(3), Some(ClickAction::ChangePage));
        assert_eq!(ClickAction::from_code(4), None);
        assert_eq!(ClickAction::from_code(-1), None);
    }

    #[test]
    fn hover_action_from_code() {
        assert_eq!(HoverAction::from_code(0), Some(HoverAction::ShowText));
        assert_eq!(HoverAction::from_code(1), Some(HoverAction::ShowEntity));
        assert_eq!(HoverAction::from_code(2), Some(HoverAction::ShowItem));
        assert_eq!(HoverAction::from_code(3), None);
        assert_eq!(HoverAction::from_code(-1), None);
    }
}
