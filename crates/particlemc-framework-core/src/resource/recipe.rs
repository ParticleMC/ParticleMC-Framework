// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 配方 API（框架层，见 `.specs/complete-partial-framework-capabilities/` R6）。
//!
//! 提供 1.21.11 真实线格式的四类值类型 [`RecipeProperty`] / [`Ingredient`] /
//! [`SlotDisplay`] / [`RecipeDisplay`]，以及 [`RecipeManager`] `Resource` 与
//! [`RecipeManager::to_bytes`]（产出可直接装入 `DeclareRecipes`(0x83) 包体的字节）。
//!
//! ## 线格式（1.21.11，对齐 Java `DeclareRecipesPacket`）
//!
//! `to_bytes` 输出两段：
//!
//! 1. `item_properties`：VarInt 计数，每条为 `RecipeProperty`（字符串 key）+ VarInt
//!    计数的 material id 列表；
//! 2. `stonecutter_recipes`：VarInt 计数，每条为 `Ingredient` + `SlotDisplay`。
//!
//! [`Ingredient`]：`Items` 写 `len + 1` 后跟各 material varint（`0` 表示 tag 名随后，
//! 对齐 Java `Ingredient.NETWORK_TYPE` 的 `+1` 约定）；`Tag` 写 `0` 后跟 tag key 字符串。
//! [`SlotDisplay`] / [`RecipeDisplay`] 均为「VarInt 类型 id + 负载」，类型 id 对齐 Java
//! `SlotDisplayType` / `RecipeDisplayType` 序位；`ItemStack` 复用
//! [`encode_item_stack`](crate::item_stack::encode_item_stack) / `decode_item_stack` 线格式。
//!
//! 解码失败（未知类型 id / 计数越界 / 数据不足）一律返回
//! [`ProtocolError`](crate::protocol::error::ProtocolError)，不 panic。
//!
//! ## 旧 `Recipe` 枚举（应用侧定义）
//!
//! 旧 5 种 [`Recipe`] 保留为「应用侧配方定义」（注册 / 注销 / 查询 API 不变）。
//! `to_bytes` 仅编码新 API（[`add_item_property`](RecipeManager::add_item_property) /
//! [`add_stonecutter_recipe`](RecipeManager::add_stonecutter_recipe)）添加的数据；
//! 其中旧 [`Recipe::Stonecutting`] 配方会被映射进 `stonecutter_recipes`
//! （ingredient→[`Ingredient::Items`]，result→[`SlotDisplay::ItemStack`]），
//! 其余类型无 1.21.11 对应字段，由应用侧经 `add_item_property` 表达。

use crate::item_stack::{ItemStack, decode_item_stack, encode_item_stack};
use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;

/// 配方属性 key（对齐 Java `RecipeProperty`，线格式为无命名空间的字符串 key）。
///
/// 七个变体对应 vanilla 配方书分类：锻造（基底 / 模板 / 附加物）与熔炉 /
/// 高炉 / 烟熏炉 / 营火的输入。每个变体可携带 `Option<String>` 自定义 category
/// 值：`Some(key)` 编码时优先使用该 key，`None` 使用 vanilla 默认 key。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeProperty {
    /// `smithing_base`：锻造基底。
    SmithingBase(Option<String>),
    /// `smithing_template`：锻造模板。
    SmithingTemplate(Option<String>),
    /// `smithing_addition`：锻造附加物。
    SmithingAddition(Option<String>),
    /// `furnace_input`：熔炉输入。
    FurnaceInput(Option<String>),
    /// `blast_furnace_input`：高炉输入。
    BlastFurnaceInput(Option<String>),
    /// `smoker_input`：烟熏炉输入。
    SmokerInput(Option<String>),
    /// `campfire_input`：营火输入。
    CampfireInput(Option<String>),
}

impl RecipeProperty {
    /// 该变体的 vanilla 默认 key（对齐 Java `RecipeProperty` 构造参数）。
    pub fn default_key(&self) -> &'static str {
        match self {
            RecipeProperty::SmithingBase(_) => "smithing_base",
            RecipeProperty::SmithingTemplate(_) => "smithing_template",
            RecipeProperty::SmithingAddition(_) => "smithing_addition",
            RecipeProperty::FurnaceInput(_) => "furnace_input",
            RecipeProperty::BlastFurnaceInput(_) => "blast_furnace_input",
            RecipeProperty::SmokerInput(_) => "smoker_input",
            RecipeProperty::CampfireInput(_) => "campfire_input",
        }
    }

    /// 线格式 key：`Some(custom)` 优先，否则为 vanilla 默认 key。
    pub fn key(&self) -> &str {
        match self {
            RecipeProperty::SmithingBase(Some(k)) => k,
            RecipeProperty::SmithingTemplate(Some(k)) => k,
            RecipeProperty::SmithingAddition(Some(k)) => k,
            RecipeProperty::FurnaceInput(Some(k)) => k,
            RecipeProperty::BlastFurnaceInput(Some(k)) => k,
            RecipeProperty::SmokerInput(Some(k)) => k,
            RecipeProperty::CampfireInput(Some(k)) => k,
            _ => self.default_key(),
        }
    }

    /// 编码为字符串 key（对齐 Java `RecipeProperty.NETWORK_TYPE`）。
    pub fn encode(&self, buf: &mut ByteBuffer) {
        buf.put_string(self.key());
    }

    /// 解码字符串 key；未知 key（含自定义 `Some` 值）返回
    /// [`ProtocolError::InvalidValue`]（自定义 key 仅编码侧扩展）。
    pub fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let key = buf.get_string()?;
        match key.as_str() {
            "smithing_base" => Ok(RecipeProperty::SmithingBase(None)),
            "smithing_template" => Ok(RecipeProperty::SmithingTemplate(None)),
            "smithing_addition" => Ok(RecipeProperty::SmithingAddition(None)),
            "furnace_input" => Ok(RecipeProperty::FurnaceInput(None)),
            "blast_furnace_input" => Ok(RecipeProperty::BlastFurnaceInput(None)),
            "smoker_input" => Ok(RecipeProperty::SmokerInput(None)),
            "campfire_input" => Ok(RecipeProperty::CampfireInput(None)),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

/// 配方原料（对齐 Java `Ingredient` 线格式）。
///
/// 线格式为「VarInt 计数 + 数据」：`Items` 写 `len + 1` 后跟各 material varint，
/// `Tag` 写 `0` 后跟 tag key 字符串（对齐 Java `Ingredient.NETWORK_TYPE` 的 `+1`
/// 约定与「0 表示 tag 名随后」注释）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingredient {
    /// material id 列表。
    Items(Vec<u32>),
    /// item tag key（如 `minecraft:planks`）。
    Tag(String),
}

