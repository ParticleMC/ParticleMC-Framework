// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! 系统：可调度的最小执行单元（IC-7/IC-9，T7）。
//!
//! 变更标识符：`implement-custom-ecs`
//!
//! 本模块实现：
//! - [`System`]：系统 trait（名称 / 执行 / 排他标记）；
//! - [`IntoSystem`]：函数/闭包 → 系统的转换（fn 0-4 参数经宏生成）；
//! - [`SystemParam`]：系统参数提取（`State` 一次初始化 + `Item<'w>` 借用形态）；
//! - [`Res`] / [`ResMut`]：只读 / 可变资源参数（IC-7）；
//! - [`FunctionSystem`]：函数式系统包装（`IntoSystem` 的产物）；
//! - [`ExclusiveSystem`]：排他系统（`fn(&mut World)`，`exclusive() == true`）。
//!
//! `Query` / `MessageWriter` / `MessageReader` / `Commands` 亦实现
//! [`SystemParam`]，供系统函数签名使用（R7.1）。
//!
//! # 借用互斥契约（A9）
//!
//! [`SystemParam::get`] 为 `unsafe`，其中 [`ResMut`] / `MessageWriter` /
//! `Commands` 经 [`World::resource_mut_unchecked`] 从共享 `&World` 下转可变
//! 引用，`Query<&mut>` 经 `World::query_any` 无约束构造。安全义务由 **调用方**
//! 承担：本模块中唯一调用方是 [`FunctionSystem::run`]——同一系统一次运行内
//! 各参数借用互斥，且系统按调度顺序串行执行（schedule 模块）。`Res<T>` 与
//! `ResMut<T>` 同参混用属框架误用（编译期不可表达：无法同时构造同一类型的
//! 共享+可变引用），跨系统冲突由调度器排他约束杜绝（R7.5）。

#![allow(unsafe_code)]

use crate::commands::{CommandBuffer, Commands};
use crate::message::{Message, MessageInbox, MessageReader, MessageWriter};
use crate::query::{Query, QueryData, QueryFilter};
use crate::resource::Resource;
use crate::world::World;

/// 系统：可调度的最小执行单元。
///
/// 所有实现必须 `Send + Sync`（可在调度线程间迁移执行，R11）。
pub trait System: Send + Sync + 'static {
    /// 系统名称（默认取函数类型名，用于调试与调度诊断）。
    fn name(&self) -> &'static str;
    /// 以独占 `&mut World` 执行本系统。
    fn run(&mut self, world: &mut World);
    /// 是否为排他系统（签名含 `&mut World` 参数的系统）。
    ///
    /// 排他系统须独占执行（R7.2 阶段屏障 / R7.5 互斥）。
    fn exclusive(&self) -> bool {
        false
    }
}

/// 函数式系统转换标记（宏为 fn 0-4 参数生成 impl）。
///
/// `Marker` 区分转换形态：`()` 为零参数系统；`fn(P0)` / `fn(P0, P1)` / … 的
/// 函数类型标记按参数个数区分（Coherence 要求不同 arity 的 impl 头不得重合，
/// 旧 ECS 方案 同款方案）；[`ExclusiveMarker`] 标记排他系统（`fn(&mut World)`）。
pub trait IntoSystem<Marker>: Sized {
    /// 转换后的具体系统类型。
    type System: System;
    /// 将函数/闭包转换为系统。
    fn into_system(self) -> Self::System;
}

/// 排他系统标记：`fn(&mut World)` 形态系统的 `IntoSystem` 目标。
pub struct ExclusiveMarker;

