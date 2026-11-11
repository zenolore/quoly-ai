use crate::{
    app_state::AppState,
    credits::{
        gateway::ModelHealthReport,
        providers::{OAuthPoll, OAuthStart},
        types::*,
    },
};
use serde_json::{json, Value};
use tauri::{Manager, State};

#[tauri::command]
pub async fn credits_import_local(
    state: State<'_, AppState>,
    platform: Platform,
    region: Option<CreditRegion>,
) -> Result<Account, String> {
    let region = region.unwrap_or_default();
    let credentials = match platform {
        Platform::MinimaxCode => crate::credits::providers::minimax::local_credentials_for_region(region)?
            .ok_or_else(|| format!("本机未找到 MiniMax Code {}登录信息，请先登录对应区域的 MiniMax Code 或 mcode CLI", if region == CreditRegion::Cn { "国内版" } else { "国际版" }))?,
        Platform::Traework => match crate::credits::providers::local_credentials_for_region(region) {
            Ok(Some(credentials)) => credentials,
            Ok(None) => {
                return Err(format!("本机未找到 TRAE {}桌面或 CLI 登录信息，请先登录对应区域的 TRAE", if region == CreditRegion::Cn { "国内版" } else { "国际版" }));
            }
            Err(message) => {
                return Err(message);
            }
        },
        _ => return Err("该平台暂不支持本机导入，请使用官方登录".into()),
    };
    state.credits.retry_secret_access()?;
    let account = state
        .credits
        .add_account(platform, String::new(), credentials)
        .await;
    account
}

#[tauri::command]
pub fn credits_dashboard(state: State<'_, AppState>) -> Result<Dashboard, String> {
    state.credits.dashboard()
}

#[tauri::command]
pub fn credits_set_view_active(state: State<'_, AppState>, app: tauri::AppHandle, active: bool) -> Result<(), String> {
    let active = active && app.get_webview_window("main").is_some_and(|window|
        window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
            && !window.is_minimized().unwrap_or(true));
    state.credits.set_credits_view_active(active)
}

#[tauri::command]
pub fn credits_network_restored(state: State<'_, AppState>) -> Result<(), String> {
    state.credits.credits_network_restored()
}

