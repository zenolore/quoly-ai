use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: String,
    pub body: Option<String>,
    pub date: Option<String>,
}

/// 检查更新
///
/// 本地补丁版（2.2.2-patch1）禁用了官方自动更新：官方 release 会覆盖本地的
/// 积分补领重试与局域网绑定改动，所以这里直接返回“无可用更新”并说明原因，
/// 而不是让 updater 去连 GitHub。tauri.conf.json 里 updater.active 也已设为 false。
#[tauri::command]
pub async fn check_update(_app: AppHandle) -> Result<UpdateInfo, String> {
    Ok(UpdateInfo {
        available: false,
        version: String::new(),
        body: Some(
            "本地补丁版已禁用官方自动更新：官方版本不含积分补领重试与局域网共享改动，\
             安装后会被覆盖回原版。如需升级，请重新用源码构建。"
                .into(),
        ),
        date: None,
    })
}

/// 下载并安装更新：补丁版禁用，避免把本地改动覆盖回官方版。
#[tauri::command]
pub async fn install_update(_app: AppHandle) -> Result<(), String> {
    Err("本地补丁版已禁用自动更新，避免覆盖积分补领与局域网共享改动。".into())
}
