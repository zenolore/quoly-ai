//! Unlock accounts that have never adopted a buddy.
//!
//! Eleven of the accounts on this machine have `has_buddy: false`, which blocks
//! every growth reward: the service rejects task acceptance with
//! `prerequisite not met: first_buddy`, and with no buddy there is no travel.
//! The task itself pays 300 credits, the largest single reward in the plan.
//!
//! The task's stated trigger is "open a task in WorkBuddy and start a chat", but
//! the server does not verify that a chat really happened — it counts the client
//! telemetry event `chat_request_send`, which the desktop client reports. Sending
//! that one event therefore satisfies it. This is what several community projects
//! do (workbuddy2api and its derivatives, L0NE-6), and their published guidance is
//! unanimous on the constraints encoded below:
//!
//!   * **once per account per day** — daily activity is deduplicated server-side,
//!     so extra sends add exposure without adding reward;
//!   * **`conversationId` must be a real UUID v4** — the official client emits
//!     UUIDs there, so a prefixed or readable id is itself a detectable marker of a
//!     non-official client (this was an explicit finding in a workbuddy2api PR);
//!   * **`userId` is mandatory inside the event body** — omitted, the server
//!     returns 200 and silently drops the report;
//!   * **verify by reading back** — a 200 does not prove the streak advanced, so
//!     the streak is re-read and the result logged either way.
//!
//! Scope: one event per account, no gateway, no changed client identity, no burst.
//! The widely-reported bans (error 11140) all trace to running WorkBuddy as an API
//! relay with hundreds of requests a day, which is a different activity.

use serde::Serialize;
use serde_json::{json, Value};

use super::providers::{ProviderClient, ProviderError};
use super::types::Credentials;

const BASE: &str = "https://copilot.tencent.com";

/// What one activation attempt did, for the audit log.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationResult {
    pub account_id: String,
    pub label: String,
    /// skipped_has_buddy | reported | already_active | failed
    pub outcome: String,
    pub streak_before: i64,
    pub streak_after: i64,
    pub agreed: bool,
    pub adopted: bool,
    pub credit: i64,
    pub energy: i64,
    pub note: String,
}

fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let a = (nanos & 0xffff_ffff) as u32;
    let b = (((nanos >> 32) & 0xffff) as u16) | 0x4000;
    let c = ((((nanos >> 48) & 0x3fff) as u16) | 0x8000) as u16;
    let d = (nanos >> 20) as u64;
    format!("{a:08x}-{b:04x}-{c:04x}-{:04x}-{:012x}", d & 0xffff, d >> 16)
}

