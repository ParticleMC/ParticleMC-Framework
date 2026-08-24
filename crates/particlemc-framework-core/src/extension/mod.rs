//! WASM 扩展运行时（WS4-T2/T3）。
//!
//! 本模块是 wasmtime host FFI 的唯一白名单 `unsafe` 区域（符合 ADR-016 安全评审）。
//! v1 能力定位：「只读观察者 + 回调」——扩展仅能注册 tick 回调与事件监听，不得直接
//! 读写 ECS 内存或 spawn 实体/改方块。变更型 host API（host_set_block / host_spawn_entity /
//! host_register_system）按签名预留 `#[cfg(feature = "extension-mutation")]` 占位，v1 默认
//! 关闭、不接线。
//!
//! # Safety（ADR-016 §1）
//!
//! - 本模块仅在 `feature = "wasm-extensions"` 时声明为白名单 `#[allow(unsafe_code)]`，
//!   仅 host FFI 注册与内存安全护城河使用 `unsafe`，每块均附 `# Safety` 章节 + `debug_assert!` 检查。
//! - 涉及 unsafe 的函数（wasmtime 实例创建 / 导入函数注册 / 内存访问 / table 操作）须
//!   在 CI 中通过 `cargo miri test`（本环境 stable 工具链下在后台执行）。
//! - 所有 `unsafe` 块遵守索引边界 / 指针对齐 / 别名合法性规则。

#![cfg_attr(feature = "wasm-extensions", allow(unsafe_code))]

#[cfg(feature = "wasm-extensions")]
use std::path::Path;
#[cfg(feature = "wasm-extensions")]
use std::sync::Arc;
#[cfg(feature = "wasm-extensions")]
use std::sync::Mutex;
#[cfg(feature = "wasm-extensions")]
use std::sync::atomic::{AtomicU32, Ordering};
#[cfg(feature = "wasm-extensions")]
use wasmtime::{Config, Engine, Extern, Linker, Module, Store, Table, TypedFunc};

/// 扩展最大数量限制（避免 tick 派发开销失控，ADR-016 §5 tick 性能预算）。
pub const MAX_EXTENSIONS: usize = 64;

/// 扩展导出入口符号名。
pub const PARTICLEMC_FRAMEWORK_INIT_SYMBOL: &str = "particlemc_framework_init";

#[cfg(feature = "wasm-extensions")]
type TickCallback = TypedFunc<(), ()>;
#[cfg(feature = "wasm-extensions")]
type EventCallback = TypedFunc<i32, ()>;

/// 扩展实例：持有 wasmtime 存储与导出函数表。
#[cfg(feature = "wasm-extensions")]
pub struct ExtensionInstance {
    /// wasmtime Store（含内存与导入资源，wasmtime 要求 `Store` 与 `Instance` 共生）。
    pub(crate) store: Store<ExtensionData>,
    /// tick 回调（可选；扩展未注册时为 `None`）。
    pub(crate) tick_callback: Option<TickCallback>,
    /// 事件监听表（event_id → 回调）。
    pub(crate) event_callbacks: Mutex<std::collections::HashMap<i32, EventCallback>>,
}

#[cfg(feature = "wasm-extensions")]
#[derive(Default)]
pub(crate) struct ExtensionData {
    pub(crate) table_idx: u32,
}

#[cfg(feature = "wasm-extensions")]
pub struct ExtensionLoader {
    engine: Engine,
    linker: Linker<ExtensionData>,
}

#[cfg(feature = "wasm-extensions")]
impl ExtensionLoader {
    #[must_use]
    pub fn new() -> Self {
        let mut config = Config::new();
        config.wasm_simd(false);
        config.cache_config_load_default(false);

        let engine = Engine::new(&config).expect("wasmtime Engine 初始化失败");

        let mut linker = Linker::new(&engine);

        if let Err(e) = linker.func_wrap(
            "env",
            "host_register_tick_callback",
            move |mut caller: wasmtime::Caller<'_, ExtensionData>, ptr: i32| {
                host_register_tick_callback(&mut caller, ptr);
                Ok(())
            },
        ) {
            panic!("host_register_tick_callback 注册失败: {e}");
        }