impl Ingredient {
    /// 编码为 1.21.11 线格式。
    pub fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match self {
            Ingredient::Items(items) => {
                let len = i32::try_from(items.len()).map_err(|_| ProtocolError::InvalidValue)?;
                let count = len.checked_add(1).ok_or(ProtocolError::InvalidValue)?;
                buf.put_varint(count);
                for m in items {
                    let m = i32::try_from(*m).map_err(|_| ProtocolError::InvalidValue)?;
                    buf.put_varint(m);
                }
            }
            Ingredient::Tag(key) => {
                buf.put_varint(0);
                buf.put_string(key);
            }
        }
        Ok(())
    }

    /// 解码 1.21.11 线格式；计数为负返回
    /// [`ProtocolError::InvalidValue`]。
    pub fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let count = buf.get_varint()?;
        if count == 0 {
            return Ok(Ingredient::Tag(buf.get_string()?));
        }
        if count < 0 {
            return Err(ProtocolError::InvalidValue);
        }
        let n = usize::try_from(count - 1).map_err(|_| ProtocolError::InvalidValue)?;
        let mut items = Vec::with_capacity(n);
        for _ in 0..n {
            let m = u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?;
            items.push(m);
        }
        Ok(Ingredient::Items(items))
    }
}

/// 槽位显示（对齐 Java `SlotDisplay` 的 `SlotDisplayType` 序位）。
///
/// 线格式为「VarInt 类型 id + 负载」：类型 id 0..=7 对应 `Empty` / `AnyFuel` /
/// `Item` / `ItemStack` / `Tag` / `SmithingTrim` / `WithRemainder` / `Composite`。
#[derive(Debug, Clone, PartialEq)]
pub enum SlotDisplay {
    /// 空槽位。
    Empty,
    /// 任意燃料。
    AnyFuel,
    /// 物品（material id）。
    Item(u32),
    /// 带组件的物品栈。
    ItemStack(ItemStack),
    /// 物品 tag key。
    Tag(String),
    /// 锻造纹饰：基底 + 纹饰材料 + 纹饰图案。
    SmithingTrim(Box<SlotDisplay>, Box<SlotDisplay>, Box<SlotDisplay>),
    /// 带剩余物：输入 + 剩余物。
    WithRemainder(Box<SlotDisplay>, Box<SlotDisplay>),
    /// 组合显示（子显示列表）。
    Composite(Vec<SlotDisplay>),
}

