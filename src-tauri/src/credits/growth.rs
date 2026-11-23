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
use std::collections::{HashMap, HashSet};

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
    /// 连登档位兑换拿到的积分。之前只记档位名、不记分，导致"今日获得"
    /// 少算（14d 档一次 50 分），而账上其实到账了。
    pub streak_credits: i64,
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
        self.checkin_credits + self.travel_claimed + self.task_credits + self.streak_credits
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

/// 平台把业务失败也包在 HTTP 200 里，`code` 才是真正结果。
///
/// 实测（2026-11-14，对已领取任务重复领取）：
/// `{"code":0,"msg":"OK","data":{"already_claimed":true,"credit":0}}`
/// 而任务没做完时同样可能是 200 + 非 0 的 code。只看 HTTP 状态会把失败记成
/// 成功 —— 这正是 accept 接口踩过的坑，claim 接口原本也有同一个毛病。
fn envelope_error(v: &Value) -> Option<String> {
    match v.get("code") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) if n.as_i64() == Some(0) => None,
        Some(Value::String(s)) if s.is_empty() || s == "0" => None,
        Some(other) => {
            let msg = text(v, "msg");
            Some(if msg.is_empty() {
                format!("code={other}")
            } else {
                format!("{msg}（code={other}）")
            })
        }
    }
}

/// 平台对「本来就不需要接取的任务」的回答 —— 不是失败，不该刷屏。
///
/// 实测 first_buddy 永远返回 `task does not require acceptance`，
/// 而 Library_read / skill_share / 关注公众号 都以它为前置。
fn is_benign_accept_answer(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains("does not require acceptance")
        || lower.contains("already accepted")
        || reason.contains("无需接取")
}

/// 一次领取请求到底发生了什么。
#[derive(Debug, PartialEq, Eq)]
enum ClaimResult {
    /// 平台确认到账，附带真实发放的积分（以平台返回值准，不信任务标称值）。
    Granted(i64),
    /// 之前已经领过，本次没有新增积分 —— 不算失败，也不能重复计数。
    AlreadyClaimed,
    /// 平台明确拒绝，带原因。
    Failed(String),
}

fn parse_claim_response(v: &Value) -> ClaimResult {
    if let Some(reason) = envelope_error(v) {
        return ClaimResult::Failed(reason);
    }
    let d = data(v);
    if d.get("already_claimed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return ClaimResult::AlreadyClaimed;
    }
    let granted = num(d, "credit");
    if granted > 0 {
        return ClaimResult::Granted(granted);
    }
    // code=0 但没给积分：不能记成"领取成功"，否则日志上多一笔、账上没有钱。
    ClaimResult::Failed("平台未返回积分（credit=0）".to_string())
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

    // 3. Accept newly opened tasks BEFORE claiming.
    //
    // 顺序不能反。原先是"先领后接"，等于先检查有没有能领的、再去接新任务，
    // 刚接上的任务这一轮永远轮不到领取。而且不是每个任务都能随手接 ——
    // 有前置依赖（实测：想接 playbook_prompt 提示 prerequisite not met:
    // first_buddy），所以接取本身还要挑顺序。先接完再统一领，一轮内就能
    // 把"接取即完成"的那批领掉。
    accept_tasks(client, c, &mut out).await;

    // 4. Task rewards for work already finished.
    claim_task_rewards(client, c, &mut out).await;

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
    // arrived 才是可领取的状态；别的状态（idle/traveling）没什么可领的，
    // 安静跳过是合理的 —— 这不是错误。
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
        // 这里原本也是静默失败。领取失败要么是重复领、要么是记录过期，
        // 都值得留一行痕迹，否则"旅行回来却没积分"永远查不出原因。
        Err(e) => out.step(format!(
            "旅行：领礼失败（{} {}）",
            e.status,
            e.message.chars().take(80).collect::<String>()
        )),
    }
}

