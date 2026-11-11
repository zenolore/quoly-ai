//! WorkBuddy growth-plan automation, scoped strictly to earning credits.
//!
//! Only steps that increase the credit balance run here. Anything that spends a
//! resource without paying credits is excluded, even when the platform offers it:
//!
//! | step            | pays credits      | spends          | runs |
//! |-----------------|-------------------|-----------------|------|
//! | check-in        | 100/day           | —               | yes  |
//! | travel claim    | 5-10 per trip     | —               | yes  |
//! | task claim      | 50-300 per task   | —               | yes  |
//! | streak 14d/28d  | 50 / 150          | —               | yes  |
//! | makeup card     | indirectly        | 1 card          | yes  |
//! | lottery draw    | unknown prize     | 1 free chance   | yes  |
//! | buddy blind box | **none, cosmetic**| 10 energy       | no   |
//!
//! The blind box was removed after checking its template payload (2026-11-09):
//! a box yields a buddy template with name/rarity/artwork and no credit or reward
//! field, while costing 10 energy that has no path back to credits. Opening boxes
//! therefore spends the very thing this module exists to accumulate.
//!
//! Endpoints and payload shapes were confirmed two ways: by observing the real web
//! client's traffic on 2026-11-09, and against the open-source implementation
//! `88lin/workbuddy-auto-signin` (MIT), whose ordering rules were validated in
//! practice. Two details are easy to get wrong and are load-bearing here:
//!
//!   * Every write endpoint except check-in takes a `client_token` of the form
//!     `u-<uuid>`; omitting it is rejected with 400.
//!   * Task acceptance takes a *plural* `task_codes` array while claiming takes the
//!     code in the path with an empty body. The singular form now returns 400.
//!
//! Task progress is credited by the platform against real usage, so this module
//! accepts tasks and claims finished ones — it never fabricates the completion
//! events that the desktop client reports. That boundary is what keeps accounts
//! out of the anti-abuse path.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::providers::{ProviderClient, ProviderError};
use super::types::{Credentials, CreditRegion};

const GROWTH: &str = "/v2/activity/growth";
const BILLING: &str = "/v2/billing/meter";

/// How many credits a single automation pass earned, per category.
///
/// Reported back to the UI so a run that did nothing is distinguishable from a
/// run that failed, and so the user can see where credits actually came from.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthOutcome {
    pub checkin_credits: i64,
    pub checkin_status: String,
    pub travel_claimed: i64,
    pub travel_dispatched: bool,
    pub travel_state: String,
    pub tasks_accepted: usize,
    pub tasks_claimed: usize,
    pub task_credits: i64,
    pub streak_tiers: Vec<String>,
    pub makeup_used: bool,
    pub lottery_draws: usize,
    pub buddy_boxes: usize,
    /// Human-readable lines, in execution order.
    pub steps: Vec<String>,
}

impl GrowthOutcome {
    pub fn total_credits(&self) -> i64 {
        self.checkin_credits + self.travel_claimed + self.task_credits
    }

    fn step(&mut self, text: impl Into<String>) {
        self.steps.push(text.into());
    }
}

/// A task as the platform reports it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTask {
    pub code: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub reward_credit: i64,
    pub reward_energy: i64,
    pub kind: String,
    pub locked: bool,
    pub jump_url: String,
}

/// Everything the UI needs to show a growth panel for one account.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthSnapshot {
    pub level: i64,
    pub completed: i64,
    pub total: i64,
    pub energy: i64,
    pub tasks: Vec<GrowthTask>,
    pub travel_state: String,
    pub travel_remaining_seconds: i64,
    pub daily_limit_reached: bool,
    pub streak_days: i64,
    pub next_tier: String,
    pub next_tier_remaining: i64,
    pub lottery_chances: i64,
    pub buddy_affordable: i64,
}

fn token(prefix: &str) -> String {
    format!("{prefix}-{}", uuid_like())
}

