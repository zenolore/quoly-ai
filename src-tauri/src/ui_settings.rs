//! 界面偏好持久化。
//!
//! 只存「界面开关」这类东西 —— 悬浮窗开关与位置、是否允许开机静默启动。
//! 业务数据（账号、积分、请求）全在 SQLite 里，这里不碰。
//!
//! 为什么单独一个文件而不是塞进数据库：这些值要在**窗口创建之前**读到
//! （悬浮窗一启动就要按上次的位置摆好），此时数据库还没初始化。一个几十字节
//! 的 JSON 比为此提前拉起 DB 简单得多。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSettings {
    /// 悬浮窗是否启用。关掉后连窗口都不创建。
    #[serde(default)]
    pub float_enabled: bool,
    /// 悬浮窗上次的位置（屏幕坐标）。None 表示用默认位置（右上角）。
    #[serde(default)]
    pub float_position: Option<(f64, f64)>,
    /// 是否已经执行过「首次启动默认开启开机自启」。
    ///
    /// 需要这个标记而不是每次启动都检查 `is_enabled()`：用户如果**故意**把自启
    /// 关掉，下次启动再按 `is_enabled()==false` 去「补开」就成了强奸意愿。
    /// 只在第一次（标记为 false）设一次默认值，之后完全听系统的。
    #[serde(default)]
    pub autostart_defaulted: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            float_enabled: false,
            float_position: None,
            autostart_defaulted: false,
        }
    }
}

fn path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".ai-toolkit").join("ui-settings.json")
}

pub fn load() -> UiSettings {
    let p = path();
    let Ok(raw) = std::fs::read_to_string(&p) else {
        return UiSettings::default();
    };
    // 文件坏了就当默认值，不要让一个手改坏的 JSON 卡住启动。
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save(settings: &UiSettings) -> Result<(), String> {
    let p = path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&p, raw).map_err(|e| format!("写入配置失败: {e}"))
}

/// 只改一个字段并落盘，返回写入后的完整设置。
pub fn update<F: FnOnce(&mut UiSettings)>(f: F) -> Result<UiSettings, String> {
    let mut s = load();
    f(&mut s);
    save(&s)?;
    Ok(s)
}