impl SlotDisplay {
    /// 编码为 1.21.11 线格式（类型 id + 负载）。
    pub fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match self {
            SlotDisplay::Empty => buf.put_varint(0),
            SlotDisplay::AnyFuel => buf.put_varint(1),
            SlotDisplay::Item(m) => {
                buf.put_varint(2);
                let m = i32::try_from(*m).map_err(|_| ProtocolError::InvalidValue)?;
                buf.put_varint(m);
            }
            SlotDisplay::ItemStack(item) => {
                buf.put_varint(3);
                encode_item_stack(item, buf)?;
            }
            SlotDisplay::Tag(key) => {
                buf.put_varint(4);
                buf.put_string(key);
            }
            SlotDisplay::SmithingTrim(base, material, pattern) => {
                buf.put_varint(5);
                base.encode(buf)?;
                material.encode(buf)?;
                pattern.encode(buf)?;
            }
            SlotDisplay::WithRemainder(input, remainder) => {
                buf.put_varint(6);
                input.encode(buf)?;
                remainder.encode(buf)?;
            }
            SlotDisplay::Composite(contents) => {
                buf.put_varint(7);
                let count =
                    i32::try_from(contents.len()).map_err(|_| ProtocolError::InvalidValue)?;
                buf.put_varint(count);
                for d in contents {
                    d.encode(buf)?;
                }
            }
        }
        Ok(())
    }

    /// 解码 1.21.11 线格式；未知类型 id / 非法计数返回
    /// [`ProtocolError::InvalidValue`]。
    pub fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        match buf.get_varint()? {
            0 => Ok(SlotDisplay::Empty),
            1 => Ok(SlotDisplay::AnyFuel),
            2 => {
                let m =
                    u32::try_from(buf.get_varint()?).map_err(|_| ProtocolError::InvalidValue)?;
                Ok(SlotDisplay::Item(m))
            }
            3 => Ok(SlotDisplay::ItemStack(decode_item_stack(buf)?)),
            4 => Ok(SlotDisplay::Tag(buf.get_string()?)),
            5 => Ok(SlotDisplay::SmithingTrim(
                Box::new(SlotDisplay::decode(buf)?),
                Box::new(SlotDisplay::decode(buf)?),
                Box::new(SlotDisplay::decode(buf)?),
            )),
            6 => Ok(SlotDisplay::WithRemainder(
                Box::new(SlotDisplay::decode(buf)?),
                Box::new(SlotDisplay::decode(buf)?),
            )),
            7 => {
                let count = buf.get_varint()?;
                if count < 0 {
                    return Err(ProtocolError::InvalidValue);
                }
                let count = usize::try_from(count).map_err(|_| ProtocolError::InvalidValue)?;
                let mut contents = Vec::with_capacity(count);
                for _ in 0..count {
                    contents.push(SlotDisplay::decode(buf)?);
                }
                Ok(SlotDisplay::Composite(contents))
            }
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

/// 配方显示（对齐 Java `RecipeDisplay` 的 `RecipeDisplayType` 序位）。
///
/// 线格式为「VarInt 类型 id + 负载」：类型 id 0..=4 对应 `CraftingShapeless` /
/// `CraftingShaped` / `Furnace` / `Stonecutter` / `Smithing`。`CraftingShaped`
/// 构造与解码时校验 `ingredients.len() == width * height`。
#[derive(Debug, Clone, PartialEq)]
pub enum RecipeDisplay {
    /// 无序合成：原料列表 + 结果 + 工作台。
    CraftingShapeless {
        /// 原料列表。
        ingredients: Vec<SlotDisplay>,
        /// 结果。
        result: SlotDisplay,
        /// 工作台显示。
        crafting_station: SlotDisplay,
    },
    /// 定形合成：宽高 + 原料列表 + 结果 + 工作台。
    CraftingShaped {
        /// 图案宽度。
        width: i32,
        /// 图案高度。
        height: i32,
        /// 原料列表（`len == width * height`）。
        ingredients: Vec<SlotDisplay>,
        /// 结果。
        result: SlotDisplay,
        /// 工作台显示。
        crafting_station: SlotDisplay,
    },
    /// 熔炉：原料 + 燃料 + 结果 + 工作台 + 时长 + 经验。
    Furnace {
        /// 原料。
        ingredient: SlotDisplay,
        /// 燃料。
        fuel: SlotDisplay,
        /// 结果。
        result: SlotDisplay,
        /// 工作台显示。
        crafting_station: SlotDisplay,
        /// 烹饪时长（tick）。
        duration: i32,
        /// 经验值。
        experience: f32,
    },
    /// 切石机：原料 + 结果 + 工作台。
    Stonecutter {
        /// 原料。
        ingredient: SlotDisplay,
        /// 结果。
        result: SlotDisplay,
        /// 工作台显示。
        crafting_station: SlotDisplay,
    },
    /// 锻造：模板 + 基底 + 附加物 + 结果 + 工作台。
    Smithing {
        /// 模板。
        template: SlotDisplay,
        /// 基底。
        base: SlotDisplay,
        /// 附加物。
        addition: SlotDisplay,
        /// 结果。
        result: SlotDisplay,
        /// 工作台显示。
        crafting_station: SlotDisplay,
    },
}

impl RecipeDisplay {
    /// 构造定形合成；原料数 ≠ 宽×高（含负数 / 乘法越界）返回
    /// [`RecipeError::InvalidShapedDimensions`]。
    pub fn crafting_shaped(
        width: i32,
        height: i32,
        ingredients: Vec<SlotDisplay>,
        result: SlotDisplay,
        crafting_station: SlotDisplay,
    ) -> Result<Self, RecipeError> {
        if check_shaped_dimensions(width, height, ingredients.len()).is_err() {
            return Err(RecipeError::InvalidShapedDimensions);
        }
        Ok(RecipeDisplay::CraftingShaped {
            width,
            height,
            ingredients,
            result,
            crafting_station,
        })
    }

    /// 编码为 1.21.11 线格式（类型 id + 负载）。
    pub fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        match self {
            RecipeDisplay::CraftingShapeless {
                ingredients,
                result,
                crafting_station,
            } => {
                buf.put_varint(0);
                let count =
                    i32::try_from(ingredients.len()).map_err(|_| ProtocolError::InvalidValue)?;
                buf.put_varint(count);
                for d in ingredients {
                    d.encode(buf)?;
                }
                result.encode(buf)?;
                crafting_station.encode(buf)?;
            }
            RecipeDisplay::CraftingShaped {
                width,
                height,
                ingredients,
                result,
                crafting_station,
            } => {
                buf.put_varint(1);
                buf.put_varint(*width);
                buf.put_varint(*height);
                let count =
                    i32::try_from(ingredients.len()).map_err(|_| ProtocolError::InvalidValue)?;
                buf.put_varint(count);
                for d in ingredients {
                    d.encode(buf)?;
                }
                result.encode(buf)?;
                crafting_station.encode(buf)?;
            }
            RecipeDisplay::Furnace {
                ingredient,
                fuel,
                result,
                crafting_station,
                duration,
                experience,
            } => {
                buf.put_varint(2);
                ingredient.encode(buf)?;
                fuel.encode(buf)?;
                result.encode(buf)?;
                crafting_station.encode(buf)?;
                buf.put_varint(*duration);
                buf.put_f32(*experience);
            }
            RecipeDisplay::Stonecutter {
                ingredient,
                result,
                crafting_station,
            } => {
                buf.put_varint(3);
                ingredient.encode(buf)?;
                result.encode(buf)?;
                crafting_station.encode(buf)?;
            }
            RecipeDisplay::Smithing {
                template,
                base,
                addition,
                result,
                crafting_station,
            } => {
                buf.put_varint(4);
                template.encode(buf)?;
                base.encode(buf)?;
                addition.encode(buf)?;
                result.encode(buf)?;
                crafting_station.encode(buf)?;
            }
        }
        Ok(())
    }

    /// 解码 1.21.11 线格式；未知类型 id / `CraftingShaped` 尺寸不符返回
    /// [`ProtocolError::InvalidValue`]。
    pub fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        match buf.get_varint()? {
            0 => {
                let count = buf.get_varint()?;
                if count < 0 {
                    return Err(ProtocolError::InvalidValue);
                }
                let count = usize::try_from(count).map_err(|_| ProtocolError::InvalidValue)?;
                let mut ingredients = Vec::with_capacity(count);
                for _ in 0..count {
                    ingredients.push(SlotDisplay::decode(buf)?);
                }
                let result = SlotDisplay::decode(buf)?;
                let crafting_station = SlotDisplay::decode(buf)?;
                Ok(RecipeDisplay::CraftingShapeless {
                    ingredients,
                    result,
                    crafting_station,
                })
            }
            1 => {
                let width = buf.get_varint()?;
                let height = buf.get_varint()?;
                let count = buf.get_varint()?;
                if count < 0 {
                    return Err(ProtocolError::InvalidValue);
                }
                let count = usize::try_from(count).map_err(|_| ProtocolError::InvalidValue)?;
                let mut ingredients = Vec::with_capacity(count);
                for _ in 0..count {
                    ingredients.push(SlotDisplay::decode(buf)?);
                }
                let result = SlotDisplay::decode(buf)?;
                let crafting_station = SlotDisplay::decode(buf)?;
                check_shaped_dimensions(width, height, ingredients.len())?;
                Ok(RecipeDisplay::CraftingShaped {
                    width,
                    height,
                    ingredients,
                    result,
                    crafting_station,
                })
            }
            2 => {
                let ingredient = SlotDisplay::decode(buf)?;
                let fuel = SlotDisplay::decode(buf)?;
                let result = SlotDisplay::decode(buf)?;
                let crafting_station = SlotDisplay::decode(buf)?;
                let duration = buf.get_varint()?;
                let experience = buf.get_f32()?;
                Ok(RecipeDisplay::Furnace {
                    ingredient,
                    fuel,
                    result,
                    crafting_station,
                    duration,
                    experience,
                })
            }
            3 => {
                let ingredient = SlotDisplay::decode(buf)?;
                let result = SlotDisplay::decode(buf)?;
                let crafting_station = SlotDisplay::decode(buf)?;
                Ok(RecipeDisplay::Stonecutter {
                    ingredient,
                    result,
                    crafting_station,
                })
            }
            4 => {
                let template = SlotDisplay::decode(buf)?;
                let base = SlotDisplay::decode(buf)?;
                let addition = SlotDisplay::decode(buf)?;
                let result = SlotDisplay::decode(buf)?;
                let crafting_station = SlotDisplay::decode(buf)?;
                Ok(RecipeDisplay::Smithing {
                    template,
                    base,
                    addition,
                    result,
                    crafting_station,
                })
            }
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

/// 校验定形配方原料数 == 宽×高（含负数 / 乘法越界防护）。
fn check_shaped_dimensions(
    width: i32,
    height: i32,
    ingredients_len: usize,
) -> Result<(), ProtocolError> {
    if width < 0 || height < 0 {
        return Err(ProtocolError::InvalidValue);
    }
    let Some(expected) = width.checked_mul(height) else {
        return Err(ProtocolError::InvalidValue);
    };
    let actual = i32::try_from(ingredients_len).map_err(|_| ProtocolError::InvalidValue)?;
    if actual != expected {
        return Err(ProtocolError::InvalidValue);
    }
    Ok(())
}

/// 切石机配方（`DeclareRecipes` 的 `stonecutter_recipes` 条目）。
///
/// 线格式为 [`Ingredient`] + [`SlotDisplay`]（option_display），对齐 Java
/// `DeclareRecipesPacket.StonecutterRecipe`。
#[derive(Debug, Clone, PartialEq)]
pub struct StonecutterRecipe {
    /// 原料。
    pub ingredient: Ingredient,
    /// 选项显示（产物展示）。
    pub option_display: SlotDisplay,
}

impl StonecutterRecipe {
    /// 编码为 1.21.11 线格式（`Ingredient` + `SlotDisplay`）。
    pub fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        self.ingredient.encode(buf)?;
        self.option_display.encode(buf)
    }

    /// 解码 1.21.11 线格式。
    pub fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(StonecutterRecipe {
            ingredient: Ingredient::decode(buf)?,
            option_display: SlotDisplay::decode(buf)?,
        })
    }
}