async fn claim_task_rewards(client: &ProviderClient, c: &Credentials, out: &mut GrowthOutcome) {
    let Ok(v) = call(client, c, "GET", &format!("{GROWTH}/tasks"), None).await else {
        return;
    };
    let Some(list) = data(&v).get("tasks").and_then(Value::as_array) else {
        return;
    };
    // 判断"可领取"的依据是进度，不是状态码。
    //
    // 2026-11-13 修：这里原本写 `accept_status == "completed"`，但接口根本不返回
    // completed 这个值 —— 实测 19 个任务只出现 accepted 和 claimed 两种。
    // 于是过滤器永远为空，任务一个都领不到，而且不报错、不告警：
    // 3309 次运行记录里 tasksClaimed 全是 0，功能从上线起就没生效过。
    //
    // 真实判据在 progress 里：
    //   accepted + progress.current >= target  → 已完成，可领
    //   accepted + 进度没满                     → 还在做，领了会失败
    //   claimed                                 → 已领过，跳过
    //
    // 顺带把 claimed 也排除，避免对着已领的任务重复 POST（403 虽然被吞掉，
    // 但每个号每次白白多打十几个请求，平台风控会记账）。
    let done: Vec<(String, i64)> = list
        .iter()
        .filter(|t| task_claimable(t))
        .filter_map(|t| {
            let code = text(t, "task_code");
            if code.is_empty() {
                return None;
            }
            Some((code, num(t, "reward_credit")))
        })
        .collect();
    for (code, advertised) in done {
        let path = format!("{GROWTH}/tasks/{code}/claim");
        let resp = match call(client, c, "POST", &path, Some(json!({}))).await {
            Ok(v) => v,
            Err(e) => {
                out.step(format!("任务：领取「{code}」失败（HTTP {}）", e.status));
                continue;
            }
        };
        // 2026-11-14 修：原代码 `Ok(_) => claimed += 1; credits += 任务标称奖励`
        // 只要 HTTP 200 就记成功。但领取接口和 accept 一样，失败也回 200，
        // 真实结果在 body 的 code / data.already_claimed / data.credit 里。
        // 结果就是：一个任务都没领到，日志却写着"领取 100 积分"。
        // 现在只认平台给的 credit，标称值只用来提示。
        match parse_claim_response(&resp) {
            ClaimResult::Granted(credit) => {
                out.tasks_claimed += 1;
                out.task_credits += credit;
                out.step(format!("任务：领取「{code}」{credit} 积分"));
            }
            // 已经领过：静默跳过，避免刷屏（一个号每次会有好几条）。
            ClaimResult::AlreadyClaimed => {}
            ClaimResult::Failed(reason) => {
                out.step(format!(
                    "任务：领取「{code}」失败（{reason}，标称 {advertised} 积分）"
                ));
            }
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
    // 先接前置任务，再接依赖它们的任务。
    //
    // 实测：想接 playbook_prompt / Library_read / Buddy_App 都会返回
    // "prerequisite not met: first_buddy"。这类依赖关系平台不会提前告知，
    // 只知道失败了、原因的文本里带着被依赖的任务码。
    //
    // 处理办法：把 first_buddy 这类"根任务"排到最前面先接，剩下的用
    // 失败重试兜底 —— 第一轮有任务接不上，等根任务接完再试一次。
    // 两轮足够覆盖现有的单层依赖，不做通用拓扑排序：任务表会变，
    // 猜依赖图不如直接重试来得稳。
    let mut ordered = codes.clone();
    ordered.sort_by_key(|code| if code == "first_buddy" { 0 } else { 1 });

    // 每个 task_code 平台的逐条判定： (是否号称接上, 原因)
    let mut said: HashMap<String, (bool, String)> = HashMap::new();
    // 平台明说"不需要接取"的（如 first_buddy）：既不成功也不失败，不再报。
    let mut benign: HashSet<String> = HashSet::new();
    for pass in 0..2 {
        if ordered.is_empty() {
            break;
        }
        // Batched at 20: the endpoint rejects larger arrays.
        let mut retry: Vec<String> = Vec::new();
        for chunk in ordered.chunks(20) {
            let body = json!({ "task_codes": chunk });
            let resp = match call(client, c, "POST", &format!("{GROWTH}/tasks/accept"), Some(body)).await
            {
                Ok(v) => v,
                Err(e) => {
                    for code in chunk {
                        said.insert(code.clone(), (false, format!("HTTP {}", e.status)));
                    }
                    continue;
                }
            };
            // 逐个看 results，不能只看 HTTP 状态。
            //
            // 2026-11-13 修：这个接口有个坑 —— 哪怕全部任务都接取失败，它也返回
            // HTTP 200 且 {code: 0, msg: "OK"}，真实结果藏在 data.results 里：
            //   {"task_code":"playbook_prompt","status":"error",
            //    "message":"prerequisite not met: first_buddy"}
            // 原逻辑只看 is_ok()，于是把"全失败"记成"接取 N 个新任务"，
            // 日志看起来一切正常，实际一个都没接上 —— 这是任务积分长期为 0 的
            // 第三个原因，也是最难查的一个。
            //
            // 2026-11-14 再修两处：
            //   * results 缺失时不再假定"整批成功"。实测就是这条推断造出了
            //     "接取 18 个新任务"的假日志 —— 那批请求平台没给 results，
            //     复查发现只接上了 1 个。现在改成全部复查后再记账。
            //   * first_buddy 返回 "task does not require acceptance"：它本来
            //     就不用接，不该记成失败刷屏。
            if let Some(reason) = envelope_error(&resp) {
                for code in chunk {
                    said.insert(code.clone(), (false, reason.clone()));
                }
                continue;
            }
            let results = data(&resp)
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if results.is_empty() {
                // 平台没给逐条结论：不猜，留空。下面的复查会给出真相。
                continue;
            }
            for item in results {
                let code = text(&item, "task_code");
                let status = text(&item, "status");
                let reason = text(&item, "message");
                if matches!(status.as_str(), "ok" | "success" | "accepted" | "already_accepted") {
                    said.insert(code, (true, String::new()));
                } else if is_benign_accept_answer(&reason) {
                    // 不需要接取的任务：既不算成功也不算失败，静默跳过。
                    benign.insert(code);
                } else if pass == 0 && reason.contains("prerequisite") {
                    // 前置依赖类的失败值得再试一轮（依赖的任务刚被接上）
                    retry.push(code);
                } else {
                    said.insert(code, (false, reason));
                }
            }
        }
        ordered = retry;
    }

    // 复查：以平台重新返回的状态为准。
    //
    // 请求返回的 status 只代表"平台收到了"，accept_status 才代表"平台记上了"。
    // 两者不一致的情况真实存在，所以最终记账只看复查结果。
    let confirmed = match call(client, c, "GET", &format!("{GROWTH}/tasks"), None).await {
        Ok(v) => data(&v)
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let mut accepted = 0usize;
    let mut blocked: Vec<(String, String)> = Vec::new();
    for code in &codes {
        if benign.contains(code) {
            continue;
        }
        let now = confirmed
            .iter()
            .find(|t| text(t, "task_code") == *code)
            .map(|t| text(t, "accept_status"))
            .unwrap_or_default();
        match now.as_str() {
            "accepted" | "claimed" => accepted += 1,
            _ => {
                let reason = said
                    .get(code)
                    .map(|(_, r)| r.clone())
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| {
                        if confirmed.is_empty() {
                            "复查失败，无法确认".to_string()
                        } else {
                            "平台未记录该接取".to_string()
                        }
                    });
                if !is_benign_accept_answer(&reason) {
                    blocked.push((code.clone(), reason));
                }
            }
        }
    }

    out.tasks_accepted += accepted;
    if accepted > 0 {
        out.step(format!("任务：接取 {accepted} 个新任务"));
    }
    // 只报真正被拒的，且同一原因合并成一条，避免 50 个号刷出几百行。
    let mut by_reason: Vec<(String, usize)> = Vec::new();
    for (_, reason) in &blocked {
        match by_reason.iter_mut().find(|(r, _)| r == reason) {
            Some((_, n)) => *n += 1,
            None => by_reason.push((reason.clone(), 1)),
        }
    }
    for (reason, n) in by_reason {
        out.step(format!("任务：{n} 个接不上（{reason}）"));
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
        match call(client, c, "POST", &format!("{GROWTH}/redeem"), Some(body)).await {
            Ok(resp) => {
                // 和 claim 一样，2xx 不代表真到账：信封里的 code 才是结论。
                if let Some(reason) = envelope_error(&resp) {
                    out.step(format!("连登：兑换 {label} 档失败（{reason}）"));
                    continue;
                }
                let got = num(data(&resp), "credit")
                    .max(num(data(&resp), "reward_credit"))
                    .max(num(data(&resp), "amount"));
                out.streak_credits += got;
                out.streak_tiers.push(label.clone());
                if got > 0 {
                    out.step(format!("连登：兑换 {label} 档，+{got} 积分"));
                } else {
                    // 平台没回积分字段时只说兑换成功，不编数字。
                    out.step(format!("连登：兑换 {label} 档"));
                }
            }
            Err(e) => out.step(format!("连登：兑换 {label} 档失败（HTTP {}）", e.status)),
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
    if text(d, "state") != "idle" {
        // 还在路上或已到达，本轮无事可做（到达后由 claim_travel 领礼物）
        return;
    }
    if d.get("daily_limit_reached").and_then(Value::as_bool).unwrap_or(false) {
        return;
    }

    // 派发旅行要求账号先有 buddy。
    //
    // 2026-11-13 实测：没有 buddy 的账号调 depart 会返回
    //   403 {"code":403,"msg":"no active buddy"}
    // 原代码对 403 一律静默吞掉，所以既没派发、日志里也没痕迹 ——
    // 用户只能看到"旅行状态一直是 idle"，查不出原因。
    //
    // 先查 buddy 列表，没有就直接给出可读的说明，跳过这个账号。
    // 这一步很关键：首次使用的新账号全是"0 个 buddy + 0 能量"，
    // 不做这个检查，会在每个新账号上白撞一次 403。
    let has_buddy = call(client, c, "GET", &format!("{GROWTH}/buddy/list"), None)
        .await
        .ok()
        .map(|b| {
            data(&b)
                .get("buddies")
                .and_then(Value::as_array)
                .is_some_and(|list| !list.is_empty())
        })
        .unwrap_or(false);
    if !has_buddy {
        out.step("旅行：跳过（账号还没有 Buddy，先在客户端领养一只）".to_string());
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
            out.step(format!("旅行：已派出（{}）", text(loc, "name")));
        }
        // 不再静默吞掉：403 往往是"没 buddy"或"今日次数用完"，
        // 这两种都该让用户看见，否则下次还是同样的困惑。
        Err(e) => out.step(format!(
            "旅行：派出失败（{} {}）",
            e.status,
            e.message.chars().take(80).collect::<String>()
        )),
    }
}

/// 一个任务当前能不能领奖。
///
/// 抽成独立函数是为了能被单测钉住 —— 这段判断错过两次，两次都是
/// "看起来合理但和接口实际不符"，不测就还会再错。
///
/// 判据（2026-11-13 用真实账号逐条核对）：
///   accept_status == "claimed"           → 已领过，跳过
///   没有 progress 字段                    → 从未开始，跳过
///   progress.current >= progress.target  → 已完成，可领
///   其余                                  → 没做完，领了接口会返回
///                                         400 task not completed
///
/// 两个陷阱，都实测过：
///
/// 1. `claimed_button_text`（"已完成"）不可信。有任务是
///    accepted + 0/1 + button_text="已完成"，真去 claim 却返回
///    task not completed。只能看 progress。
///
/// 2. `progress == null` 不等于"无进度概念直接算完成"。实测三个账号：
///    已正常领取 16 个任务的账号，没有一个 progress=null 的任务；
///    而一个从未在客户端操作过的账号，17 个任务全是 progress=null
///    且 status=not_accepted。所以 null 表示"这个任务还没被激活"，
///    当它可领会对每个任务白打一次 400。
fn task_claimable(task: &Value) -> bool {
    if text(task, "accept_status") == "claimed" {
        return false;
    }
    let Some(progress) = task.get("progress").filter(|p| !p.is_null()) else {
        return false;
    };
    let current = num(progress, "current");
    // target 为 0 或缺失时按 1 处理：这类任务没有进度刻度，
    // 有 current 就说明已经被记过一次，视为完成。
    let target = num(progress, "target").max(1);
    current >= target
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: &str, current: Option<i64>, target: Option<i64>, button: &str) -> Value {
        let progress = match (current, target) {
            (Some(c), Some(t)) => json!({ "current": c, "target": t }),
            _ => Value::Null,
        };
        json!({
            "task_code": "x",
            "accept_status": status,
            "progress": progress,
            "claimed_button_text": button,
            "reward_credit": 100,
        })
    }

    /// 已领取的任务不能再领 —— 否则每个号每次白打十几个请求。
    #[test]
    fn claimed_tasks_are_skipped() {
        assert!(!task_claimable(&task("claimed", Some(1), Some(1), "已完成")));
        // 极端情况：claimed 但进度看着没满，也不能领
        assert!(!task_claimable(&task("claimed", Some(0), Some(5), "已完成")));
    }

    /// 进度满了才能领。这是修复的核心。
    #[test]
    fn only_full_progress_is_claimable() {
        assert!(task_claimable(&task("accepted", Some(1), Some(1), "已完成")));
        assert!(task_claimable(&task("accepted", Some(5), Some(5), "已完成")));
        assert!(task_claimable(&task("accepted", Some(3), Some(3), "-")));
        // 没满，领了接口会 400
        assert!(!task_claimable(&task("accepted", Some(0), Some(1), "已完成")));
        assert!(!task_claimable(&task("accepted", Some(2), Some(5), "已完成")));
    }

    /// not_accepted（还没接）且没进度的，不该拿去领。
    #[test]
    fn unaccepted_tasks_without_progress_are_not_claimable() {
        assert!(!task_claimable(&task("not_accepted", None, None, "已完成")));
    }

    /// progress 为 null 表示"任务从未被激活"，不是"已完成"。
    ///
    /// 这是第二次修正的结论，来自真实数据：已正常领完 16 个任务的账号里
    /// 没有一个 progress=null 的任务；而一个从未在客户端操作过的账号，
    /// 17 个任务全是 progress=null + not_accepted。
    /// 若把 null 当可领，会对每个任务白打一次 400 task not completed。
    #[test]
    fn null_progress_means_never_started() {
        assert!(!task_claimable(&task("accepted", None, None, "-")));
        assert!(!task_claimable(&task("not_accepted", None, None, "已完成")));
    }

    /// target 缺失或为 0 时，有 current 就视为完成。
    ///
    /// 这类任务没有进度刻度（"点一下就算"），平台只记 1 次。
    #[test]
    fn zero_or_missing_target_uses_current_presence() {
        let t = json!({
            "accept_status": "accepted",
            "progress": { "current": 1 },
            "reward_credit": 100,
        });
        assert!(task_claimable(&t));

        let t = json!({
            "accept_status": "accepted",
            "progress": { "current": 1, "target": 0 },
            "reward_credit": 100,
        });
        assert!(task_claimable(&t));
    }

    /// `claimed_button_text` 是陷阱，不能当判据。
    ///
    /// 实测存在 button="已完成" 但 progress=0/1 的任务，
    /// 调 claim 返回 400 task not completed。这条测试把它钉死：
    /// 光看 button 会误判，必须看 progress。
    #[test]
    fn button_text_is_not_a_claim_signal() {
        let misleading = task("accepted", Some(0), Some(1), "已完成");
        assert!(
            !task_claimable(&misleading),
            "button_text=已完成 但进度 0/1，不该判为可领"
        );
    }

    /// 领取结果必须按平台返回的 credit 记账，不能按任务标称奖励。
    ///
    /// 实测响应（2026-11-14，重复领取已领任务）：
    ///   {"code":0,"msg":"OK","data":{"already_claimed":true,"credit":0,"energy":0}}
    /// 原代码只要 HTTP 200 就 `claimed += 1; credits += 任务标称值`，
    /// 于是"一个都没领到"被记成"领取 100 积分"。
    #[test]
    fn already_claimed_is_not_counted_as_new_credit() {
        let resp = json!({
            "code": 0,
            "msg": "OK",
            "data": { "already_claimed": true, "credit": 0, "energy": 0 }
        });
        assert_eq!(parse_claim_response(&resp), ClaimResult::AlreadyClaimed);
    }

    #[test]
    fn granted_credit_comes_from_the_response_not_the_task() {
        let resp = json!({ "code": 0, "msg": "OK", "data": { "credit": 100, "energy": 8 } });
        assert_eq!(parse_claim_response(&resp), ClaimResult::Granted(100));

        // 平台给的比标称少（活动打折），也要按平台给的记。
        let resp = json!({ "code": 0, "msg": "OK", "data": { "credit": 30 } });
        assert_eq!(parse_claim_response(&resp), ClaimResult::Granted(30));
    }

    /// HTTP 200 + 非 0 code 必须当失败，不能静默算成功。
    #[test]
    fn business_error_inside_http_200_is_a_failure() {
        let resp = json!({ "code": 400, "msg": "task not completed", "data": {} });
        match parse_claim_response(&resp) {
            ClaimResult::Failed(reason) => assert!(reason.contains("task not completed")),
            other => panic!("应判为失败，实际 {other:?}"),
        }
    }

    /// code=0 但补发 0 积分（既不是已领、也没报错）同样不能记成功。
    #[test]
    fn zero_credit_without_already_claimed_is_a_failure() {
        let resp = json!({ "code": 0, "msg": "OK", "data": { "credit": 0 } });
        assert!(matches!(parse_claim_response(&resp), ClaimResult::Failed(_)));
    }

    /// 没有 code 字段的旧响应按成功处理，不能因为缺字段就判失败。
    #[test]
    fn missing_envelope_code_is_treated_as_success() {
        assert!(envelope_error(&json!({ "data": { "credit": 50 } })).is_none());
        assert!(envelope_error(&json!({ "code": "0", "data": {} })).is_none());
        assert!(envelope_error(&json!({ "code": 1001, "msg": "限流" })).is_some());
    }

    /// first_buddy 会说「不需要接取」，这是正常回答，不能当失败刷屏。
    ///
    /// 实测（2026-11-14，新号 07b3480e）：
    ///   {"task_code":"first_buddy","status":"error",
    ///    "message":"task does not require acceptance"}
    /// 而 Library_read / 关注公众号 / skill_share 全部以它为前置，
    /// 于是每轮都会刷 3 条 "prerequisite not met: first_buddy"。
    #[test]
    fn acceptance_that_is_not_required_is_benign() {
        assert!(is_benign_accept_answer("task does not require acceptance"));
        assert!(!is_benign_accept_answer("prerequisite not met: first_buddy"));
        assert!(!is_benign_accept_answer(""));
    }
}
