import { invoke } from "@tauri-apps/api/core";

/**
 * 界面偏好：桌面悬浮窗 + 开机静默启动。
 *
 * 全部走自定义命令而不是 `@tauri-apps/plugin-autostart` 的 JS 包：
 * 那个包只为了三个调用再加一份前端依赖，而 Rust 侧本来就要注册插件，
 * 直接在命令里转一次更省事，也让「开关+状态」能在同一次往返里读全。
 */
export interface ShellSettings {
  floatEnabled: boolean;
  floatVisible: boolean;
  autostartEnabled: boolean;
}

export const shellApi = {
  settings: () => invoke<ShellSettings>("shell_settings"),
  setFloatEnabled: (enabled: boolean) =>
    invoke<ShellSettings>("set_float_enabled", { enabled }),
  showFloat: () => invoke<ShellSettings>("float_show"),
  hideFloat: () => invoke<void>("float_hide"),
  rememberFloatPosition: () => invoke<void>("float_remember_position"),
  setAutostart: (enabled: boolean) =>
    invoke<ShellSettings>("set_autostart", { enabled }),
  openMainWindow: () => invoke<void>("open_main_window"),
  openLogFolder: () => invoke<void>("open_log_folder"),
};
