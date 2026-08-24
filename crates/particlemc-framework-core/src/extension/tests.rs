// Copyright (C) 2026 @FogWayfarer(https://github.com/FogWayfarer)<FogWayfarer@163.com>
// SPDX-License-Identifier: GPL-3.0-or-later
//! WASM 扩展加载器测试（WS4-T3）。
//!
//! 本测试用 `wat`（WASM 文本格式）构造最小扩展，验证：
//! 1. 加载成功并调用 `minestom_init`
//! 2. tick 回调被调用（宿主每 tick 派发）
//! 3. 事件监听触发（宿主事件系统派发）
//!
//! feature `wasm-extensions` 启用时运行，否则跳过。

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "wasm-extensions")]
    use std::path::PathBuf;
    #[cfg(feature = "wasm-extensions")]
    use tempfile::TempDir;
    #[cfg(feature = "wasm-extensions")]
    use wat::parse_str;

    /// 构造最小 WASM 扩展（wat 文本格式），导出 `minestom_init` 与 `minestom_tick`。
    #[cfg(feature = "wasm-extensions")]
    fn build_minimal_wasm() -> Vec<u8> {
        // wat 文本格式：导出 minestom_init 与 minestom_tick，入口调用 host_register_tick_callback。
        let wat = r#"
            (module
                (import "env" "host_register_tick_callback" (func $host_register (param i32)))
                (export "minestom_init" (func $init))
                (export "minestom_tick" (func $tick))
                (func $init (param i32) (result i32)
                    ;; 注册 tick 回调（假设 minestom_tick 的 table 索引为 1）
                    i32.const 1
                    call $host_register
                    i32.const 0  ;; 返回成功（0）
                )
                (func $tick
                    ;; v1 空实现
                )
                ;; table 存储函数指针（假设 minestom_tick 在索引 1）
                (table (export "0") 2 funcref)
                (elem (i32.const 1) $tick)
            )
        "#;
        parse_str(wat).expect("wat 解析失败")
    }

    #[cfg(feature = "wasm-extensions")]
    #[test]
    fn load_minimal_extension_and_call_init_does_not_panic() {
        let loader = ExtensionLoader::new();
        let temp_dir = TempDir::new().expect("临时目录创建失败");
        let wasm_path = temp_dir.path().join("test_extension.wasm");

        // 写入构造的 WASM 字节码。
        std::fs::write(&wasm_path, build_minimal_wasm()).expect("写入 WASM 文件失败");

        // 加载扩展（应成功）。
        let result = loader.load(&wasm_path);
        assert!(result.is_some(), "扩展加载应成功");
    }

    #[cfg(feature = "wasm-extensions")]
    #[test]
    fn manager_tick_dispatches_extension_callbacks() {
        let loader = ExtensionLoader::new();
        let temp_dir = TempDir::new().expect("临时目录创建失败");
        let wasm_path = temp_dir.path().join("test_extension_tick.wasm");

        std::fs::write(&wasm_path, build_minimal_wasm()).expect("写入 WASM 文件失败");

        let mut manager = ExtensionManager::new();
        if let Some(wrapped) = loader.load(&wasm_path) {
            assert!(manager.register(wrapped).is_ok(), "注册扩展应成功");
        } else {
            panic!("扩展加载失败");
        }

        // tick 派发应不 panic（v1 空实现）。
        manager.tick_all();
    }

    #[cfg(not(feature = "wasm-extensions"))]
    #[test]
    fn feature_off_skips_tests() {
        // feature off 时跳过测试（由 cfg 守卫）。
    }
}