/// 系统参数：从 [`World`] 提取的借用形态。
///
/// - `State`：初始化状态（一次初始化、多次提取复用）；
/// - `Item<'w>`：单次运行的借用产出（`Res<'w, T>` 等）；
/// - `init_state`：初始化参数并返回状态（资源经 `init_resource` 惰性补默认
///   值，幂等）；
/// - `get`：从世界提取借用（unsafe，互斥契约见 [`crate::system`] 模块文档）。
pub trait SystemParam {
    /// 参数初始化状态。
    ///
    /// 附加约束（A12）：在 IC-9 的 `Send + Sync` 基础上增加 `Default`，使
    /// [`FunctionSystem`] 在无法预借 `&mut World` 的构造期可用占位状态，
    /// 首次 `run` 时再经 `init_state` 正式初始化。
    type State: Send + Sync + 'static;
    /// 借用产出形态，生命周期为 World 借用。
    type Item<'w>: SystemParam;
    /// 初始化参数（惰性补默认资源），返回可复用的状态。
    fn init_state(world: &mut World) -> Self::State;
    /// 从世界提取参数借用。
    ///
    /// # Safety
    ///
    /// 调用方（[`FunctionSystem::run`] 及元组 `get`）必须保证：同一系统一次
    /// 运行内，各参数对同一资源的借用互斥（A9 契约）。
    unsafe fn get<'w>(state: &Self::State, world: &'w World) -> Self::Item<'w>;
}

/// [`SystemParam::Item`] 投影别名（旧 ECS 方案 `SystemParamItem` 同款）。
///
/// 在 `for<'a> &'a mut Func: FnMut(SystemParamItem<$param>)` 这一 HRTB 约束中，
/// 省略的 `'w` 会被 Rust 推断为**独立的高阶生命周期**（与 `+` 左项的 `&'a mut Func`
/// 借用生命周期 `'a` 解耦），使 `call_inner` 调用点不再要求 `&mut self` 借用 `'1`
/// 必须等于参数 `'w`，从而消解 `lifetime may not live long enough`（T7 第三轮 E0xxx）。
pub(crate) type SystemParamItem<'w, P> = <P as SystemParam>::Item<'w>;

/// 只读资源参数：持有 `&'w T`，经 `Deref` 访问（IC-7）。
pub struct Res<'w, T: Resource>(pub(crate) &'w T);

impl<T: Resource> std::ops::Deref for Res<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: Resource + Default> SystemParam for Res<'_, T> {
    type State = ();
    type Item<'w> = Res<'w, T>;

    fn init_state(world: &mut World) -> Self::State {
        world.init_resource::<T>();
    }

    unsafe fn get<'w>(_: &Self::State, world: &'w World) -> Res<'w, T> {
        match world.resource::<T>() {
            Some(r) => Res(r),
            // 不可达：init_state（init_resource）已保证资源存在；若应用在两次
            // 运行间显式移除资源属框架误用，由调度器/应用契约约束
            None => unreachable!("Res 资源缺失：init_state 已初始化（不可达）"),
        }
    }
}

/// 可变资源参数：持有 `&'w mut T`，经 `Deref`/`DerefMut` 访问（IC-7）。
pub struct ResMut<'w, T: Resource>(pub(crate) &'w mut T);

impl<T: Resource> std::ops::Deref for ResMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &*self.0
    }
}

impl<T: Resource> std::ops::DerefMut for ResMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0
    }
}

impl<T: Resource + Default> SystemParam for ResMut<'_, T> {
    type State = ();
    type Item<'w> = ResMut<'w, T>;

    fn init_state(world: &mut World) -> Self::State {
        world.init_resource::<T>();
    }

    unsafe fn get<'w>(_: &Self::State, world: &'w World) -> ResMut<'w, T> {
        // SAFETY: 互斥契约由调用方（FunctionSystem::run）承担，本处直接透传
        match unsafe { world.resource_mut_unchecked::<T>() } {
            Some(r) => ResMut(r),
            // 不可达：init_state（init_resource）已保证资源存在
            None => unreachable!("ResMut 资源缺失：init_state 已初始化（不可达）"),
        }
    }
}