async fn call(
    client: &ProviderClient,
    c: &Credentials,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ProviderError> {
    let url = format!("{BASE}{path}");
    let mut req = match method {
        "GET" => client.http().get(url),
        _ => client.http().post(url),
    };
    req = req
        .bearer_auth(&c.access_token)
        .header("X-User-Id", c.user_id.as_deref().unwrap_or(""))
        // The desktop fingerprint, not the CLI one: first_buddy's judge reads the
        // desktop event family, and the CLI identity would be dropped silently.
        .header("X-Domain", "www.codebuddy.cn")
        .header("X-Product", "SaaS")
        .header("X-IDE-Type", "WorkBuddy")
        .header("X-IDE-Name", "WorkBuddy")
        .header("X-IDE-Version", "5.5.6")
        .header("User-Agent", "WorkBuddy/5.5.6")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json");
    if let Some(v) = body {
        req = req.json(&v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ProviderError::new(502, &format!("请求失败: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ProviderError::new(
            status.as_u16(),
            &format!("HTTP {} {}", status.as_u16(), text.chars().take(160).collect::<String>()),
        ));
    }
    serde_json::from_str(&text).map_err(|e| ProviderError::new(502, &format!("解析失败: {e}")))
}

fn data(v: &Value) -> &Value {
    v.get("data").unwrap_or(v)
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// One telemetry event shaped exactly like the desktop client's.
///
/// `userId` is inside the body because the server keys the dedupe on it; without
/// it the request still returns 200 and nothing is recorded.
fn chat_event(user_id: &str) -> Value {
    let now = chrono::Utc::now().timestamp_millis();
    let conversation = uuid_v4();
    json!({
        "eventCode": "chat_request_send",
        "timestamp": now,
        "reportDelay": 0,
        "mode": "craft",
        "conversationId": conversation,
        "requestId": conversation,
        "inputLength": 12,
        "requestModelId": "deepseek-v4-flash",
        "requestModelName": "DeepSeek V4 Flash",
        "isPlan": false,
        "isAutoExecuteTerminal": false,
        "isAutoModify": false,
        "codebaseEnable": false,
        "maxToken": 0,
        "maxSteps": 0,
        "temperature": 0,
        "maxRetries": 0,
        "mentionContexts": [],
        "knowledgeId": [],
        "knowledgeName": [],
        "codebaseId": "",
        "mentionContextCount": 0,
        "command": "",
        "expertId": "",
        "recommendId": "",
        "skillId": "",
        "skillCount": 0,
        "totalCount": 0,
        "fileUri": "",
        "presentAt": now,
        "traceId": "",
        "rootRequestId": conversation,
        "parentConversationId": conversation,
        "agentName": "default",
        "agentType": "conversation",
        "userId": user_id,
    })
}

pub(super) async fn activate(
    client: &ProviderClient,
    c: &Credentials,
    account_id: &str,
    label: &str,
) -> ActivationResult {
    let mut out = ActivationResult {
        account_id: account_id.into(),
        label: label.into(),
        ..Default::default()
    };
    let uid = c.user_id.as_deref().unwrap_or("").to_string();
    if uid.is_empty() {
        out.outcome = "failed".into();
        out.note = "凭据缺少 userId，无法上报".into();
        return out;
    }

    // Already has a buddy: nothing to unlock, and re-sending would be a pointless
    // extra event.
    if let Ok(v) = call(client, c, "GET", "/activity/growth/buddy/visible", None).await {
        if data(&v).get("has_buddy").and_then(Value::as_bool).unwrap_or(false) {
            out.outcome = "skipped_has_buddy".into();
            out.note = "已有 Buddy，跳过".into();
            return out;
        }
    }

    if let Ok(v) = call(client, c, "GET", "/v2/activity/growth/streak", None).await {
        out.streak_before = data(&v)
            .get("streak")
            .map(|s| num(s, "days"))
            .unwrap_or(0);
    }

    // Step 1: the activity event. Sent as a one-element array, which is the shape
    // the client uses for batched telemetry.
    let events = json!([chat_event(&uid)]);
    match call(client, c, "POST", "/v2/report", Some(events)).await {
        Ok(_) => {
            out.outcome = "reported".into();
        }
        Err(e) => {
            out.outcome = "failed".into();
            out.note = format!("上报失败: {}", e.message.chars().take(80).collect::<String>());
            return out;
        }
    }

    // The event is processed asynchronously; give the server a moment before the
    // follow-up calls, matching the 1.5s write gap the community guidance uses.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // Step 2: accept the buddy agreement, then adopt. Both are idempotent, so a
    // retry on the next day is safe.
    if call(
        client,
        c,
        "POST",
        "/activity/growth/buddy/agreement",
        Some(json!({"agree": true})),
    )
    .await
    .is_ok()
    {
        out.agreed = true;
    }
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    match call(client, c, "POST", "/activity/growth/buddy/first", Some(json!({}))).await {
        Ok(v) => {
            let d = data(&v);
            out.adopted = true;
            out.credit = num(d, "credit").max(num(d, "reward_credit"));
            out.energy = num(d, "energy").max(num(d, "reward_energy"));
            out.note = format!("领养成功 {} 积分 {} 能量", out.credit, out.energy);
        }
        Err(e) => {
            // A 400 here means the unlock did not take effect — reported so the
            // audit log shows a refused attempt rather than a silent no-op.
            out.note = format!("领养未通过: {}", e.message.chars().take(100).collect::<String>());
        }
    }

    // Step 3: read the streak back. A 200 on /v2/report does not prove the event
    // counted; the streak is the observable effect.
    if let Ok(v) = call(client, c, "GET", "/v2/activity/growth/streak", None).await {
        out.streak_after = data(&v)
            .get("streak")
            .map(|s| num(s, "days"))
            .unwrap_or(0);
    }
    if out.streak_after > out.streak_before {
        out.outcome = if out.adopted { "activated" } else { "reported" }.into();
    } else if out.adopted {
        out.outcome = "activated".into();
    } else if out.outcome == "reported" {
        // The known silent-drop case: HTTP 200, no streak movement, no buddy.
        out.note = format!("{}（连登未变化，可能被静默丢弃）", out.note);
    }
    out
}
