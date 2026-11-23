// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 给本进程指定**专属的** WebView2 调试端口。
    //
    // 为什么需要（2026-11-19 实测定位）：
    //
    //   本机用户环境变量里有一条全局设置：
    //     WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223
    //
    //   它让**所有** WebView2 应用都去抢 9223，谁先启动谁占。实测当时占着
    //   9223 的是 Clash Verge（小哇调度台），于是本程序根本绑不上调试端口，
    //   外部看护脚本(ait_watcher)连进去只会看到别的应用，返回
    //   "Command credits_dashboard not found"，用量采集停了 19 小时。
    //
    //   更糟的是：那个脚本每次连 CDP 都会把目标窗口从最小化状态唤醒，
    //   用户感受是"窗口自己跳出来"。
    //
    // 解法：在进程启动的最早时刻，用 set_var 覆盖这个值。
    //   - 只影响本进程及其子进程（WebView2 是子进程），不污染系统环境变量，
    //     别的应用（Clash Verge 等）该用 9223 继续用，互不干扰；
    //   - 端口固定为 9333，看护脚本里 CDP_CANDIDATES 首位就是它，天然对上；
    //   - 9333 目前无人使用（已确认）。
    //
    // 必须在 run() 之前设置：WebView2 环境在第一次创建 webview 时就固化了，
    // 之后再改环境变量不生效。
    //
    // 注意：不加 --remote-debugging-address，WebView2 默认只监听
    // 127.0.0.1 与 [::1]（实测如此），不会暴露到局域网。
    #[cfg(windows)]
    {
        const DEBUG_PORT: &str = "9333";
        // 默认 9333；沙箱验证时用 QUOLY_DEBUG_PORT 指定别的端口，
        // 否则沙箱实例和生产实例会抢同一个调试端口，连上去分不清是谁。
        let debug_port = std::env::var("QUOLY_DEBUG_PORT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or_else(|| DEBUG_PORT.to_string());
        // 覆盖为只含本程序端口的参数，避免继承全局那条 9223。
        std::env::set_var(
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            format!("--remote-debugging-port={debug_port}"),
        );
    }

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