/// A v4-shaped UUID without pulling in a dependency for one call site.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let a = (nanos & 0xffff_ffff) as u32;
    let b = ((nanos >> 32) & 0xffff) as u16 | 0x4000;
    let c = (((nanos >> 48) & 0x3fff) as u16) | 0x8000;
    let d = (nanos >> 20) as u64;
    format!("{a:08x}-{b:04x}-{c:04x}-{:04x}-{:012x}", d & 0xffff, d >> 16)
}

/// Base URL for these endpoints. The web client and the desktop client both use
/// copilot.tencent.com; anything else here would need re-verification.
fn base() -> &'static str {
    "https://copilot.tencent.com"
}

async fn call(
    client: &ProviderClient,
    c: &Credentials,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, ProviderError> {
    let url = format!("{}{}", base(), path);
    let mut req = match method {
        "GET" => client.http().get(url),
        _ => client.http().post(url),
    };
    req = req
        .bearer_auth(&c.access_token)
        .header("X-User-Id", c.user_id.as_deref().unwrap_or(""))
        .header("Accept", "application/json");
    if let Some(domain) = c.extra.get("domain").and_then(Value::as_str) {
        req = req.header("X-Domain", domain);
    }
    if let Some(v) = body {
        req = req.json(&v);
    }
    let resp = req.send().await.map_err(|e| {
        ProviderError::new(502, &format!("成长中心请求失败: {e}"))
    })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        // 403 from travel endpoints means "no buddy adopted yet", which is a
        // normal state, not an error worth surfacing.
        return Err(ProviderError::new(
            status.as_u16(),
            &format!("成长中心 {} {}", status.as_u16(), text.chars().take(160).collect::<String>()),
        ));
    }
    serde_json::from_str(&text).map_err(|e| ProviderError::new(502, &format!("响应解析失败: {e}")))
}

/// The platform wraps every payload as {code, msg, data}.
fn data(v: &Value) -> &Value {
    v.get("data").unwrap_or(v)
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

pub(super) async fn snapshot(
    client: &ProviderClient,
    c: &Credentials,
) -> Result<GrowthSnapshot, ProviderError> {
    let mut snap = GrowthSnapshot::default();

    if let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/tasks"), None).await {
        let d = data(&v);
        if let Some(list) = d.get("tasks").and_then(Value::as_array) {
            snap.tasks = list
                .iter()
                .map(|t| GrowthTask {
                    code: text(t, "task_code"),
                    title: text(t, "title"),
                    description: text(t, "task_desc"),
                    status: text(t, "accept_status"),
                    reward_credit: num(t, "reward_credit"),
                    reward_energy: num(t, "reward_energy"),
                    kind: text(t, "task_type"),
                    locked: t.get("locked").and_then(Value::as_bool).unwrap_or(false),
                    jump_url: text(t, "jump_url"),
                })
                .collect();
            snap.total = snap.tasks.len() as i64;
            snap.completed = snap
                .tasks
                .iter()
                .filter(|t| t.status == "claimed")
                .count() as i64;
        }
    }

    if let Ok(v) = call(client, c, "GET", "/v2/activity/growth/profile", None).await {
        let d = data(&v);
        snap.level = num(d, "level");
        if let Some(c) = d.get("completed").and_then(Value::as_i64) {
            snap.completed = c;
        }
        if let Some(t) = d.get("total").and_then(Value::as_i64) {
            snap.total = t;
        }
    }

    if let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/energy"), None).await {
        snap.energy = num(data(&v), "balance");
    }

    if let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/buddy/travel/status"), None).await {
        let d = data(&v);
        snap.travel_state = text(d, "state");
        snap.daily_limit_reached = d
            .get("daily_limit_reached")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let arrive = num(d, "arrive_at");
        let now = num(d, "server_now");
        if arrive > now && now > 0 {
            snap.travel_remaining_seconds = arrive - now;
        }
    }

    if let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/streak"), None).await {
        let d = data(&v);
        if let Some(s) = d.get("streak") {
            snap.streak_days = num(s, "days");
            snap.next_tier = text(s, "next_tier");
            snap.next_tier_remaining = num(s, "next_tier_remaining");
        }
    }

    if let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/lottery/chances"), None).await {
        snap.lottery_chances = num(data(&v), "balance");
    }

    if let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/buddy/quota"), None).await {
        let d = data(&v);
        let affordable = num(d, "affordable");
        let max_open = num(d, "max_open_count");
        snap.buddy_affordable = if max_open > 0 {
            affordable.min(max_open)
        } else {
            affordable
        };
    }

    Ok(snap)
}