/// 配方值类型（五种形态，应用侧定义）。
///
/// 对应 Minestom 的 `ShapedRecipe` / `ShapelessRecipe` / `SmeltingRecipe` /
/// `StonecuttingRecipe` / `SmithingRecipe`（见 `.specs/complete-partial-framework-capabilities/`
/// R6）。`id` 在 `RecipeManager` 内唯一。
///
/// **边界**：v1 中 `to_bytes` 仅把 [`Recipe::Stonecutting`] 映射进
/// `stonecutter_recipes`，其余类型需经 [`RecipeManager::add_item_property`] 表达。
#[derive(Debug, Clone, PartialEq)]
pub enum Recipe {
    /// 合成台定形配方。
    Shaped {
        /// 配方 id。
        id: String,
        /// 图案宽度。
        width: i32,
        /// 图案高度。
        height: i32,
        /// 图案行（每行 `width` 个字符）。
        pattern: Vec<String>,
        /// 产物。
        result: ItemStack,
    },
    /// 合成台无序配方。
    Shapeless {
        /// 配方 id。
        id: String,
        /// 原料列表。
        ingredients: Vec<ItemStack>,
        /// 产物。
        result: ItemStack,
    },
    /// 熔炉配方。
    Smelting {
        /// 配方 id。
        id: String,
        /// 原料。
        ingredient: ItemStack,
        /// 产物。
        result: ItemStack,
        /// 经验值。
        experience: f32,
        /// 烹饪时长（tick）。
        cooking_time: i32,
    },
    /// 切石机配方。
    Stonecutting {
        /// 配方 id。
        id: String,
        /// 原料。
        ingredient: ItemStack,
        /// 产物。
        result: ItemStack,
    },
    /// 锻造台配方。
    Smithing {
        /// 配方 id。
        id: String,
        /// 基础物品。
        base: ItemStack,
        /// 附加物。
        addition: ItemStack,
        /// 产物。
        result: ItemStack,
    },
}

