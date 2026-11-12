// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 注意：tauri/tao 在 Windows 上要求事件循环必须在主线程初始化。
    // 曾尝试把 run() 放到 32 MiB 大栈子线程跑（为了解 0xc0000409 栈溢出），
    // 结果 tao 直接 panic："event loop outside of the main thread"，
    // 程序一启动就退出——所以这里必须留在主线程。
    //
    // 栈溢出的真正修复在 gateway.rs：
    // - 大 body 解析线程栈 16 MiB → 4 MiB（预留 32 MiB → 12 MiB），
    // - 解析槽位 2 → 3，线程创建失败时回退到当前线程解析。
    ai_toolkit_lib::run()
}