/// `Query` 作为系统参数（R7.1）：`D` 可含 `&mut` 元素，可变拆借契约由
/// 调度器承担（A9）。
impl<D: QueryData, F: QueryFilter> SystemParam for Query<'_, D, F> {
    type State = ();
    type Item<'w> = Query<'w, D, F>;

    fn init_state(_world: &mut World) -> Self::State {}

    unsafe fn get<'w>(_: &Self::State, world: &'w World) -> Query<'w, D, F> {
        // SAFETY: 调用方（调度器）保证查询生命周期内 D 中组件列无其他可变
        // 借用（R7.5）；只读形态（ReadOnlyQueryData）下天然无冲突
        world.query_any::<D, F>()
    }
}

/// 消息写入端作为系统参数：独占 inbox 可变引用（IC-8）。
impl<T: Message> SystemParam for MessageWriter<'_, T> {
    type State = ();
    type Item<'w> = MessageWriter<'w, T>;

    fn init_state(world: &mut World) -> Self::State {
        // inbox 以资源形式存在 World（add_message 注册 + 首次运行注入）
        world.init_resource::<MessageInbox<T>>();
    }

    unsafe fn get<'w>(_: &Self::State, world: &'w World) -> MessageWriter<'w, T> {
        match unsafe { world.resource_mut_unchecked::<MessageInbox<T>>() } {
            Some(inbox) => MessageWriter(inbox),
            // 不可达：init_state 已注入；缺失仅当用户显式移除 inbox 资源
            None => unreachable!("MessageInbox 缺失：init_state 已初始化（不可达）"),
        }
    }
}

/// 消息读取端作为系统参数：共享 inbox 引用（IC-8）。
impl<T: Message> SystemParam for MessageReader<'_, T> {
    type State = ();
    type Item<'w> = MessageReader<'w, T>;

    fn init_state(world: &mut World) -> Self::State {
        world.init_resource::<MessageInbox<T>>();
    }

    unsafe fn get<'w>(_: &Self::State, world: &'w World) -> MessageReader<'w, T> {
        match world.resource::<MessageInbox<T>>() {
            Some(inbox) => MessageReader(inbox),
            // 不可达：init_state 已注入
            None => unreachable!("MessageInbox 缺失：init_state 已初始化（不可达）"),
        }
    }
}

/// `Commands` 作为系统参数：独占 CommandBuffer 资源引用（R5.1）。
///
/// 系统执行期间仅入队命令，每 tick 起始由 schedule 批量 apply
/// （remove 资源 → apply → 插回，规避借用冲突）。
impl SystemParam for Commands<'_> {
    type State = ();
    type Item<'w> = Commands<'w>;

    fn init_state(world: &mut World) -> Self::State {
        world.init_resource::<CommandBuffer>();
    }

    unsafe fn get<'w>(_: &Self::State, world: &'w World) -> Commands<'w> {
        match unsafe { world.resource_mut_unchecked::<CommandBuffer>() } {
            Some(buffer) => Commands::new(buffer),
            // 不可达：init_state 已注入
            None => unreachable!("CommandBuffer 缺失：init_state 已初始化（不可达）"),
        }
    }
}

/// 函数式系统：`func`（函数/闭包）+ 惰性初始化的参数状态。
///
/// 由 `IntoSystem` 宏构造；`run` 首次调用时经 `init_state` 补资源默认值并
/// 缓存状态，之后复用。`Marker` 为函数类型 `fn(P0, P1, …)`（其参数生命周期
/// 被 Rust 量化为 HRTB，故 `FunctionSystem` 整体 `'static`）；参数元组类型经
/// [`SystemParamFunction::Param`] 关联取得，不出现在 struct 泛型中，从而规避
/// `Res<'w, T>` 携带的 `'w` 破坏 `'static`（E0310 根因）。
pub struct FunctionSystem<F, Marker>
where
    F: SystemParamFunction<Marker>,
{
    /// 原始函数/闭包。
    func: F,
    /// 参数状态（首次运行经 `init_state` 初始化后缓存；`None` 表示未运行过）。
    state: Option<<F::Param as SystemParam>::State>,
}

