// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! particlemc-framework-ecs 的过程宏 crate：`Component` / `Archetype` / `Message` 派生宏
//! 与 `register_archetypes!` / `register_components!` 注册宏。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 宏输出引用 `particlemc_framework_ecs` crate 的权威类型（IC-2 / IC-3）。组件 ID 遵循
//! AI Amendment A1 的惰性全局分配语义：`#[derive(Component)]` 生成的 `id()`
//! 调用 `particlemc_framework_ecs::component::register_component_id`；Archetype 表由
//! `register_archetypes!` 聚合为启动期一次性初始化的静态表。派生宏在外部
//! crate 中展开，生成代码使用 `particlemc_framework_ecs::...` 限定的裸路径（不带前导
//! `::`），既兼容真实 crate 的 extern prelude 解析，也让集成测试能以同名
//! `mod particlemc_framework_ecs` 存根验证展开。

#![deny(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use proc_macro::TokenStream;
use proc_macro2::{Ident, Literal, Span, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Attribute, Data, DeriveInput, Error, Lit, Meta, Token, parse_macro_input, parse_quote};

/// `#[component(storage = "...")]` 解析出的存储类别（IC-2 / R2.2）。
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum StorageKind {
    /// 结构体数组（SoA）列存储，默认。
    SoA,
    /// SparseSet 独立存储（`#[component(storage = "sparse")]`）。
    Sparse,
}

/// 解析 `#[component(...)]` 属性中的存储类别。
///
/// 接受 `storage = "sparse"`（Sparse）与 `storage = "soa"`（SoA）；属性缺失或
/// 为 `storage = "soa"` 时默认 SoA。未知取值、重复指定或未知键返回 `Err`
/// （derive 入口将其转为 `compile_error!`）。
fn parse_storage(attrs: &[Attribute]) -> syn::Result<StorageKind> {
    let mut storage: Option<StorageKind> = None;
    for attr in attrs {
        if !attr.path().is_ident("component") {
            continue;
        }
        let list = match &attr.meta {
            Meta::List(list) => list,
            Meta::Path(path) => {
                return Err(Error::new_spanned(
                    path,
                    "`component` 属性需为 `#[component(storage = \"...\")]` 形式",
                ));
            }
            Meta::NameValue(value) => {
                return Err(Error::new_spanned(
                    value,
                    "`component` 属性不支持名值形式，需为 `#[component(storage = \"...\")]`",
                ));
            }
        };
        list.parse_nested_meta(|meta| {
            if meta.path.is_ident("storage") {
                let value: syn::LitStr = meta.value()?.parse()?;
                let kind = match value.value().as_str() {
                    "sparse" => StorageKind::Sparse,
                    "soa" => StorageKind::SoA,
                    other => {
                        return Err(Error::new_spanned(
                            &value,
                            format!("未知存储类别 `{other}`，仅支持 \"soa\" 或 \"sparse\""),
                        ));
                    }
                };
                if storage.replace(kind).is_some() {
                    return Err(Error::new_spanned(
                        &value,
                        "`storage` 属性重复指定（每个组件至多一个）",
                    ));
                }
                Ok(())
            } else {
                Err(Error::new_spanned(
                    &meta.path,
                    "未知 `component` 属性，仅支持 storage",
                ))
            }
        })?;
    }
    Ok(storage.unwrap_or(StorageKind::SoA))
}