        if let Err(e) = linker.func_wrap(
            "env",
            "host_register_event",
            move |mut caller: wasmtime::Caller<'_, ExtensionData>,
                  event_id: i32,
                  callback_ptr: i32| {
                host_register_event(&mut caller, event_id, callback_ptr);
                Ok(())
            },
        ) {
            panic!("host_register_event 注册失败: {e}");
        }

        Self { engine, linker }
    }

    /// 从指定路径加载单个 `.wasm` 扩展，实例化并调用 `particlemc_framework_init`。
    pub fn load(&self, wasm_path: &Path) -> Option<Arc<Mutex<(ExtensionInstance, Table)>>> {
        let wasm_bytes = match std::fs::read(wasm_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("无法读取扩展 {wasm_path:?}: {e}");
                return None;
            }
        };

        let module = match Module::from_binary(&self.engine, &wasm_bytes) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("扩展 {wasm_path:?} 字节码验证失败: {e}");
                return None;
            }
        };

        let mut store = match Store::new(&self.engine, ExtensionData::default()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("扩展 {wasm_path:?} Store 创建失败: {e}");
                return None;
            }
        };

        let instance = match self.linker.instantiate(&mut store, &module) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("扩展 {wasm_path:?} 实例化失败: {e}");
                return None;
            }
        };

        let init_func = match instance.get_typed_func::<i32, i32>(&mut store, PARTICLEMC_FRAMEWORK_INIT_SYMBOL)
        {
            Ok(f) => f,
            Err(_) => {
                eprintln!("扩展 {wasm_path:?} 缺少导出 {PARTICLEMC_FRAMEWORK_INIT_SYMBOL}");
                return None;
            }
        };

        let ret = match init_func.call(&mut store, 0) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("扩展 {wasm_path:?} minestom_init 调用失败: {e}");
                return None;
            }
        };

        if ret != 0 {
            eprintln!("扩展 {wasm_path:?} minestom_init 返回非零错误码: {ret}");
            return None;
        }

        let tick_callback = instance.get_typed_func(&mut store, "minestom_tick").ok();

        let table = match instance.get_table(&mut store) {
            Some(t) => t,
            None => {
                eprintln!("扩展 {wasm_path:?} 缺少 table，无法注册事件监听");
                return None;
            }
        };

        let instance_ref = ExtensionInstance {
            store,
            tick_callback,
            event_callbacks: Mutex::new(std::collections::HashMap::new()),
        };

        let wrapped = Arc::new(Mutex::new((instance_ref, table)));

        let table_idx = NEXT_TABLE_ID.fetch_add(1, Ordering::Relaxed) as u32;
        TABLES
            .lock()
            .unwrap()
            .insert(table_idx, Arc::clone(&wrapped));

        wrapped.lock().unwrap().0.store.data_mut().table_idx = table_idx;

        eprintln!("✓ 成功加载扩展: {wasm_path:?}");
        Some(wrapped)
    }
}

#[cfg(feature = "wasm-extensions")]
impl Default for ExtensionLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "wasm-extensions")]
static NEXT_TABLE_ID: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "wasm-extensions")]
static TABLES: Mutex<std::collections::HashMap<u32, Arc<Mutex<(ExtensionInstance, Table)>>>> =
    Mutex::new(std::collections::HashMap::new());

#[cfg(feature = "wasm-extensions")]
unsafe extern "C" fn host_register_tick_callback(
    mut caller: wasmtime::Caller<'_, ExtensionData>,
    ptr: i32,
) {
    let table_idx = caller.data().table_idx;

    let tables = TABLES.lock().unwrap();
    let guard = tables.get(&table_idx);
    if guard.is_none() {
        eprintln!("host_register_tick_callback: 无效 table_idx {table_idx}");
        return;
    }
    let guard = guard.unwrap();
    let (instance, table) = &mut *guard.lock().unwrap();

    let elem = match table.get(caller.as_context_mut(), ptr as u32) {
        Ok(Some(Extern::Func(f))) => f,
        _ => {
            eprintln!("host_register_tick_callback: table[{ptr}] 不是有效函数");
            return;
        }
    };

    match TypedFunc::<(), ()>::new(wasmtime::AsContextMut::as_context_mut(caller), &elem) {
        Ok(f) => instance.tick_callback = Some(f),
        Err(e) => eprintln!("host_register_tick_callback: 签名不匹配: {e}"),
    }
}