impl<F, Marker> FunctionSystem<F, Marker>
where
    F: SystemParamFunction<Marker>,
{
    fn new(func: F) -> Self {
        FunctionSystem { func, state: None }
    }
}

/// 排他系统：`fn(&mut World)` 的包装，`exclusive() == true`（IC-9）。
pub struct ExclusiveSystem<F> {
    func: F,
}

impl<F> System for ExclusiveSystem<F>
where
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn run(&mut self, world: &mut World) {
        (self.func)(world);
    }

    fn exclusive(&self) -> bool {
        true
    }
}

impl<F> IntoSystem<ExclusiveMarker> for F
where
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    type System = ExclusiveSystem<F>;

    fn into_system(self) -> Self::System {
        ExclusiveSystem { func: self }
    }
}

impl World {
    /// 从共享引用提取可变资源（`ResMut` 系统参数路径）。
    ///
    /// # Safety
    ///
    /// 调用方必须保证：返回的 `&mut T` 生命周期内，该资源无其他可变或共享
    /// 借用并存（与同系统其他参数互斥，A9 契约）。
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn resource_mut_unchecked<T: Resource>(&self) -> Option<&mut T> {
        // SAFETY: 互斥契约由调用方承担；具体下转经 ResourceMap::get_mut_unchecked
        unsafe { self.resources.get_mut_unchecked::<T>() }
    }
}

/// 元组 [`SystemParam`]：多参数系统的参数组合（0-4 元）。
macro_rules! impl_system_param_tuple {
    ($($P:ident),*) => {
        // 0 元参数时 `world` 未用、`unsafe { () }` 无 unsafe 操作；`$P` 同时作
        // 类型与 ref 绑定名（大写），allow 抑制上述 lint
        #[allow(unused_unsafe, unused_variables, non_snake_case, clippy::unused_unit)]
        impl<$($P: SystemParam),*> SystemParam for ($($P,)*) {
            type State = ($($P::State,)*);
            type Item<'w> = ($($P::Item<'w>,)*);

            fn init_state(world: &mut World) -> Self::State {
                // 依次初始化各参数并收集状态元组（0 元时为空元组）
                ($($P::init_state(world),)*)
            }

            unsafe fn get<'w>(state: &Self::State, world: &'w World) -> Self::Item<'w> {
                // 按位解构状态元组（ref 绑定借用元素，不移动）；$P 同时用作
                // 类型（泛型参数）与值（ref 绑定），分处类型/值命名空间不冲突
                let ($(ref $P,)*) = *state;
                // SAFETY: 委托调用方契约——FunctionSystem::run 保证同系统
                // 各参数借用互斥（A9）
                unsafe { ($(<$P as SystemParam>::get($P, world),)*) }
            }
        }
    };
}
impl_system_param_tuple!();
impl_system_param_tuple!(A);
impl_system_param_tuple!(A, B);
impl_system_param_tuple!(A, B, C);
impl_system_param_tuple!(A, B, C, D);
impl_system_param_tuple!(A, B, C, D, E);
impl_system_param_tuple!(A, B, C, D, E, F);
impl_system_param_tuple!(A, B, C, D, E, F, G);
impl_system_param_tuple!(A, B, C, D, E, F, G, H);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
impl_system_param_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q);