impl Recipe {
    /// 返回该配方的 id（五种形态统一取 `id` 字段）。
    pub fn id(&self) -> &str {
        match self {
            Recipe::Shaped { id, .. }
            | Recipe::Shapeless { id, .. }
            | Recipe::Smelting { id, .. }
            | Recipe::Stonecutting { id, .. }
            | Recipe::Smithing { id, .. } => id,
        }
    }
}

/// 配方管理器（旧 ECS 方案 `Resource`）。
///
/// 由 [`crate::plugin::McServerPlugin`] 装配或在应用侧自行插入。持有三类数据：
/// 应用侧旧配方（[`register`](RecipeManager::register)）、切石机配方
/// （[`add_stonecutter_recipe`](RecipeManager::add_stonecutter_recipe)）与
/// 配方属性（[`add_item_property`](RecipeManager::add_item_property)）。
#[derive(Default)]
pub struct RecipeManager {
    /// 已注册应用侧配方（保持注册顺序）。
    recipes: Vec<Recipe>,
    /// 切石机配方（保持添加顺序）。
    stonecutter_recipes: Vec<StonecutterRecipe>,
    /// 配方属性（保持添加顺序）。
    item_properties: Vec<(RecipeProperty, Vec<u32>)>,
}

impl RecipeManager {
    /// 注册一个应用侧配方；id 重复返回 [`RecipeError::DuplicateId`]。
    pub fn register(&mut self, r: Recipe) -> Result<(), RecipeError> {
        if self.recipes.iter().any(|x| x.id() == r.id()) {
            return Err(RecipeError::DuplicateId(r.id().to_string()));
        }
        self.recipes.push(r);
        Ok(())
    }

    /// 注销一个应用侧配方，返回是否存在。
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.recipes.len();
        self.recipes.retain(|r| r.id() != id);
        self.recipes.len() != before
    }

    /// 全部已注册应用侧配方（只读）。
    pub fn all(&self) -> &[Recipe] {
        &self.recipes
    }

    /// 添加一条切石机配方（进入 `to_bytes` 的 `stonecutter_recipes`）。
    pub fn add_stonecutter_recipe(&mut self, ingredient: Ingredient, option_display: SlotDisplay) {
        self.stonecutter_recipes.push(StonecutterRecipe {
            ingredient,
            option_display,
        });
    }

    /// 添加一条配方属性：category → material id 列表（进入 `to_bytes` 的
    /// `item_properties`）。
    pub fn add_item_property(&mut self, property: RecipeProperty, materials: Vec<u32>) {
        self.item_properties.push((property, materials));
    }

    /// 全部切石机配方（只读）。
    pub fn stonecutter_recipes(&self) -> &[StonecutterRecipe] {
        &self.stonecutter_recipes
    }

    /// 全部配方属性（只读，保持添加顺序）。
    pub fn item_properties(&self) -> &[(RecipeProperty, Vec<u32>)] {
        &self.item_properties
    }

    /// 编码为 1.21.11 真实线格式（可直接装入 `DeclareRecipes`(0x83) 包体）。
    ///
    /// 输出两段：`item_properties`（VarInt 计数，每条 `RecipeProperty` + VarInt
    /// 计数的 material 列表）+ `stonecutter_recipes`（VarInt 计数，每条
    /// `Ingredient` + `SlotDisplay`）。切石机配方顺序为：新 API 添加的在前
    /// （添加顺序），旧 [`Recipe::Stonecutting`] 映射在后（注册顺序）。
    /// 单条编码失败（材料 id 越界等病理数据）时跳过该条，其余照常输出。
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = ByteBuffer::with_capacity(64);

        // 1) item_properties
        let mut prop_entries: Vec<Vec<u8>> = Vec::new();
        for (property, materials) in &self.item_properties {
            let mut scratch = ByteBuffer::with_capacity(8);
            property.encode(&mut scratch);
            if encode_material_list(materials, &mut scratch).is_ok() {
                prop_entries.push(scratch.into_inner());
            }
        }
        if let Ok(count) = i32::try_from(prop_entries.len()) {
            out.put_varint(count);
            for entry in prop_entries {
                out.put_bytes(&entry);
            }
        } else {
            out.put_varint(0);
        }

        // 2) stonecutter_recipes：新 API 在前，旧 Stonecutting 映射在后。
        let mut sc_entries: Vec<Vec<u8>> = Vec::new();
        for sr in &self.stonecutter_recipes {
            let mut scratch = ByteBuffer::with_capacity(8);
            if sr.encode(&mut scratch).is_ok() {
                sc_entries.push(scratch.into_inner());
            }
        }
        for r in &self.recipes {
            if let Recipe::Stonecutting {
                ingredient, result, ..
            } = r
            {
                let mapped = StonecutterRecipe {
                    ingredient: Ingredient::Items(vec![ingredient.material]),
                    option_display: SlotDisplay::ItemStack(result.clone()),
                };
                let mut scratch = ByteBuffer::with_capacity(8);
                if mapped.encode(&mut scratch).is_ok() {
                    sc_entries.push(scratch.into_inner());
                }
            }
        }
        if let Ok(count) = i32::try_from(sc_entries.len()) {
            out.put_varint(count);
            for entry in sc_entries {
                out.put_bytes(&entry);
            }
        } else {
            out.put_varint(0);
        }

        out.into_inner()
    }
}