/// One full automation pass: claim everything available, then dispatch a trip.
pub(super) async fn run(
    client: &ProviderClient,
    c: &Credentials,
) -> Result<GrowthOutcome, ProviderError> {
    let mut out = GrowthOutcome::default();

    // Region matters: the international build settles activity credits
    // automatically and exposes no claim endpoints.
    if c.region == CreditRegion::Intl {
        out.step("国际版账号成长中心自动结算，无需领取");
        return Ok(out);
    }

    // 1. Check-in first — it is the only reward guaranteed to exist daily.
    claim_checkin(client, c, &mut out).await;

    // 2. Travel gift: only claimable once the buddy is back.
    claim_travel(client, c, &mut out).await;

    // 3. Task rewards for work already finished.
    claim_task_rewards(client, c, &mut out).await;

    // 4. Accept newly opened tasks so progress starts counting.
    accept_tasks(client, c, &mut out).await;

    // 5. Streak: fill a gap if a makeup card is available, then redeem tiers.
    // This is the second-largest source of credits after tasks — the 28-day tier
    // alone pays 150 — so it runs before the decorative steps, not after.
    makeup(client, c, &mut out).await;
    redeem_streak(client, c, &mut out).await;

    // 6. Draw lottery tickets while any remain. Lottery entries do pay credits.
    //
    // Buddy blind boxes are deliberately NOT opened: the box yields a cosmetic
    // buddy template (name / rarity / artwork, verified 2026-11-09 — the template
    // payload carries no credit or reward field) at 10 energy each, and energy has
    // no conversion path back to credits. Spending it therefore reduces the only
    // thing this module exists to collect.
    draw_lottery(client, c, &mut out).await;

    // 7. Dispatch last: opening boxes used to overwrite the travel record, and
    // keeping dispatch at the end keeps that ordering stable if boxes return.
    depart_travel(client, c, &mut out).await;

    Ok(out)
}

