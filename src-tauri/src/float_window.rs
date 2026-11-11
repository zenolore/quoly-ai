//! 桌面悬浮窗 —— 竖向圆角条。
//!
//! 无边框、透明、置顶、不进任务栏、可拖动、位置记忆。窗口标签 `float`，加载
//! 同一份前端产物，靠 URL hash `#/float` 分流到悬窗组件 —— 单独开一个 HTML
//! 入口要多一套构建配置，而这里只有四组数字，不值得。
//!
//! 为什么是竖的：横版卡片会占掉桌面角落一整块矩形；竖条窄，能贴边当侧栏。
//! 宽高卡死（92×276 + 每边 6px 透明留白），不提供缩放——能拖大拖小只会把它
//! 变成一个半残的主界面。

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

use crate::ui_settings;

pub const FLOAT_WINDOW: &str = "float";
/// 内容区尺寸。窗口比它大 12px，四周各留 6px 给投影和圆角抗锯齿，
/// 否则阴影会被窗口边界切掉一圈硬边。
const CONTENT_WIDTH: f64 = 92.0;
const CONTENT_HEIGHT: f64 = 276.0;
const SHADOW_PAD: f64 = 6.0;
const WIDTH: f64 = CONTENT_WIDTH + SHADOW_PAD * 2.0;
const HEIGHT: f64 = CONTENT_HEIGHT + SHADOW_PAD * 2.0;

/// 创建悬浮窗（已存在则直接返回）。`visible` 决定建好后是否立刻显形。
pub fn create<R: Runtime>(app: &AppHandle<R>, visible: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(FLOAT_WINDOW) {
        return if visible {
            window.show().map_err(|e| format!("显示悬浮窗失败: {e}"))
        } else {
            Ok(())
        };
    }
    let settings = ui_settings::load();

    let builder = WebviewWindowBuilder::new(
        app,
        FLOAT_WINDOW,
        // 只给 index.html，**不要**写 `index.html#/float`：`WebviewUrl::App`
        // 传的是 PathBuf，`#` 会被当成路径字符编码掉，WebView2 拿到的是一个
        // 不存在的地址，窗口里直接显示「无法访问此页面」。前端靠窗口标签
        // （label == "float"）区分是主界面还是悬浮窗，不依赖 URL。
        WebviewUrl::App("index.html".into()),
    )
    .title("Quoly AI 悬浮窗")
    .inner_size(WIDTH, HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    // 先建后摆再显示，避免在默认位置闪一下才跳到上次的位置。
    .visible(false);

    let window = builder.build().map_err(|e| format!("创建悬浮窗失败: {e}"))?;

    // 位置必须在**建好之后**按真实尺寸算，不能在建之前用常量估。
    // 实际踩到的坑：`inner_size(104, 288)` 建出来的窗口却是 136×288（WebView2
    // 的透明窗口有最小宽度），而位置按 104 算的，结果右边缘出屏 16px、看起来
    // 像「没显示」。改用 `outer_size()` 的真值，多宽都不会算错。
    //
    // 全程用物理像素：`monitor.size()/position()` 都是物理量，`outer_size()` 也是，
    // 不再和 scale_factor 混用——上一版就是混着算，在 3840 屏上偏了 40px。
    let size = window.outer_size().map_err(|e| format!("读取悬浮窗尺寸失败: {e}"))?;
    let (w, h) = (size.width as f64, size.height as f64);

    let target = match settings.float_position {
        Some((x, y)) => (x, y),
        None => default_position(app, w, h),
    };
    let (x, y) = clamp_to_screen(app, w, h, target.0, target.1);
    window
        .set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .map_err(|e| format!("设置悬浮窗位置失败: {e}"))?;

    // 透明窗口在 Windows 上偶尔留上一次合成的残影，置顶重设一次更稳。
    let _ = window.set_always_on_top(true);
    if visible {
        window.show().map_err(|e| format!("显示悬浮窗失败: {e}"))?;
    }
    Ok(())
}

/// 把坐标夹到「整个窗口都在主屏内」。夹而不是丢弃：用户特意摆的位置尽量
/// 保留，只是不许它掉到看不见的地方。
fn clamp_to_screen<R: Runtime>(
    app: &AppHandle<R>,
    w: f64,
    h: f64,
    x: f64,
    y: f64,
) -> (f64, f64) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return (x, y);
    };
    let pos = monitor.position();
    let size = monitor.size();
    const MARGIN: f64 = 6.0;
    let min_x = pos.x as f64 + MARGIN;
    let min_y = pos.y as f64 + MARGIN;
    let max_x = (pos.x as f64 + size.width as f64 - w - MARGIN).max(min_x);
    let max_y = (pos.y as f64 + size.height as f64 - h - MARGIN).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

/// 右侧竖排，垂直方向放在上五分之一处——避开任务栏和常见的右下角通知区，
/// 又比正中间更靠近视线落点。
fn default_position<R: Runtime>(app: &AppHandle<R>, w: f64, h: f64) -> (f64, f64) {
    let Ok(Some(monitor)) = app.primary_monitor() else {
        return (1200.0, 80.0);
    };
    let pos = monitor.position();
    let size = monitor.size();
    let x = pos.x as f64 + size.width as f64 - w - 16.0;
    let y = pos.y as f64 + size.height as f64 * 0.18;
    let _ = h;
    (x, y)
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(FLOAT_WINDOW) {
        window.hide().map_err(|e| format!("隐藏悬浮窗失败: {e}"))?;
    }
    Ok(())
}

pub fn destroy<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(FLOAT_WINDOW) {
        let _ = window.close();
    }
}

/// 记下当前坐标。拖动结束时前端调一次——不跟随移动事件，那会一秒写几十次盘。
///
/// 存物理像素，和 `create` 里的 `set_position(PhysicalPosition)` 对齐。
/// 不除 scale_factor：除过再乘回去，只要中间有一次缩放变化就会漂。
pub fn remember_position<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let Some(window) = app.get_webview_window(FLOAT_WINDOW) else {
        return Ok(());
    };
    let pos = window
        .outer_position()
        .map_err(|e| format!("读取窗口位置失败: {e}"))?;
    ui_settings::update(|s| s.float_position = Some((pos.x as f64, pos.y as f64)))?;
    Ok(())
}

/// 静默启动时主窗口不该露脸。`--silent` 由开机自启项带进来。
pub fn launched_silently() -> bool {
    std::env::args().any(|a| a == "--silent")
}
