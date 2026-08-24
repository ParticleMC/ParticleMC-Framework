//! 扩展示例：最小起步模板（WS4-T3 G7）。
//!
//! 本 crate 编译为 `.wasm` 模块，导出 `minestom_init` 入口，
//! 注册 tick 回调与事件监听，展示扩展作者最小实现。
//!
//! # 编译为 WASM
//!
//! ```bash
//! cargo build --release --target wasm32-unknown-unknown
//! cp target/wasm32-unknown-unknown/release/extension_hello.wasm ../../extensions/
//! ```

#![no_std]

// panic 处理（WASM 环境需提供）。
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// 最小 tick 回调（v1 仅演示，不做实际逻辑）。
#[no_mangle]
extern "C" fn minestom_tick() {
    // v1 空实现：宿主每 tick 调用一次。
    // 后续可在此处扩展逻辑（如递增计数器）。
}

// 最小事件监听回调（v1 仅演示，不做实际逻辑）。
#[no_mangle]
extern "C" fn on_event(event_id: i32) {
    // v1 空实现：宿主在事件发生时调用。
    // 后续可在此处按 event_id 分发处理。
}

// 入口函数：注册 tick 回调与事件监听。
#[no_mangle]
extern "C" fn minestom_init(api: i32) -> i32 {
    // v1 刻意忽略 `api` 参数（ADR-016 §5）。
    let _ = api;

    unsafe {
        // 注册 tick 回调（参数为函数指针，即 table 索引）。
        // v1 简化：实际需通过 table 索引获取函数指针，这里硬编码 1 为假设索引。
        host_register_tick_callback(1);

        // 注册事件监听（event_id=0 表示 PlayerJoin，假设映射）。
        host_register_event(0, 2); // 回调函数指针假设为 2
    }

    0 // 成功
}

// host 导入函数声明（宿主侧签名必须匹配）。
extern "C" {
    fn host_register_tick_callback(ptr: i32);
    fn host_register_event(event_id: i32, callback_ptr: i32);
}