/// 解耦「函数签名类型」与「[`SystemParam::Item`] 投影」（旧 ECS 方案 同款模式）。
///
/// `F` 经 `for<'a> &'a mut F: FnMut(P) + FnMut(P::Item<'a>)` 约束：
/// - 正向 `FnMut(P)`（P 为 `ResMut<T>` 等 `SystemParam` 类型）驱动 `P` 由函数
///   签名正推，规避 `FnMut(P::Item)` 投影逆推的 E0283；
/// - `FnMut(P::Item<'a>)` 保证 `run` 实际调用时参数类型（`P::Item`）匹配。
///
/// [`FunctionSystem`] 的 [`System`] 实现与 [`IntoSystem`] 均仅约束
/// `F: SystemParamFunction<Marker>`，不直接对 `F` 写 `FnMut`，从而彻底解耦
/// 并消除 E0277（转换后 `F` 的 `FnMut` 约束与 [`FunctionSystem`] 实际
/// `FnMut(P::Item)` 约束不一致）。
pub trait SystemParamFunction<Marker>: Send + Sync + 'static {
    /// 参数元组类型（`(P0, P1, …)`），经 `get` 提取为 [`SystemParam::Item`]。
    type Param: SystemParam;
    /// 执行一次：参数已提取为 `Param::Item`（World 借用形态）。
    fn run<'w>(&mut self, param: <Self::Param as SystemParam>::Item<'w>);
}

/// 为 `fn(P0, P1, …)` 形态（0-4 参数）生成 [`SystemParamFunction`] impl。
///
/// `Marker` 取函数类型 `fn($($P,)*)`：其参数生命周期被 Rust 量化为 HRTB，
/// 故 `fn($($P,)*)` 类型本身 `'static`，使 `FunctionSystem<F, fn(…)>` 满足
/// `System: 'static`。
macro_rules! impl_system_function {
    ($($param: ident),*) => {
        #[allow(non_snake_case, clippy::too_many_arguments)]
        impl<Func, $($param: SystemParam),*> SystemParamFunction<fn($($param,)*)> for Func
        where
            Func: Send + Sync + 'static,
            // 正向 `FnMut(P)` 驱动 P 推断；`FnMut(SystemParamItem)` 中省略的
            // 生命周期被推断为独立高阶生命周期（与 `&'a mut Func` 的 `'a` 解耦），
            // 保证 `call_inner` 调用点 `&mut self` 借用与参数 `'w` 互不约束。
            for<'a> &'a mut Func:
                FnMut($($param),*) +
                FnMut($(SystemParamItem<$param>),*),
        {
            type Param = ($($param,)*);
            #[inline]
            fn run<'w>(&mut self, param_value: <Self::Param as SystemParam>::Item<'w>) {
                // 旧 ECS 方案 同款辅助函数：以 `impl FnMut(P)` 形式接受 self，让 P 在
                // 调用点统一为 `P::Item`（第二个 FnMut 约束承担）；第一个
                // FnMut(P) 约束驱动 P 由闭包签名正推（消 E0283）。
                fn call_inner<$($param,)*>(
                    mut f: impl FnMut($($param),*),
                    $($param: $param,)*
                ) {
                    f($($param),*)
                }
                let ($($param,)*) = param_value;
                call_inner(self, $($param),*)
            }
        }
    };
}
impl_system_function!();
impl_system_function!(P0);
impl_system_function!(P0, P1);
impl_system_function!(P0, P1, P2);
impl_system_function!(P0, P1, P2, P3);
impl_system_function!(P0, P1, P2, P3, P4);
impl_system_function!(P0, P1, P2, P3, P4, P5);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7, P8);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12);
impl_system_function!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13);
impl_system_function!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14
);
impl_system_function!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15
);
impl_system_function!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16
);