async fn claim_checkin(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let status = call(
        client,
        c,
        "POST",
        &format!("{BILLING}/checkin-activity-status"),
        Some(json!({})),
    )
    .await;

    let already = status
        .as_ref()
        .ok()
        .map(|v| {
            let d = data(v);
            // The service returns snake_case here (today_checked_in), unlike the
            // camelCase used elsewhere. Both spellings are checked so a format
            // change on either side does not silently cause a duplicate claim.
            d.get("today_checked_in")
                .or_else(|| d.get("todayCheckedIn"))
                .or_else(|| d.get("checked_in"))
                .or_else(|| d.get("checkedIn"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if already {
        out.checkin_status = "已领取".into();
        out.step("签到：今日已领");
        return;
    }

    match call(client, c, "POST", &format!("{BILLING}/daily-checkin"), Some(json!({}))).await {
        Ok(v) => {
            let d = data(&v);
            let got = num(d, "credit")
                .max(num(d, "reward_credit"))
                .max(num(d, "amount"));
            out.checkin_credits = got;
            out.checkin_status = "已领取".into();
            out.step(format!("签到：领取 {got} 积分"));
        }
        Err(e) => {
            out.checkin_status = format!("失败 {}", e.status);
            out.step(format!("签到：失败（{}）", e.message.chars().take(60).collect::<String>()));
        }
    }
}

async fn claim_travel(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/buddy/travel/status"), None).await else {
        return;
    };
    let d = data(&v);
    out.travel_state = text(d, "state");
    let record_id = num(d, "record_id");
    if out.travel_state != "arrived" || record_id == 0 {
        return;
    }
    match call(
        client,
        c,
        "POST",
        &format!("{GROWTH}/buddy/travel/claim"),
        Some(json!({"record_id": record_id})),
    )
    .await
    {
        Ok(r) => {
            let got = num(data(&r), "reward_credit");
            out.travel_claimed = got;
            out.step(format!("旅行：领回礼物 {got} 积分"));
        }
        Err(e) => out.step(format!("旅行：领礼失败（{}）", e.status)),
    }
}

async fn claim_task_rewards(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/tasks"), None).await else {
        return;
    };
    let Some(list) = data(&v).get("tasks").and_then(Value::as_array) else {
        return;
    };
    let done: Vec<(String, i64)> = list
        .iter()
        .filter(|t| text(t, "accept_status") == "completed")
        .map(|t| (text(t, "task_code"), num(t, "reward_credit")))
        .collect();
    for (code, credit) in done {
        if code.is_empty() {
            continue;
        }
        let path = format!("{GROWTH}/tasks/{code}/claim");
        match call(client, c, "POST", &path, Some(json!({}))).await {
            Ok(_) => {
                out.tasks_claimed += 1;
                out.task_credits += credit;
                out.step(format!("任务：领取「{code}」{credit} 积分"));
            }
            Err(e) if e.status == 403 => {}
            Err(_) => {}
        }
    }
}

async fn accept_tasks(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/tasks"), None).await else {
        return;
    };
    let Some(list) = data(&v).get("tasks").and_then(Value::as_array) else {
        return;
    };
    // Only unlocked tasks that have not been accepted yet; accepting an already
    // accepted task is harmless but noisy in the log.
    let codes: Vec<String> = list
        .iter()
        .filter(|t| {
            text(t, "accept_status") == "not_accepted"
                && !t.get("locked").and_then(Value::as_bool).unwrap_or(false)
        })
        .map(|t| text(t, "task_code"))
        .filter(|c| !c.is_empty())
        .collect();
    if codes.is_empty() {
        return;
    }
    // Batched at 20: the endpoint rejects larger arrays.
    for chunk in codes.chunks(20) {
        let body = json!({ "task_codes": chunk });
        if call(client, c, "POST", &format!("{GROWTH}/tasks/accept"), Some(body))
            .await
            .is_ok()
        {
            out.tasks_accepted += chunk.len();
        }
    }
    if out.tasks_accepted > 0 {
        out.step(format!("任务：接取 {} 个新任务", out.tasks_accepted));
    }
}

async fn makeup(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/streak"), None).await else {
        return;
    };
    let d = data(&v);
    let cards = d
        .get("makeup_cards")
        .map(|m| num(m, "balance"))
        .unwrap_or(0);
    if cards <= 0 {
        return;
    }
    let Ok(h) = call(client, c, "GET", &format!("{GROWTH}/heatmap"), None).await else {
        return;
    };
    // The calendar comes back as `cells` with a numeric `score`, not `days` with
    // `count` — verified against the live service on 2026-11-09.
    let Some(cells) = data(&h).get("cells").and_then(Value::as_array) else {
        return;
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let month = today[..7].to_string();
    let used: Vec<String> = d
        .get("streak")
        .and_then(|s| s.get("makeup_dates"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default();
    // Only this month, before today, and a day with zero activity.
    let mut target = None;
    for cell in cells.iter().rev() {
        let date = text(cell, "date");
        if date.len() < 10 {
            continue;
        }
        let day = &date[..10];
        if &day[..7] != month.as_str() || day >= &today[..10] {
            continue;
        }
        if num(cell, "score") != 0 {
            continue;
        }
        if used.iter().any(|u| u.starts_with(day)) {
            continue;
        }
        target = Some(day.to_string());
        break;
    }
    let Some(target) = target else { return };
    if call(
        client,
        c,
        "POST",
        &format!("{GROWTH}/makeup-cards/use"),
        Some(json!({"target_date": target})),
    )
    .await
    .is_ok()
    {
        out.makeup_used = true;
        out.step(format!("连登：用补登卡补了 {target}"));
    }
}

async fn redeem_streak(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/streak"), None).await else {
        return;
    };
    let d = data(&v);
    let Some(rs) = d.get("redemption_status") else { return };
    let Some(tiers) = rs.get("tiers").and_then(Value::as_array) else {
        return;
    };
    // The status is NOT inside each tier object — it sits beside it as
    // `tier_7d_status`, `tier_14d_status`, ... Reading `tier.status` (which looks
    // natural) yields nothing, so redemption would silently never fire.
    for tier in tiers {
        let label = text(tier, "tier");
        if label.is_empty() {
            continue;
        }
        let key = format!("tier_{}_status", label.replace('-', "_"));
        let status = rs.get(&key).and_then(Value::as_str).unwrap_or("");
        if status != "claimable" && status != "available" && status != "unlocked" {
            continue;
        }
        let body = json!({"tier": label, "client_token": token("u")});
        if call(client, c, "POST", &format!("{GROWTH}/redeem"), Some(body))
            .await
            .is_ok()
        {
            out.streak_tiers.push(label.clone());
            out.step(format!("连登：兑换 {label} 档"));
        }
    }
}

async fn draw_lottery(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/lottery/chances"), None).await else {
        return;
    };
    let chances = num(data(&v), "balance");
    if chances <= 0 {
        return;
    }
    // One per pass. Chances come free with streak tiers, so drawing cannot reduce
    // credits — even though the prize table is not documented, a free draw with no
    // downside belongs in a credits-first pass.
    let body = json!({"client_token": token("u")});
    if call(client, c, "POST", &format!("{GROWTH}/lottery/draw"), Some(body))
        .await
        .is_ok()
    {
        out.lottery_draws = 1;
        out.step("抽奖：使用 1 次机会");
    }
}

/// Disabled on purpose: boxes cost energy and pay only cosmetics.
///
/// Kept (not deleted) because the quota fields are still read for the UI, and
/// because a future event may make energy convertible — at which point opening
/// boxes should be re-evaluated rather than rewritten from scratch.
#[allow(dead_code)]
async fn open_buddy_boxes(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/buddy/quota"), None).await else {
        return;
    };
    let d = data(&v);
    let affordable = num(d, "affordable");
    let max_open = num(d, "max_open_count");
    let count = if max_open > 0 {
        affordable.min(max_open)
    } else {
        affordable
    };
    if count <= 0 {
        return;
    }
    let body = json!({"count": count, "client_token": token("u")});
    if call(client, c, "POST", &format!("{GROWTH}/buddy/open"), Some(body))
        .await
        .is_ok()
    {
        out.buddy_boxes = count as usize;
        out.step(format!("盲盒：开启 {count} 个 Buddy"));
    }
}

async fn depart_travel(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/buddy/travel/status"), None).await else {
        return;
    };
    let d = data(&v);
    if text(d, "state") != "idle"
        || d.get("daily_limit_reached").and_then(Value::as_bool).unwrap_or(false)
    {
        return;
    }
    let Ok(cfg) = call(client, c, "GET", &format!("{GROWTH}/buddy/travel/config"), None).await else {
        return;
    };
    let Some(loc) = data(&cfg)
        .get("locations")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    else {
        return;
    };
    let location_id = num(loc, "id");
    if location_id == 0 {
        return;
    }
    match call(
        client,
        c,
        "POST",
        &format!("{GROWTH}/buddy/travel/depart"),
        Some(json!({"location_id": location_id})),
    )
    .await
    {
        Ok(_) => {
            out.travel_dispatched = true;
            out.step(format!("旅行：已派出（{})", text(loc, "name")));
        }
        Err(e) if e.status == 403 => {}
        Err(_) => {}
    }
}