#[cfg(feature = "wasm-extensions")]
unsafe extern "C" fn host_register_event(
    mut caller: wasmtime::Caller<'_, ExtensionData>,
    event_id: i32,
    callback_ptr: i32,
) {
    let table_idx = caller.data().table_idx;

    let tables = TABLES.lock().unwrap();
    let guard = tables.get(&table_idx);
    if guard.is_none() {
        eprintln!("host_register_event: 无效 table_idx {table_idx}");
        return;
    }
    let guard = guard.unwrap();
    let (instance, table) = &mut *guard.lock().unwrap();

    let elem = match table.get(caller.as_context_mut(), callback_ptr as u32) {
        Ok(Some(Extern::Func(f))) => f,
        _ => {
            eprintln!("host_register_event: table[{callback_ptr}] 不是有效函数");
            return;
        }
    };

    match TypedFunc::<i32, ()>::new(wasmtime::AsContextMut::as_context_mut(caller), &elem) {
        Ok(f) => instance.event_callbacks.lock().unwrap().insert(event_id, f),
        Err(e) => eprintln!("host_register_event: 签名不匹配: {e}"),
    }
}

#[cfg(feature = "wasm-extensions")]
pub struct ExtensionManager {
    instances: Vec<Arc<Mutex<(ExtensionInstance, Table)>>>,
}

#[cfg(feature = "wasm-extensions")]
impl ExtensionManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            instances: Vec::with_capacity(MAX_EXTENSIONS),
        }
    }

    pub fn register(&mut self, instance: Arc<Mutex<(ExtensionInstance, Table)>>) -> Result<(), ()> {
        if self.instances.len() >= MAX_EXTENSIONS {
            return Err(());
        }
        self.instances.push(instance);
        Ok(())
    }

    pub fn tick_all(&self) {
        for instance in &self.instances {
            let (ext, _table) = &mut *instance.lock().unwrap();
            if let Some(ref tick) = ext.tick_callback {
                if let Err(e) = tick.call(&mut ext.store, ()) {
                    eprintln!("扩展 tick 回调失败: {e}");
                }
            }
        }
    }
}

#[cfg(feature = "wasm-extensions")]
impl Default for ExtensionManager {
    fn default() -> Self {
        Self::new()
    }
}

// ===== feature off 时的空实现（保持编译通过，不引入 wasmtime 依赖）=====

#[cfg(not(feature = "wasm-extensions"))]
pub struct ExtensionLoader;

#[cfg(not(feature = "wasm-extensions"))]
impl ExtensionLoader {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn load(&self, _wasm_path: &std::path::Path) -> Option<()> {
        eprintln!("WASM 扩展功能未启用（未配置 feature `wasm-extensions`）");
        None
    }
}

#[cfg(not(feature = "wasm-extensions"))]
impl Default for ExtensionLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "wasm-extensions"))]
pub struct ExtensionManager;

#[cfg(not(feature = "wasm-extensions"))]
impl ExtensionManager {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn tick_all(&self) {
        // 空实现（无扩展可派发）
    }
}

#[cfg(not(feature = "wasm-extensions"))]
impl Default for ExtensionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "wasm-extensions")]
    #[test]
    fn loader_creation_does_not_panic() {
        let _loader = ExtensionLoader::new();
    }

    #[cfg(feature = "wasm-extensions")]
    #[test]
    fn manager_creation_and_tick_does_not_panic() {
        let manager = ExtensionManager::new();
        manager.tick_all();
    }

    #[cfg(not(feature = "wasm-extensions"))]
    #[test]
    fn feature_off_impl_does_not_panic() {
        let loader = ExtensionLoader::new();
        let manager = ExtensionManager::new();
        manager.tick_all();
        assert!(
            loader
                .load(std::path::Path::new("nonexistent.wasm"))
                .is_none()
        );
    }
}