/// [`IntoSystem`] 宏：为 `fn(P0, P1, …)` 形态（0-4 参数）生成系统转换。
///
/// Marker 为函数类型 `fn($($P,)*)`（参数生命周期量化 → `'static`）。
/// `F` 仅需满足 `SystemParamFunction<fn($($P,)*)>`——`P` 的正向推断由
/// [`SystemParamFunction`] impl 的 `FnMut(P)` 约束承担，本 impl 不产生
/// 投影逆推，故 `Schedule::add_system` 的自由 `M` 可唯一确定（消 E0283）。
macro_rules! impl_into_system {
    ($($param: ident),*) => {
        impl<F, $($param: SystemParam + 'static),*> IntoSystem<fn($($param,)*)> for F
        where
            F: SystemParamFunction<fn($($param,)*)>,
        {
            type System = FunctionSystem<F, fn($($param,)*)>;

            fn into_system(self) -> Self::System {
                FunctionSystem::new(self)
            }
        }
    };
}
impl_into_system!();
impl_into_system!(P0);
impl_into_system!(P0, P1);
impl_into_system!(P0, P1, P2);
impl_into_system!(P0, P1, P2, P3);
impl_into_system!(P0, P1, P2, P3, P4);
impl_into_system!(P0, P1, P2, P3, P4, P5);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7, P8);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12);
impl_into_system!(P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13);
impl_into_system!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14
);
impl_into_system!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15
);
impl_into_system!(
    P0, P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15, P16
);

