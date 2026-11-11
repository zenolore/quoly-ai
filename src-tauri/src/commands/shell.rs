//! 界面偏好相关的命令：悬浮窗开关、开机静默启动。
//!
//! 这些命令不碰数据库，只操作 `ui-settings.json`、窗口和系统自启项，
//! 所以单独成文件，别和 app.rs 的业务配置混一起。

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::{float_window, ui_settings};

/// 悬浮窗 + 自启 的当前状态，一次读全，省得前端连发三个命令。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellSettings {
    /// 悬浮窗是否启用（持久化开关）。
    pub float_enabled: bool,
    /// 悬浮窗此刻是否真的可见（可能被托盘临时隐藏）。
    pub float_visible: bool,
    /// 是否已加入开机启动项。以系统状态为准。
    pub autostart_enabled: bool,
}

fn read<R: Runtime>(app: &AppHandle<R>) -> ShellSettings {
    let settings = ui_settings::load();
    let float_visible = app
        .get_webview_window(float_window::FLOAT_WINDOW)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    // 系统的自启状态才是真相：用户可能在任务管理器里手动禁用过，
    // 本地 JSON 说「开着」也白搭。
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    ShellSettings {
        float_enabled: settings.float_enabled,
        float_visible,
        autostart_enabled,
    }
}

#[tauri::command]
pub async fn shell_settings<R: Runtime>(app: AppHandle<R>) -> Result<ShellSettings, String> {
    Ok(read(&app))
}

/// 开/关悬浮窗。开启时立即创建并显示，关闭时持久化并销毁窗口。
#[tauri::command]
pub async fn set_float_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<ShellSettings, String> {
    ui_settings::update(|s| s.float_enabled = enabled)?;
    if enabled {
        float_window::create(&app, true)?;
    } else {
        float_window::destroy(&app);
    }
    // 销毁是异步的，稍等一下再读可见性；读不到就算 false，前端只拿来做图标状态。
    std::thread::sleep(std::time::Duration::from_millis(80));
    Ok(read(&app))
}

/// 悬浮窗拖动结束后记位置。前端在 `startDragging()` 解析后调用一次。
#[tauri::command]
pub async fn float_remember_position<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    float_window::remember_position(&app)
}

/// 悬窗右上角的 ×：只隐藏，不动持久化开关，下次启动仍会按开关回来。
#[tauri::command]
pub async fn float_hide<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    float_window::hide(&app)
}

/// 托盘「显示悬浮窗」用：临时显示，不改持久化开关。
#[tauri::command]
pub async fn float_show<R: Runtime>(app: AppHandle<R>) -> Result<ShellSettings, String> {
    // 开关是关的，但从托盘点了「显示」——那就把开关一并打开，
    // 否则窗口显示出来了、设置页却写着「关闭」，下次重启又消失。
    if !ui_settings::load().float_enabled {
        ui_settings::update(|s| s.float_enabled = true)?;
    }
    float_window::create(&app, true)?;
    Ok(read(&app))
}

/// 开/关开机自启。
///
/// 开启时带 `--silent`：开机拉起进程但不弹主窗口，只留托盘图标。这是
/// 「静默启动到托盘」的全部实现——比另建开机任务简单，也不容易被安全软件
/// 当成可疑计划任务盯上。
#[tauri::command]
pub async fn set_autostart<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<ShellSettings, String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| format!("开启开机自启失败: {e}"))?;
    } else {
        manager.disable().map_err(|e| format!("关闭开机自启失败: {e}"))?;
    }
    Ok(read(&app))
}

/// 打开日志所在目录。
///
/// 路径由主进程算，前端不猜 `~`：打包后的进程环境里 HOME 未必是用户以为的那个，
/// 而 `credits/trail.rs` 写日志用的是 `dirs::home_dir()`，两边必须同一处。
#[tauri::command]
pub async fn open_log_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    use tauri_plugin_shell::ShellExt;
    let dir = dirs::home_dir()
        .ok_or_else(|| "取不到用户目录".to_string())?
        .join(".ai-toolkit");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return Err(format!("创建目录失败: {error}"));
    }
    // shell 插件的 open 在 v2 里标了 deprecated（推荐 tauri-plugin-opener），
    // 但项目已经依赖 shell 做外链打开，这里复用它，不为一个「打开文件夹」再加插件。
    #[allow(deprecated)]
    app.shell()
        .open(dir.to_string_lossy().to_string(), None)
        .map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(())
}

/// 悬窗和托盘的「打开主界面」都走这里。
#[tauri::command]
pub async fn open_main_window<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    crate::tray::restore_main_window(&app);
    Ok(())
}

/// setup 期间调用：按持久化开关把悬浮窗建出来。
///
/// **当前停用**：WebView2 下让悬窗加载同一份前端产物走不通（窗口里显示
/// 「无法访问此页面」）。设置入口已收起，这里也直接不建，免得旧的
/// `floatEnabled: true` 残留又弹一个空白窗出来。
///
/// 代码保留：`float_window.rs` 和命令都还在，等悬窗改成独立 HTML 入口
/// （vite 多页）之后再打开这一个开关即可。
pub fn create_float_if_enabled<R: Runtime>(app: &tauri::App<R>, visible: bool) {
    let _ = (app, visible);
}

/// setup 期间调用：首次运行默认开启开机自启（静默进托盘）。
///
/// 只做一次。用 `autostart_defaulted` 标记而不是每次查 `is_enabled()`：
/// 用户主动关掉自启后再被「补开」是很烦人的行为。第一次没写成的话标记也
/// 不落盘，下次启动还会再试，直到成功或被用户手动关掉。
pub fn apply_default_autostart<R: Runtime>(app: &tauri::App<R>) {
    let settings = ui_settings::load();
    if settings.autostart_defaulted {
        return;
    }
    if let Err(error) = app.autolaunch().enable() {
        eprintln!("默认开启开机自启失败: {error}");
        return;
    }
    if let Err(error) = ui_settings::update(|s| s.autostart_defaulted = true) {
        eprintln!("记录自启默认值失败: {error}");
    }
}