/// 组件派生宏：生成 `Component` trait 实现（IC-2）。
///
/// 属性：`#[component(storage = "sparse" | "soa")]`（缺省 SoA）。泛型类型参数
/// 自动追加 `T: 'static` 约束以满足 trait 超 trait。
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_component(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_component(input: DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let storage = parse_storage(&input.attrs)?;
    let storage_path = match storage {
        StorageKind::SoA => quote!(particlemc_framework_ecs::component::ComponentStorage::SoA),
        StorageKind::Sparse => quote!(particlemc_framework_ecs::component::ComponentStorage::Sparse),
    };
    let mut generics = input.generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(parse_quote!('static));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics particlemc_framework_ecs::component::Component
            for #ident #ty_generics #where_clause
        {
            /// 惰性全局分配组件 ID（启动期一次性初始化，之后恒定；AI Amendment A1）。
            fn id() -> particlemc_framework_ecs::component::ComponentId {
                particlemc_framework_ecs::component::register_component_id(
                    ::std::any::TypeId::of::<#ident #ty_generics>(),
                )
            }

            /// 组件存储类别：默认 SoA；`#[component(storage = "sparse")]` 为 Sparse。
            const STORAGE: particlemc_framework_ecs::component::ComponentStorage = #storage_path;

            /// 存储元数据占位（列类型/大小/对齐，T3 存储实现消费）。
            type Registry = ();
        }
    })
}

/// Archetype 派生宏：结构体字段均为组件类型，生成运行时定义构造与组件类型元组。
///
/// 字段类型未实现 [`Component`] 时，`archetype_def` 内对 `Component::id()` 的
/// 调用使编译失败（编译期强制，R2.1）。不支持泛型参数（静态 Archetype 的
/// 组件集合必须编译期固定）。
#[proc_macro_derive(Archetype)]
pub fn derive_archetype(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_archetype(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_archetype(input: DeriveInput) -> syn::Result<TokenStream2> {
    let data = match &input.data {
        Data::Struct(data) => data,
        Data::Enum(data) => {
            return Err(Error::new_spanned(
                data.enum_token,
                "Archetype 仅支持结构体（每个字段为一个组件类型）",
            ));
        }
        Data::Union(data) => {
            return Err(Error::new_spanned(
                data.union_token,
                "Archetype 仅支持结构体（每个字段为一个组件类型）",
            ));
        }
    };
    // 泛型会改变字段组件集合，与"编译期固定"的静态 Archetype 语义冲突，直接拒绝
    //（OnceLock 静态表也无法按类型参数区分实例）。
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.ident,
            "Archetype 不支持泛型参数（静态 Archetype 的组件集合必须编译期固定）",
        ));
    }
    let fields: Vec<&syn::Field> = data.fields.iter().collect();
    if fields.is_empty() {
        return Err(Error::new_spanned(
            &input.ident,
            "Archetype 至少需要一个组件字段",
        ));
    }
    let ident = &input.ident;
    let field_count = Literal::usize_unsuffixed(fields.len());
    // `{类型名}ComponentList` 顶层别名：同一模块内类型名唯一，避免多个 Archetype
    // 都叫 `ComponentList` 造成的别名冲突（stable Rust 尚无 inherent associated type）。
    let list_alias = Ident::new(&format!("{}ComponentList", ident), ident.span());

    let id_calls = fields.iter().map(|field| {
        let ty = &field.ty;
        quote!(<#ty as particlemc_framework_ecs::component::Component>::id())
    });
    let type_of_calls = fields.iter().map(|field| {
        let ty = &field.ty;
        quote!(::std::any::TypeId::of::<#ty>())
    });
    let component_types = fields.iter().map(|field| &field.ty);

    Ok(quote! {
        impl #ident {
            /// 构造该 Archetype 的运行时定义。
            ///
            /// `id` 与 `entity_kind` 由 `register_archetypes!` 注入。组件 ID 经
            /// 惰性全局分配（AI Amendment A1），无法 const 求值，故 `component_ids`
            /// 在函数内 `static` `OnceLock` 中一次性初始化后以 `'static` 切片返回；
            /// 字段类型未实现 [`particlemc_framework_ecs::component::Component`] 时，此处调用
            /// `Component::id()` 使编译失败（编译期强制，R2.1）。
            pub fn archetype_def(
                id: particlemc_framework_ecs::archetype::ArchetypeId,
                entity_kind: particlemc_framework_ecs::entity::EntityTypeId,
            ) -> particlemc_framework_ecs::archetype::ArchetypeDef {
                static COMPONENT_IDS: ::std::sync::OnceLock<
                    [particlemc_framework_ecs::component::ComponentId; #field_count]
                > = ::std::sync::OnceLock::new();
                static COMPONENT_TYPES: ::std::sync::OnceLock<
                    [::std::any::TypeId; #field_count]
                > = ::std::sync::OnceLock::new();
                particlemc_framework_ecs::archetype::ArchetypeDef {
                    id,
                    name: ::core::stringify!(#ident),
                    component_ids: COMPONENT_IDS.get_or_init(|| [
                        #(#id_calls),*
                    ]),
                    entity_kind,
                    component_types: COMPONENT_TYPES.get_or_init(|| [
                        #(#type_of_calls),*
                    ]),
                }
            }
        }

        /// 组件类型元组（供 Query 编译期匹配与 `match_mask` 推导使用）。
        pub type #list_alias = (#(#component_types,)*);
    })
}

/// Message 派生宏：生成 `Message: Send + Sync + 'static` 的 trait 实现（IC-8）。
///
/// 泛型类型参数自动追加 `T: Send + Sync + 'static` 约束。
#[proc_macro_derive(Message)]
pub fn derive_message(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_message(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_message(input: DeriveInput) -> syn::Result<TokenStream2> {
    let ident = &input.ident;
    let mut generics = input.generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(parse_quote!(Send));
        param.bounds.push(parse_quote!(Sync));
        param.bounds.push(parse_quote!('static));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        /// 空实现即满足超 trait 要求：消息类型必须可跨线程发送与 `'static`。
        impl #impl_generics particlemc_framework_ecs::message::Message
            for #ident #ty_generics #where_clause
        {}
    })
}

/// 将 `Option<TokenTree>` 转为 token；缺失时返回 `message` 错误。
fn expect_token(token: Option<TokenTree>, message: &str) -> syn::Result<TokenTree> {
    token.ok_or_else(|| Error::new(Span::call_site(), message))
}

/// 断言下一 token 为指定标点字符。
fn expect_punct(token: Option<TokenTree>, ch: char, message: &str) -> syn::Result<()> {
    match expect_token(token, message)? {
        TokenTree::Punct(punct) if punct.as_char() == ch => Ok(()),
        other => Err(Error::new_spanned(other, message)),
    }
}

/// 解析整数字面量为 u8（禁止 `as` 缩窄，用 `TryFrom`）。
fn parse_u8_literal(literal: &Literal) -> syn::Result<u8> {
    let int = match Lit::new(literal.clone()) {
        Lit::Int(int) => int,
        _ => return Err(Error::new_spanned(literal, "期望整数字面量")),
    };
    let value: u64 = int
        .base10_parse()
        .map_err(|err| Error::new(int.span(), format!("实体类型 ID 字面量解析失败：{err}")))?;
    u8::try_from(value).map_err(|_| Error::new(int.span(), "实体类型 ID 超出 u8 范围（0-255）"))
}

/// 解析注册条目右侧：整数字面量或 `EntityTypeId(<整数字面量>)`，返回实体类型 ID。
///
/// 注意 `EntityTypeId(0)` 中的括号在 token 流中是 `Group`（括号定界符），
/// 需解包 group 再解析内部字面量。
fn parse_entity_kind(tokens: TokenStream2) -> syn::Result<u8> {
    let mut iter = tokens.into_iter();
    let first = expect_token(
        iter.next(),
        "期望实体类型 ID（整数字面量或 EntityTypeId(整数字面量)）",
    )?;
    match first {
        TokenTree::Literal(literal) => parse_u8_literal(&literal),
        TokenTree::Ident(ident) if ident == "EntityTypeId" => {
            let group = match iter.next() {
                Some(TokenTree::Group(group)) => group,
                other => {
                    return Err(Error::new_spanned(
                        other,
                        "`EntityTypeId` 需后接 `(整数字面量)`",
                    ));
                }
            };
            let mut inner = group.stream().into_iter();
            let literal = match inner.next() {
                Some(TokenTree::Literal(literal)) => literal,
                other => {
                    return Err(Error::new_spanned(
                        other,
                        "`EntityTypeId` 括号内需为整数字面量",
                    ));
                }
            };
            if inner.next().is_some() {
                return Err(Error::new_spanned(
                    &literal,
                    "`EntityTypeId` 括号内只能有一个整数字面量",
                ));
            }
            parse_u8_literal(&literal)
        }
        other => Err(Error::new_spanned(
            other,
            "期望整数字面量或 `EntityTypeId(整数字面量)`",
        )),
    }
}

/// 类型名转 UPPER_SNAKE（全大写 + 下划线分隔）。
fn to_upper_snake(ident: &Ident) -> String {
    let mut name = String::new();
    for ch in ident.to_string().chars() {
        if ch.is_ascii_uppercase() {
            if !name.is_empty() {
                name.push('_');
            }
            name.push(ch);
        } else if ch.is_ascii_lowercase() {
            name.push(ch.to_ascii_uppercase());
        } else {
            name.push(ch);
        }
    }
    name
}

/// Archetype ID 常量名：类型名 UPPER_SNAKE + `_ARCHETYPE` 后缀。
fn const_name_for(ident: &Ident) -> String {
    let base = to_upper_snake(ident);
    if base.ends_with("_ARCHETYPE") {
        base
    } else {
        format!("{base}_ARCHETYPE")
    }
}

/// 解析条目前的连续外层属性（含文档注释），原样返回以便转发到生成常量上。
fn parse_leading_attrs(
    iter: &mut std::iter::Peekable<impl Iterator<Item = TokenTree>>,
) -> Vec<TokenStream2> {
    let mut attrs = Vec::new();
    loop {
        match iter.peek() {
            Some(TokenTree::Punct(punct))
                if punct.as_char() == '#' && punct.spacing() == proc_macro2::Spacing::Alone =>
            {
                let hash = match iter.next() {
                    Some(token) => token,
                    None => break,
                };
                match iter.next() {
                    Some(TokenTree::Group(group))
                        if group.delimiter() == proc_macro2::Delimiter::Bracket =>
                    {
                        attrs.push(TokenStream2::from_iter([hash, TokenTree::Group(group)]));
                    }
                    // 残缺的 `#[` 不算属性：把 `#` 交回给入口解析，由其后报错
                    _ => {
                        attrs.push(TokenStream2::from(hash));
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    attrs
}

/// `register_archetypes!`：聚合 Archetype 定义表与 ID 常量（IC-3）。
///
/// 输入语法：`[#属性] TypeName => EntityTypeId(n),` 或 `TypeName => n,`（n 为
/// 0-255 字面量，宏内部包 `EntityTypeId`）。表下标即 `ArchetypeId`（按参数位置
/// 从 0 递增），实体类型 ID 取右侧字面量；条目前缀属性转发到生成的 ID 常量。
#[proc_macro]
pub fn register_archetypes(input: TokenStream) -> TokenStream {
    let input: TokenStream2 = input.into();
    match expand_register_archetypes(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_register_archetypes(input: TokenStream2) -> syn::Result<TokenStream2> {
    let mut iter = input.into_iter().peekable();
    let mut entries: Vec<(Vec<TokenStream2>, Ident, u8)> = Vec::new();
    while iter.peek().is_some() {
        let attrs = parse_leading_attrs(&mut iter);
        let ty = match expect_token(iter.next(), "期望 Archetype 类型名（标识符）")? {
            TokenTree::Ident(ident) => ident,
            other => return Err(Error::new_spanned(other, "期望 Archetype 类型名（标识符）")),
        };
        expect_punct(iter.next(), '=', "期望 `=>`（类型名与实体类型 ID 之间）")?;
        expect_punct(iter.next(), '>', "期望 `=>`（类型名与实体类型 ID 之间）")?;
        let mut rhs = TokenStream2::new();
        for token in iter.by_ref() {
            if let TokenTree::Punct(punct) = &token
                && punct.as_char() == ','
            {
                break;
            }
            rhs.extend([token]);
        }
        let kind = parse_entity_kind(rhs)?;
        entries.push((attrs, ty, kind));
    }
    if entries.is_empty() {
        return Err(Error::new(
            Span::call_site(),
            "`register_archetypes!` 至少需要一个 Archetype",
        ));
    }

    let mut defs = Vec::new();
    let mut constants = Vec::new();
    for (index, (attrs, ty, kind)) in entries.iter().enumerate() {
        let id_literal = match u16::try_from(index) {
            Ok(value) => Literal::u16_suffixed(value),
            Err(_) => return Err(Error::new(ty.span(), "Archetype 数量超过 u16 上限")),
        };
        let kind_literal = Literal::u8_suffixed(*kind);
        let const_name = Ident::new(&const_name_for(ty), ty.span());
        defs.push(quote! {
            #ty::archetype_def(
                particlemc_framework_ecs::archetype::ArchetypeId(#id_literal),
                particlemc_framework_ecs::entity::EntityTypeId(#kind_literal),
            )
        });
        constants.push(quote! {
            #(#attrs)*
            /// 编译期 Archetype ID 常量（值即表内下标，IC-3）。
            pub const #const_name: particlemc_framework_ecs::archetype::ArchetypeId =
                particlemc_framework_ecs::archetype::ArchetypeId(#id_literal);
        });
    }

    Ok(quote! {
        /// 全部 Archetype 的运行时定义表。
        ///
        /// 表下标即 `ArchetypeId`；首次访问时对每个字段类型调用 `Component::id()`，
        /// 完成组件惰性注册（AI Amendment A1）。组件 ID 为运行时惰性分配（无法
        /// const 求值），故以 `LazyLock` 包装 `&'static [ArchetypeDef]` 表达静态
        /// 表语义——IC-3 字面意义上的 const 静态表在本机制下不可行，语义等价。
        pub static ARCHETYPES: ::std::sync::LazyLock<
            &'static [particlemc_framework_ecs::archetype::ArchetypeDef]
        > = ::std::sync::LazyLock::new(|| {
            static DEFS: ::std::sync::OnceLock<
                ::std::vec::Vec<particlemc_framework_ecs::archetype::ArchetypeDef>
            > = ::std::sync::OnceLock::new();
            DEFS.get_or_init(|| {
                ::std::vec![
                    #(#defs),*
                ]
            }).as_slice()
        });

        #(#constants)*

        /// 启动期调用一次以完成惰性注册（Archetype 表与其中全部组件 ID）。
        #[inline]
        pub fn register_all() {
            let _ = &*ARCHETYPES;
        }
    })
}

/// `register_components!`：显式组件清单注册（可选加速，AI Amendment A1）。
///
/// 输入：类型列表（以逗号分隔），每个类型需实现 [`particlemc_framework_ecs::component::Component`]。
/// 展开为 `__register_all_components()`：对每个类型调用 `Component::id()`，强制
/// 惰性注册在启动期一次完成。
#[proc_macro]
pub fn register_components(input: TokenStream) -> TokenStream {
    let input: TokenStream2 = input.into();
    let types = match Punctuated::<syn::Type, Token![,]>::parse_terminated.parse2(input) {
        Ok(types) => types,
        Err(err) => return err.to_compile_error().into(),
    };
    let calls = types
        .iter()
        .map(|ty| quote!(let _ = <#ty as particlemc_framework_ecs::component::Component>::id();));
    quote! {
        /// 显式注册组件清单（可选加速）：启动期调用一次，完成全部列出组件的惰性 ID 分配。
        #[inline]
        pub fn __register_all_components() {
            #(#calls)*
        }
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn storage_defaults_to_soa() {
        assert_eq!(parse_storage(&[]).unwrap(), StorageKind::SoA);
    }

    #[test]
    fn storage_parses_sparse() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[component(storage = "sparse")])];
        assert_eq!(parse_storage(&attrs).unwrap(), StorageKind::Sparse);
    }

    #[test]
    fn storage_parses_soa() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[component(storage = "soa")])];
        assert_eq!(parse_storage(&attrs).unwrap(), StorageKind::SoA);
    }

    #[test]
    fn storage_ignores_unrelated_attributes() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[doc = "说明"])];
        assert_eq!(parse_storage(&attrs).unwrap(), StorageKind::SoA);
    }

    #[test]
    fn storage_rejects_unknown_value() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[component(storage = "bogus")])];
        assert!(parse_storage(&attrs).is_err());
    }

    #[test]
    fn storage_rejects_duplicate() {
        let attrs: Vec<Attribute> =
            vec![parse_quote!(#[component(storage = "sparse", storage = "sparse")])];
        assert!(parse_storage(&attrs).is_err());
    }

    #[test]
    fn storage_rejects_unknown_key() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[component(width = "sparse")])];
        assert!(parse_storage(&attrs).is_err());
    }

    #[test]
    fn const_name_conversion() {
        let ident: Ident = parse_quote!(PlayerArchetype);
        assert_eq!(to_upper_snake(&ident), "PLAYER_ARCHETYPE");
        assert_eq!(const_name_for(&ident), "PLAYER_ARCHETYPE");
        let player: Ident = parse_quote!(Player);
        assert_eq!(const_name_for(&player), "PLAYER_ARCHETYPE");
        let item: Ident = parse_quote!(ItemArchetype);
        assert_eq!(const_name_for(&item), "ITEM_ARCHETYPE");
    }

    #[test]
    fn entity_kind_parses_bare_literal() {
        let ts: TokenStream2 = "0".parse().unwrap();
        assert_eq!(parse_entity_kind(ts).unwrap(), 0);
    }

    #[test]
    fn entity_kind_parses_wrapped() {
        let ts: TokenStream2 = "EntityTypeId(1)".parse().unwrap();
        assert_eq!(parse_entity_kind(ts).unwrap(), 1);
    }

    #[test]
    fn entity_kind_rejects_overflow() {
        let ts: TokenStream2 = "300".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn entity_kind_rejects_garbage() {
        let ts: TokenStream2 = "foo".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn entity_kind_rejects_missing_paren() {
        let ts: TokenStream2 = "EntityTypeId".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn entity_kind_rejects_extra_tokens_in_paren() {
        let ts: TokenStream2 = "EntityTypeId(1 2)".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn entity_kind_rejects_non_literal() {
        let ts: TokenStream2 = "EntityTypeId(a)".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn storage_rejects_path_form() {
        // `#[component]` 无参形式：不接受，需显式 storage
        let attrs: Vec<Attribute> = vec![parse_quote!(#[component])];
        assert!(parse_storage(&attrs).is_err());
    }

    #[test]
    fn storage_rejects_name_value_form() {
        let attrs: Vec<Attribute> = vec![parse_quote!(#[component = "sparse"])];
        assert!(parse_storage(&attrs).is_err());
    }

    #[test]
    fn expand_component_default_soa() {
        let input: DeriveInput = parse_quote! {
            struct Position {
                x: f32,
                y: f32,
            }
        };
        let out = expand_component(input).unwrap().to_string();
        assert!(out.contains("register_component_id"));
        assert!(out.contains("ComponentStorage :: SoA"));
        assert!(out.contains("type Registry = ()"));
    }

    #[test]
    fn expand_component_sparse() {
        let input: DeriveInput = parse_quote! {
            #[component(storage = "sparse")]
            struct Potion {
                level: u8,
            }
        };
        let out = expand_component(input).unwrap().to_string();
        assert!(out.contains("ComponentStorage :: Sparse"));
    }

    #[test]
    fn expand_component_generic_adds_static_bound() {
        let input: DeriveInput = parse_quote! {
            struct GenericMarker<T> {
                value: T,
            }
        };
        let out = expand_component(input).unwrap().to_string();
        assert!(out.contains("impl < T : 'static >"));
    }

    #[test]
    fn expand_archetype_generates_def_and_alias() {
        let input: DeriveInput = parse_quote! {
            struct PlayerArchetype {
                position: Position,
                velocity: Velocity,
            }
        };
        let out = expand_archetype(input).unwrap().to_string();
        assert!(out.contains("pub fn archetype_def"));
        assert!(out.contains("PlayerArchetypeComponentList = (Position , Velocity"));
        assert!(out.contains("< Position as particlemc_framework_ecs :: component :: Component > :: id"));
    }

    #[test]
    fn expand_archetype_rejects_enum() {
        let input: DeriveInput = parse_quote! {
            enum E {
                A,
            }
        };
        assert!(expand_archetype(input).is_err());
    }

    #[test]
    fn expand_archetype_rejects_union() {
        let input: DeriveInput = parse_quote! {
            union U {
                a: u32,
            }
        };
        assert!(expand_archetype(input).is_err());
    }

    #[test]
    fn expand_archetype_rejects_empty_fields() {
        let input: DeriveInput = parse_quote! {
            struct Empty {}
        };
        assert!(expand_archetype(input).is_err());
    }

    #[test]
    fn expand_archetype_rejects_generics() {
        let input: DeriveInput = parse_quote! {
            struct GenericArch<T> {
                value: T,
            }
        };
        let err = expand_archetype(input).unwrap_err();
        assert!(err.to_string().contains("不支持泛型参数"));
    }

    #[test]
    fn expand_message_generates_impl() {
        let input: DeriveInput = parse_quote! {
            struct PlayerJoin {
                name: String,
            }
        };
        let out = expand_message(input).unwrap().to_string();
        assert!(out.contains("impl particlemc_framework_ecs :: message :: Message for PlayerJoin"));
    }

    #[test]
    fn expand_message_generic_adds_send_sync_static() {
        let input: DeriveInput = parse_quote! {
            struct BlockEvent<T> {
                payload: T,
            }
        };
        let out = expand_message(input).unwrap().to_string();
        assert!(out.contains("impl < T : Send + Sync + 'static >"));
    }

    #[test]
    fn expand_register_archetypes_generates_table() {
        let input: TokenStream2 = "PlayerArchetype => EntityTypeId(0), ItemArchetype => 1"
            .parse()
            .unwrap();
        let out = expand_register_archetypes(input).unwrap().to_string();
        assert!(out.contains("ARCHETYPES"));
        assert!(out.contains("pub const PLAYER_ARCHETYPE"));
        assert!(out.contains("pub const ITEM_ARCHETYPE"));
        assert!(out.contains("pub fn register_all"));
        assert!(out.contains("ArchetypeId (0u16)"));
        assert!(out.contains("ArchetypeId (1u16)"));
        assert!(out.contains("EntityTypeId (0u8)"));
    }

    #[test]
    fn expand_register_archetypes_preserves_entry_attrs() {
        let input: TokenStream2 = "/// 玩家 Archetype\nPlayerArchetype => 0".parse().unwrap();
        let out = expand_register_archetypes(input).unwrap().to_string();
        assert!(out.contains("doc ="));
    }

    #[test]
    fn expand_register_archetypes_rejects_empty() {
        let input: TokenStream2 = TokenStream2::new();
        assert!(expand_register_archetypes(input).is_err());
    }

    #[test]
    fn expand_register_archetypes_rejects_missing_arrow() {
        let input: TokenStream2 = "PlayerArchetype 0".parse().unwrap();
        assert!(expand_register_archetypes(input).is_err());
    }

    #[test]
    fn expand_register_archetypes_rejects_bad_entry_name() {
        let input: TokenStream2 = "0 => EntityTypeId(0)".parse().unwrap();
        assert!(expand_register_archetypes(input).is_err());
    }

    #[test]
    fn const_name_dedup_when_already_suffixed() {
        // 类型名 UPPER_SNAKE 后已含 `_ARCHETYPE` 时不再追加后缀
        let ident: Ident = parse_quote!(Player_Archetype);
        assert_eq!(const_name_for(&ident), "PLAYER__ARCHETYPE");
    }

    #[test]
    fn leading_attrs_parsed_and_rest_untouched() {
        let input: TokenStream2 = "/// 说明\nPlayerArchetype => 0".parse().unwrap();
        let mut iter = input.into_iter().peekable();
        let attrs = parse_leading_attrs(&mut iter);
        assert_eq!(attrs.len(), 1);
        let rest: TokenStream2 = iter.collect();
        assert_eq!(rest.to_string(), "PlayerArchetype => 0");
    }

    #[test]
    fn leading_attrs_isolated_hash_is_kept() {
        // 孤立的 `#`（后接非属性 token）不算属性，原样返回给入口报错
        let input: TokenStream2 = "# foo".parse().unwrap();
        let mut iter = input.into_iter().peekable();
        let attrs = parse_leading_attrs(&mut iter);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].to_string(), "#");
    }

    #[test]
    fn leading_attrs_empty_input_returns_empty() {
        let mut iter = TokenStream2::new().into_iter().peekable();
        assert!(parse_leading_attrs(&mut iter).is_empty());
    }

    #[test]
    fn entity_kind_rejects_float_literal() {
        let ts: TokenStream2 = "1.5".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn entity_kind_rejects_oversized_literal() {
        // 超出 u64 的字面量：syn 会把各类进制归一化为十进制 digits，
        // 此处触发 base10_parse 解析失败分支
        let ts: TokenStream2 = "99999999999999999999999999".parse().unwrap();
        assert!(parse_entity_kind(ts).is_err());
    }

    #[test]
    fn expand_register_archetypes_rejects_attr_without_entry() {
        let input: TokenStream2 = "/// 只有注释没有条目".parse().unwrap();
        assert!(expand_register_archetypes(input).is_err());
    }

    #[test]
    fn expand_register_archetypes_rejects_lone_ident() {
        let input: TokenStream2 = "PlayerArchetype".parse().unwrap();
        assert!(expand_register_archetypes(input).is_err());
    }
}