/// [`FunctionSystem`] 的 [`System`] 实现：按 `F::Param`（`SystemParam` 元组）
/// 惰性初始化并提取参数，调用 [`SystemParamFunction::run`]。
impl<F, Marker: 'static> System for FunctionSystem<F, Marker>
where
    F: SystemParamFunction<Marker>,
{
    fn name(&self) -> &'static str {
        std::any::type_name::<F>()
    }

    fn run(&mut self, world: &mut World) {
        // 首次运行惰性初始化参数状态（init_resource 幂等，不覆盖已存在的
        // 资源值；后续运行复用缓存状态）
        let state = self
            .state
            .get_or_insert_with(|| <F::Param as SystemParam>::init_state(world));
        // SAFETY: FunctionSystem::run 是参数提取的唯一入口，一次运行内各
        // 参数借用互斥（A9 契约；Res/ResMut 同型混用属框架误用，由调度器
        // 排他约束杜绝）
        let params = unsafe { <F::Param as SystemParam>::get(state, world) };
        self.func.run(params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 测试资源（blanket impl 自动实现 Resource；需 Default 供
    // init_state 惰性初始化）。命名字段避免与 ResMut 元组字段 `.0` 混淆 ----

    #[derive(Default, Debug, PartialEq)]
    struct Counter {
        n: u32,
    }

    #[derive(Default, Debug, PartialEq)]
    struct Other {
        tag: u32,
    }

    #[derive(Default, Debug, PartialEq)]
    struct A {
        v: u32,
    }
    #[derive(Default, Debug, PartialEq)]
    struct B {
        v: u32,
    }
    #[derive(Default, Debug, PartialEq)]
    struct C {
        v: u32,
    }
    #[derive(Default, Debug, PartialEq)]
    struct D {
        v: u32,
    }

    // ---- 测试系统（fn 0-4 参数）----

    fn inc_counter(mut c: ResMut<Counter>) {
        c.n += 1;
    }

    fn apply_offset(mut c: ResMut<Counter>, o: Res<Other>) {
        c.n += o.tag;
    }

    fn copy_counter(mut out: ResMut<Other>, src: Res<Counter>) {
        out.tag = src.n;
    }

    fn noop() {}

    fn sys_two(mut a: ResMut<A>, b: Res<B>) {
        a.v += b.v;
    }

    fn sys_three(mut a: ResMut<A>, b: Res<B>, mut c: ResMut<C>) {
        a.v += b.v;
        c.v += 1;
    }

    fn sys_four(mut a: ResMut<A>, b: Res<B>, mut c: ResMut<C>, d: Res<D>) {
        a.v += b.v;
        c.v += d.v;
    }

    #[test]
    fn res_mut_system_mutates_resource() {
        let mut world = World::new();
        world.insert_resource(Counter { n: 0 });
        // 探针：into_system 返回 FunctionSystem<F, fn(ResMut<Counter>)>，
        // 由编译器推断具体类型
        let mut sys = inc_counter.into_system();
        sys.run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(1));
    }

    #[test]
    fn res_mut_system_auto_init_missing_resource() {
        let mut world = World::new();
        // 未手动插入 Counter：init_state 惰性补默认值后自增
        inc_counter.into_system().run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(1));
    }

    #[test]
    fn init_state_keeps_existing_resource_value() {
        let mut world = World::new();
        world.insert_resource(Counter { n: 100 });
        inc_counter.into_system().run(&mut world);
        // 首次运行 init_resource 幂等：不覆盖既有值
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(101));
    }

    #[test]
    fn res_param_inits_missing_resource() {
        // 只读 Res 的 init_state 同样惰性补齐资源默认值
        let mut world = World::new();
        copy_counter.into_system().run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(0));
        assert_eq!(world.resource::<Other>().map(|o| o.tag), Some(0));
    }

    #[test]
    fn multi_param_system_reads_and_writes() {
        let mut world = World::new();
        world.insert_resource(Counter { n: 10 });
        world.insert_resource(Other { tag: 5 });
        apply_offset.into_system().run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(15));
    }

    #[test]
    fn read_only_param_observes_value() {
        let mut world = World::new();
        world.insert_resource(Counter { n: 42 });
        copy_counter.into_system().run(&mut world);
        // src.n（只读 Res Deref）→ out.tag（可变 ResMut DerefMut）
        assert_eq!(world.resource::<Other>().map(|o| o.tag), Some(42));
    }

    #[test]
    fn zero_param_system_runs() {
        let mut world = World::new();
        noop.into_system().run(&mut world);
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn two_three_four_param_systems() {
        let mut world = World::new();
        world.insert_resource(B { v: 3 });
        world.insert_resource(D { v: 7 });
        sys_two.into_system().run(&mut world);
        sys_three.into_system().run(&mut world);
        sys_four.into_system().run(&mut world);
        // A：0 + 3 → 3，+ 3 → 6，+ 3 → 9；C：0 + 1 → 1，+ 7 → 8
        assert_eq!(world.resource::<A>().map(|r| r.v), Some(9));
        assert_eq!(world.resource::<C>().map(|r| r.v), Some(8));
    }

    #[test]
    fn repeated_runs_accumulate() {
        let mut world = World::new();
        let mut system = inc_counter.into_system();
        system.run(&mut world);
        system.run(&mut world);
        system.run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(3));
    }

    #[test]
    fn system_name_is_fn_type_name() {
        let system = inc_counter.into_system();
        let name = system.name();
        assert!(!name.is_empty());
        assert!(name.contains("inc_counter"));
        // 普通系统非排他
        assert!(!system.exclusive());
    }

    #[test]
    fn boxed_dyn_system_trait_object() {
        let mut world = World::new();
        world.insert_resource(Counter { n: 100 });
        let mut system: Box<dyn System> = Box::new(inc_counter.into_system());
        system.run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(101));
        assert!(!system.exclusive());
    }

    #[test]
    fn closure_system_works() {
        let mut world = World::new();
        // 闭包同样实现 FnMut(ResMut<Counter>) → IntoSystem
        let mut system: Box<dyn System> = Box::new(
            (|mut c: ResMut<Counter>| {
                c.n += 10;
            })
            .into_system(),
        );
        system.run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(10));
    }

    // ---- 排他系统（ExclusiveSystem）----

    fn exclusive_inc(world: &mut World) {
        let counter = world.resource_mut::<Counter>().unwrap();
        counter.n += 100;
    }

    #[test]
    fn exclusive_system_flags_and_runs() {
        let mut world = World::new();
        world.init_resource::<Counter>();
        let mut system = exclusive_inc.into_system();
        assert!(system.exclusive());
        system.run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(100));
    }

    #[test]
    fn exclusive_system_trait_object() {
        let mut world = World::new();
        world.init_resource::<Counter>();
        let mut system: Box<dyn System> = Box::new(exclusive_inc.into_system());
        assert!(system.exclusive());
        system.run(&mut world);
        assert_eq!(world.resource::<Counter>().map(|c| c.n), Some(100));
    }
}