/// 写出 material id 列表（VarInt 计数 + 各 material varint）。
fn encode_material_list(materials: &[u32], buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
    let count = i32::try_from(materials.len()).map_err(|_| ProtocolError::InvalidValue)?;
    buf.put_varint(count);
    for m in materials {
        let m = i32::try_from(*m).map_err(|_| ProtocolError::InvalidValue)?;
        buf.put_varint(m);
    }
    Ok(())
}

/// 配方操作错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeError {
    /// 配方 id 已注册。
    DuplicateId(String),
    /// 定形配方原料数 ≠ 宽×高（或尺寸非法）。
    InvalidShapedDimensions,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 测试辅助：SlotDisplay roundtrip。
    fn roundtrip_slot(d: &SlotDisplay) -> SlotDisplay {
        let mut buf = ByteBuffer::with_capacity(32);
        d.encode(&mut buf).unwrap();
        let mut buf = ByteBuffer::new(buf.into_inner());
        SlotDisplay::decode(&mut buf).unwrap()
    }

    /// 测试辅助：RecipeDisplay roundtrip。
    fn roundtrip_display(d: &RecipeDisplay) -> RecipeDisplay {
        let mut buf = ByteBuffer::with_capacity(32);
        d.encode(&mut buf).unwrap();
        let mut buf = ByteBuffer::new(buf.into_inner());
        RecipeDisplay::decode(&mut buf).unwrap()
    }

    /// 测试辅助：解码 `RecipeManager::to_bytes` 输出为两段。
    fn decode_manager_bytes(
        data: &[u8],
    ) -> (Vec<(RecipeProperty, Vec<u32>)>, Vec<StonecutterRecipe>) {
        let mut buf = ByteBuffer::new(data.to_vec());
        let props_count = buf.get_varint().unwrap();
        let mut props = Vec::new();
        for _ in 0..props_count {
            let property = RecipeProperty::decode(&mut buf).unwrap();
            let mcount = buf.get_varint().unwrap();
            let mut materials = Vec::new();
            for _ in 0..mcount {
                materials.push(u32::try_from(buf.get_varint().unwrap()).unwrap());
            }
            props.push((property, materials));
        }
        let sc_count = buf.get_varint().unwrap();
        let mut scs = Vec::new();
        for _ in 0..sc_count {
            scs.push(StonecutterRecipe::decode(&mut buf).unwrap());
        }
        (props, scs)
    }

    // ── SlotDisplay ──

    #[test]
    fn slot_display_empty_and_any_fuel() {
        assert_eq!(roundtrip_slot(&SlotDisplay::Empty), SlotDisplay::Empty);
        assert_eq!(roundtrip_slot(&SlotDisplay::AnyFuel), SlotDisplay::AnyFuel);
        // 类型 id：Empty=0, AnyFuel=1，无负载。
        let mut b = ByteBuffer::with_capacity(2);
        SlotDisplay::Empty.encode(&mut b).unwrap();
        SlotDisplay::AnyFuel.encode(&mut b).unwrap();
        assert_eq!(b.as_slice(), &[0x00, 0x01]);
    }

    #[test]
    fn slot_display_item_roundtrip() {
        let d = SlotDisplay::Item(264);
        assert_eq!(roundtrip_slot(&d), d);
        // 类型 2 + VarInt material 264（0x88 0x02）。
        let mut b = ByteBuffer::with_capacity(4);
        d.encode(&mut b).unwrap();
        assert_eq!(b.as_slice(), &[0x02, 0x88, 0x02]);
    }

    #[test]
    fn slot_display_item_stack_roundtrip() {
        let d = SlotDisplay::ItemStack(ItemStack::new(264, 2));
        assert_eq!(roundtrip_slot(&d), d);
    }

    #[test]
    fn slot_display_tag_roundtrip() {
        let d = SlotDisplay::Tag("minecraft:planks".to_string());
        assert_eq!(roundtrip_slot(&d), d);
    }

    #[test]
    fn slot_display_smithing_trim_roundtrip() {
        let d = SlotDisplay::SmithingTrim(
            Box::new(SlotDisplay::Item(1)),
            Box::new(SlotDisplay::Tag("minecraft:trim_materials".to_string())),
            Box::new(SlotDisplay::Item(2)),
        );
        assert_eq!(roundtrip_slot(&d), d);
    }

    #[test]
    fn slot_display_with_remainder_roundtrip() {
        let d = SlotDisplay::WithRemainder(
            Box::new(SlotDisplay::Item(1)),
            Box::new(SlotDisplay::Item(325)),
        );
        assert_eq!(roundtrip_slot(&d), d);
    }

    #[test]
    fn slot_display_composite_roundtrip() {
        let d = SlotDisplay::Composite(vec![
            SlotDisplay::Empty,
            SlotDisplay::Item(1),
            SlotDisplay::Composite(vec![SlotDisplay::AnyFuel]),
        ]);
        assert_eq!(roundtrip_slot(&d), d);
    }

    #[test]
    fn slot_display_unknown_type_rejected() {
        let mut b = ByteBuffer::with_capacity(8);
        b.put_varint(99);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            SlotDisplay::decode(&mut b),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn slot_display_composite_negative_count_rejected() {
        let mut b = ByteBuffer::with_capacity(8);
        b.put_varint(7); // Composite
        b.put_varint(-1);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            SlotDisplay::decode(&mut b),
            Err(ProtocolError::InvalidValue)
        );
    }

    // ── RecipeDisplay ──

    #[test]
    fn recipe_display_shapeless_roundtrip() {
        let d = RecipeDisplay::CraftingShapeless {
            ingredients: vec![SlotDisplay::Item(17), SlotDisplay::Item(17)],
            result: SlotDisplay::Item(5),
            crafting_station: SlotDisplay::Item(58),
        };
        assert_eq!(roundtrip_display(&d), d);
    }

    #[test]
    fn recipe_display_shaped_roundtrip() {
        let d = RecipeDisplay::CraftingShaped {
            width: 2,
            height: 1,
            ingredients: vec![SlotDisplay::Item(17), SlotDisplay::Item(17)],
            result: SlotDisplay::Item(280),
            crafting_station: SlotDisplay::Item(58),
        };
        assert_eq!(roundtrip_display(&d), d);
    }

    #[test]
    fn recipe_display_furnace_roundtrip() {
        let d = RecipeDisplay::Furnace {
            ingredient: SlotDisplay::Item(15),
            fuel: SlotDisplay::Item(263),
            result: SlotDisplay::Item(265),
            crafting_station: SlotDisplay::Item(61),
            duration: 200,
            experience: 0.7,
        };
        assert_eq!(roundtrip_display(&d), d);
        // f32 位精确往返
        if let RecipeDisplay::Furnace {
            duration,
            experience,
            ..
        } = roundtrip_display(&d)
        {
            assert_eq!(duration, 200);
            assert_eq!(experience, 0.7);
        }
    }

    #[test]
    fn recipe_display_stonecutter_roundtrip() {
        let d = RecipeDisplay::Stonecutter {
            ingredient: SlotDisplay::Item(1),
            result: SlotDisplay::Item(44),
            crafting_station: SlotDisplay::Item(449),
        };
        assert_eq!(roundtrip_display(&d), d);
    }

    #[test]
    fn recipe_display_smithing_roundtrip() {
        let d = RecipeDisplay::Smithing {
            template: SlotDisplay::Item(650),
            base: SlotDisplay::Item(276),
            addition: SlotDisplay::Item(742),
            result: SlotDisplay::Item(743),
            crafting_station: SlotDisplay::Item(1091),
        };
        assert_eq!(roundtrip_display(&d), d);
    }

    #[test]
    fn crafting_shaped_rejects_dimension_mismatch() {
        let ok = RecipeDisplay::crafting_shaped(
            2,
            2,
            vec![SlotDisplay::Empty; 4],
            SlotDisplay::Empty,
            SlotDisplay::Empty,
        );
        assert!(ok.is_ok());
        // 原料数 3 ≠ 宽×高 4
        let bad = RecipeDisplay::crafting_shaped(
            2,
            2,
            vec![SlotDisplay::Empty; 3],
            SlotDisplay::Empty,
            SlotDisplay::Empty,
        );
        assert_eq!(bad, Err(RecipeError::InvalidShapedDimensions));
        // 负数尺寸
        let neg =
            RecipeDisplay::crafting_shaped(-1, 2, vec![], SlotDisplay::Empty, SlotDisplay::Empty);
        assert_eq!(neg, Err(RecipeError::InvalidShapedDimensions));
        // 乘法越界
        let overflow = RecipeDisplay::crafting_shaped(
            i32::MAX,
            2,
            vec![],
            SlotDisplay::Empty,
            SlotDisplay::Empty,
        );
        assert_eq!(overflow, Err(RecipeError::InvalidShapedDimensions));
    }

    #[test]
    fn crafting_shaped_decode_rejects_dimension_mismatch() {
        // 手工构造：宽2×高2 但原料 3 个 → decode 拒绝。
        let mut b = ByteBuffer::with_capacity(32);
        b.put_varint(1); // CraftingShaped
        b.put_varint(2); // width
        b.put_varint(2); // height
        b.put_varint(3); // ingredients len
        for _ in 0..3 {
            SlotDisplay::Empty.encode(&mut b).unwrap();
        }
        SlotDisplay::Empty.encode(&mut b).unwrap(); // result
        SlotDisplay::Empty.encode(&mut b).unwrap(); // crafting_station
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            RecipeDisplay::decode(&mut b),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn recipe_display_unknown_type_rejected() {
        let mut b = ByteBuffer::with_capacity(8);
        b.put_varint(42);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            RecipeDisplay::decode(&mut b),
            Err(ProtocolError::InvalidValue)
        );
    }

    // ── RecipeProperty ──

    #[test]
    fn recipe_property_all_variants_roundtrip() {
        let all = [
            RecipeProperty::SmithingBase(None),
            RecipeProperty::SmithingTemplate(None),
            RecipeProperty::SmithingAddition(None),
            RecipeProperty::FurnaceInput(None),
            RecipeProperty::BlastFurnaceInput(None),
            RecipeProperty::SmokerInput(None),
            RecipeProperty::CampfireInput(None),
        ];
        for expected in &all {
            let mut b = ByteBuffer::with_capacity(32);
            expected.encode(&mut b);
            let mut b = ByteBuffer::new(b.into_inner());
            assert_eq!(RecipeProperty::decode(&mut b).unwrap(), *expected);
        }
    }

    #[test]
    fn recipe_property_default_keys() {
        assert_eq!(
            RecipeProperty::FurnaceInput(None).default_key(),
            "furnace_input"
        );
        assert_eq!(RecipeProperty::SmithingBase(None).key(), "smithing_base");
        // 自定义 key 优先
        assert_eq!(
            RecipeProperty::CampfireInput(Some("my_category".to_string())).key(),
            "my_category"
        );
    }

    #[test]
    fn recipe_property_unknown_key_rejected() {
        let mut b = ByteBuffer::with_capacity(16);
        b.put_string("not_a_real_category");
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(
            RecipeProperty::decode(&mut b),
            Err(ProtocolError::InvalidValue)
        );
    }

    // ── Ingredient ──

    #[test]
    fn ingredient_items_roundtrip_exact_bytes() {
        let d = Ingredient::Items(vec![1, 2]);
        let mut b = ByteBuffer::with_capacity(8);
        d.encode(&mut b).unwrap();
        // len+1=3，后跟 material 1、2。
        assert_eq!(b.as_slice(), &[0x03, 0x01, 0x02]);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(Ingredient::decode(&mut b).unwrap(), d);
    }

    #[test]
    fn ingredient_empty_items_roundtrip() {
        let d = Ingredient::Items(vec![]);
        let mut b = ByteBuffer::with_capacity(8);
        d.encode(&mut b).unwrap();
        // len+1=1，无 material。
        assert_eq!(b.as_slice(), &[0x01]);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(Ingredient::decode(&mut b).unwrap(), d);
    }

    #[test]
    fn ingredient_tag_roundtrip() {
        let d = Ingredient::Tag("minecraft:planks".to_string());
        let mut b = ByteBuffer::with_capacity(32);
        d.encode(&mut b).unwrap();
        // 计数 0 表示 tag 名随后。
        assert_eq!(b.as_slice().first(), Some(&0x00));
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(Ingredient::decode(&mut b).unwrap(), d);
    }

    #[test]
    fn ingredient_negative_count_rejected() {
        let mut b = ByteBuffer::with_capacity(8);
        b.put_varint(-1);
        let mut b = ByteBuffer::new(b.into_inner());
        assert_eq!(Ingredient::decode(&mut b), Err(ProtocolError::InvalidValue));
    }

    // ── RecipeManager ──

    #[test]
    fn manager_empty_encodes_two_zero_counts() {
        let mgr = RecipeManager::default();
        // 空 manager：item_properties 计数 0 + stonecutter_recipes 计数 0。
        assert_eq!(mgr.to_bytes(), vec![0x00, 0x00]);
    }

    #[test]
    fn manager_item_property_and_stonecutter_wire_output() {
        let mut mgr = RecipeManager::default();
        mgr.add_item_property(RecipeProperty::FurnaceInput(None), vec![1, 2]);
        mgr.add_item_property(RecipeProperty::SmokerInput(None), vec![3]);
        mgr.add_stonecutter_recipe(
            Ingredient::Items(vec![1]),
            SlotDisplay::ItemStack(ItemStack::new(44, 2)),
        );
        mgr.add_stonecutter_recipe(
            Ingredient::Tag("minecraft:logs".to_string()),
            SlotDisplay::Item(5),
        );

        let (props, scs) = decode_manager_bytes(&mgr.to_bytes());
        assert_eq!(
            props,
            vec![
                (RecipeProperty::FurnaceInput(None), vec![1, 2]),
                (RecipeProperty::SmokerInput(None), vec![3]),
            ]
        );
        assert_eq!(
            scs,
            vec![
                StonecutterRecipe {
                    ingredient: Ingredient::Items(vec![1]),
                    option_display: SlotDisplay::ItemStack(ItemStack::new(44, 2)),
                },
                StonecutterRecipe {
                    ingredient: Ingredient::Tag("minecraft:logs".to_string()),
                    option_display: SlotDisplay::Item(5),
                },
            ]
        );
    }

    #[test]
    fn manager_preserves_item_property_order() {
        let mut mgr = RecipeManager::default();
        mgr.add_item_property(RecipeProperty::SmithingTemplate(None), vec![650]);
        mgr.add_item_property(RecipeProperty::CampfireInput(None), vec![9]);
        let (props, _) = decode_manager_bytes(&mgr.to_bytes());
        let first = props.first().unwrap();
        assert_eq!(first.0, RecipeProperty::SmithingTemplate(None));
        assert_eq!(first.1, vec![650]);
    }

    #[test]
    fn manager_legacy_stonecutter_maps_to_wire() {
        let mut mgr = RecipeManager::default();
        mgr.register(Recipe::Stonecutting {
            id: "stone_slab".to_string(),
            ingredient: ItemStack::new(1, 1),
            result: ItemStack::new(44, 2),
        })
        .unwrap();
        let (props, scs) = decode_manager_bytes(&mgr.to_bytes());
        assert!(props.is_empty());
        let sc = scs.first().unwrap();
        assert_eq!(sc.ingredient, Ingredient::Items(vec![1]));
        assert_eq!(
            sc.option_display,
            SlotDisplay::ItemStack(ItemStack::new(44, 2))
        );
    }

    #[test]
    fn manager_legacy_api_register_unregister() {
        let mut mgr = RecipeManager::default();
        let a = Recipe::Shapeless {
            id: "x".to_string(),
            ingredients: vec![ItemStack::new(17, 1)],
            result: ItemStack::new(5, 4),
        };
        let dup = Recipe::Shapeless {
            id: "x".to_string(),
            ingredients: vec![ItemStack::new(17, 1)],
            result: ItemStack::new(5, 1),
        };
        assert_eq!(mgr.register(a), Ok(()));
        assert_eq!(
            mgr.register(dup),
            Err(RecipeError::DuplicateId("x".to_string()))
        );
        assert_eq!(mgr.all().len(), 1);
        assert!(mgr.unregister("x"));
        assert!(!mgr.unregister("x"));
        assert!(mgr.all().is_empty());
        // 应用侧非切石机配方不进入 stonecutter_recipes。
        mgr.register(Recipe::Shapeless {
            id: "y".to_string(),
            ingredients: vec![ItemStack::new(17, 1)],
            result: ItemStack::new(5, 4),
        })
        .unwrap();
        let (_, scs) = decode_manager_bytes(&mgr.to_bytes());
        assert!(scs.is_empty());
    }

    #[test]
    fn stonecutter_recipe_wire_roundtrip() {
        let mut mgr = RecipeManager::default();
        mgr.add_stonecutter_recipe(
            Ingredient::Items(vec![1, 3]),
            SlotDisplay::SmithingTrim(
                Box::new(SlotDisplay::Item(650)),
                Box::new(SlotDisplay::Tag("minecraft:trim_materials".to_string())),
                Box::new(SlotDisplay::Item(651)),
            ),
        );
        let (_, scs) = decode_manager_bytes(&mgr.to_bytes());
        assert_eq!(scs, mgr.stonecutter_recipes());
    }
}