#[tauri::command]
pub async fn credits_set_account_enabled(
    state: State<'_, AppState>,
    account_id: String,
    enabled: bool,
) -> Result<(), String> {
    state.credits.set_enabled(&account_id, enabled).await
}
#[tauri::command]
pub async fn credits_delete_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    state.credits.retry_secret_access()?;
    state.credits.delete_account(&account_id).await
}
#[tauri::command]
pub async fn credits_refresh(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<Account>, String> {
    state.credits.retry_secret_access()?;
    state.credits.refresh(account_id.as_deref()).await
}
#[tauri::command]
pub async fn credits_claim(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<RewardOutcome>, String> {
    state.credits.retry_secret_access()?;
    state.credits.claim(account_id.as_deref()).await
}

/// Run the WorkBuddy growth-center automation (travel, tasks, streak, boxes).
#[tauri::command]
pub async fn credits_run_growth(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> Result<Vec<crate::credits::growth::GrowthOutcome>, String> {
    state.credits.retry_secret_access()?;
    state.credits.run_growth(account_id.as_deref()).await
}

/// Read-only growth snapshot for one account (tasks, energy, travel, streak).
#[tauri::command]
pub async fn credits_growth_snapshot(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<crate::credits::growth::GrowthSnapshot, String> {
    state.credits.growth_snapshot(&account_id).await
}

/// Per-day growth totals for the history table (default 14 days).
#[tauri::command]
pub async fn credits_growth_daily(
    state: State<'_, AppState>,
    days: Option<u32>,
) -> Result<Vec<crate::credits::GrowthDay>, String> {
    state.credits.growth_daily(days.unwrap_or(14).clamp(1, 90))
}

/// Per-day token / request totals for the overview trend chart.
///
/// Returns chronological order (oldest first) so the chart can plot it
/// directly. Capped at 90 days: the aggregation scans every request in the
/// window, and a year of traffic is not something this chart is asked for.
#[tauri::command]
pub async fn credits_usage_daily(
    state: State<'_, AppState>,
    days: Option<u32>,
) -> Result<Vec<crate::credits::types::UsageDay>, String> {
    state.credits.usage_daily(days.unwrap_or(30).clamp(1, 90))
}

/// Which accounts are ready for growth automation, and why the others are not.
#[tauri::command]
pub async fn credits_growth_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<crate::credits::GrowthAccountState>, String> {
    state.credits.growth_accounts().await
}

/// Activate buddy-less accounts so they can earn growth rewards.
///
/// 解析一个账号文件，只回报发现了什么，不写库。界面上「选择文件」先调这个。
#[tauri::command]
pub fn credits_parse_account_file(
    state: State<'_, AppState>,
    content: String,
) -> Result<crate::credits::account_file::ParseReport, String> {
    let existing = state.credits.existing_user_ids()?;
    crate::credits::account_file::parse(&content, &existing)
}

/// 真正导入。逐个交给 add_account 校验——它会调平台接口确认凭据有效才落库。
#[tauri::command]
pub async fn credits_import_accounts(
    state: State<'_, AppState>,
    content: String,
    label_suffix: Option<String>,
) -> Result<Vec<ImportOutcome>, String> {
    let suffix = label_suffix.unwrap_or_default();
    let existing = state.credits.existing_user_ids()?;
    let parsed = crate::credits::account_file::parse(&content, &existing)?;
    state.credits.import_parsed(parsed, &suffix).await
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub nickname: String,
    pub label: String,
    pub ok: bool,
    pub message: String,
}

/// `accountIds` is required so this can only run against an explicit list; there
/// is no "all accounts" default for an operation that sends telemetry.
#[tauri::command]
pub async fn credits_activate_accounts(
    state: State<'_, AppState>,
    account_ids: Vec<String>,
) -> Result<Vec<crate::credits::activate::ActivationResult>, String> {
    if account_ids.is_empty() {
        return Err("请先选择要激活的账户".into());
    }
    if account_ids.len() > 5 {
        return Err("单次最多激活 5 个账户，请分批观察".into());
    }
    state.credits.activate_accounts(account_ids).await
}

/// Recent activation attempts, for the audit view.
#[tauri::command]
pub async fn credits_activations(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<(String, String, serde_json::Value, i64)>, String> {
    state.credits.activations(limit.unwrap_or(50))
}
#[tauri::command]
pub async fn credits_save_settings(
    state: State<'_, AppState>,
    settings: ProxySettings,
) -> Result<ProxySettings, String> {
    state.credits.save_settings(settings).await
}
#[tauri::command]
pub async fn credits_start_proxy(state: State<'_, AppState>) -> Result<GatewayStatus, String> {
    let settings = state.credits.store.settings()?;
    state.credits.start_proxy_for_user(settings.port).await
}
#[tauri::command]
pub async fn credits_stop_proxy(state: State<'_, AppState>) -> Result<GatewayStatus, String> {
    state.credits.gateway.stop().await
}
#[tauri::command]
pub async fn credits_proxy_key(state: State<'_, AppState>) -> Result<String, String> {
    state.credits.retry_gateway_key_access()?;
    state.credits.gateway_key()
}
#[tauri::command]
pub async fn credits_probe_proxy(state: State<'_, AppState>) -> Result<String, String> {
    state.credits.retry_gateway_key_access()?;
    state.credits.probe_proxy().await
}

/// Live per-model health for the panel: which routes on which platform are
/// actually answering, how fast, and which are temporarily quarantined.
#[tauri::command]
pub async fn credits_model_health(
    state: State<'_, AppState>,
) -> Result<Vec<ModelHealthReport>, String> {
    Ok(state.credits.gateway.model_health())
}
#[tauri::command]
pub async fn credits_oauth_start(
    state: State<'_, AppState>,
    platform: Platform,
    region: Option<CreditRegion>,
) -> Result<OAuthStart, String> {
    state
        .credits
        .provider
        .start_login(platform, region.unwrap_or_default())
        .await
        .map_err(|e| e.message)
}
#[tauri::command]
pub async fn credits_oauth_poll(
    state: State<'_, AppState>,
    flow_id: String,
) -> Result<Value, String> {
    match state
        .credits
        .provider
        .poll_login(&flow_id)
        .await
        .map_err(|e| e.message)?
    {
        OAuthPoll::Pending => Ok(json!({"status":"pending"})),
        OAuthPoll::Failed { message } => Ok(json!({"status":"failed","message":message})),
        OAuthPoll::Completed {
            platform,
            user_id: _,
            label,
            credentials,
        } => {
            state.credits.retry_secret_access()?;
            let account = state
                .credits
                .add_account(platform, label, credentials)
                .await?;
            Ok(json!({"status":"completed","account":account}))
        }
    }
}
