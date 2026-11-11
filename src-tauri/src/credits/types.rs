use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Workbuddy,
    Traework,
    Qoder,
    #[serde(rename = "minimax-code")]
    MinimaxCode,
}
impl Platform {
    pub fn key(self) -> &'static str {
        match self {
            Self::Workbuddy => "workbuddy",
            Self::Traework => "traework",
            Self::Qoder => "qoder",
            Self::MinimaxCode => "minimax-code",
        }
    }

    pub fn route_key(self, region: CreditRegion) -> String {
        match region {
            CreditRegion::Cn => self.key().into(),
            CreditRegion::Intl => format!("{}-intl", self.key()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum CreditRegion {
    #[default]
    Cn,
    Intl,
}
impl CreditRegion {
    pub fn key(self) -> &'static str {
        match self { Self::Cn => "cn", Self::Intl => "intl" }
    }
}

// Secrets are persisted only in the operating system credential store.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default)]
    pub region: CreditRegion,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditPack {
    #[serde(default = "credits_unit")]
    pub unit: String,
    pub id: String,
    pub name: String,
    pub remaining: Option<f64>,
    pub expires_at: Option<i64>,
}
fn credits_unit() -> String { "credits".into() }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    /// The platform's own model key when `id` is a display-derived slug
    /// (e.g. Qoder key `gfmodel` behind the public id `glm-5.3-flash`).
    #[serde(default)]
    pub upstream_id: Option<String>,
    /// Underlying model IDs explicitly advertised by the catalog, separate from routing keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_model_ids: Vec<String>,
    pub name: String,
    pub canonical_id: Option<String>,
    pub supports_tools: bool,
    pub context_window: Option<u64>,
    /// Context tiers advertised by the channel; context_window is the active selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_window_options: Vec<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub supports_images: Option<bool>,
    #[serde(default)]
    pub supports_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_effort_options: Vec<String>,
    #[serde(default)]
    pub default_reasoning_effort: Option<String>,
    pub verified: bool,
    pub verification_note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reward {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub status: String,
    pub amount: Option<f64>,
    pub next_available_at: Option<i64>,
    /// Optional activity details link. Credit claims use the provider protocol
    /// and the account's live campaign ID, never this URL.
    #[serde(default)]
    pub action_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub balance: Option<f64>,
    pub unit: String,
    pub quota: Option<String>,
    pub packs: Vec<CreditPack>,
    pub rewards: Vec<Reward>,
    pub models: Vec<ModelInfo>,
    pub fetched_at: i64,
    pub stale: bool,
    pub diagnostic: Option<String>,
    #[serde(default)]
    pub sync: SnapshotSync,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSync {
    pub rewards_fetched_at: Option<i64>,
    pub models_fetched_at: Option<i64>,
    pub rewards_error: Option<String>,
    pub models_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub platform: Platform,
    #[serde(default)]
    pub region: CreditRegion,
    pub user_id: String,
    pub label: String,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub snapshot: Option<ProviderSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOutcome {
    pub account_id: String,
    pub reward_id: String,
    pub name: String,
    pub status: String,
    pub amount: Option<f64>,
    pub kind: String,
    pub message: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettings {
    pub port: u16,
    pub auto_claim: bool,
    #[serde(default = "auto_refresh_default")]
    pub auto_refresh: bool,
    /// Run the WorkBuddy growth-center pass (travel, streak, boxes, task claims)
    /// on the same daily schedule as auto_claim. Off by default: it makes more
    /// calls per account than check-in, so it should be an explicit choice.
    #[serde(default)]
    pub auto_growth: bool,
    /// Bind the shared gateway on 0.0.0.0 so LAN devices can call it
    /// (http://<lan-ip>:port/v1). Defaults to true to preserve the
    /// behavior LAN callers depend on.
    #[serde(default = "lan_share_default")]
    pub lan_share: bool,
}
fn auto_refresh_default() -> bool { true }
fn lan_share_default() -> bool { true }
impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            port: 17860,
            auto_claim: false,
            auto_refresh: true,
            auto_growth: false,
            lan_share: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
    pub port: u16,
    pub base_url: String,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDiagnostic {
    pub http_status: u16,
    pub upstream_code: Option<String>,
    pub upstream_request_id: Option<String>,
    pub stage: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub id: String,
    pub account_id: String,
    pub platform: Platform,
    #[serde(default)]
    pub region: CreditRegion,
    pub model: String,
    pub elapsed_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// Prompt tokens answered from the upstream cache. Absent for records
    /// written before this field existed, and for upstreams that do not report
    /// it — treat `None` as "unknown", not as zero cache hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    /// Credits the upstream actually charged. Authoritative when present;
    /// absent for platforms that do not report it (TRAE, Qoder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<f64>,
    pub status: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<RequestDiagnostic>,
}

/// Token usage for one bucket (a platform, a model, an account, …).
///
/// `prompt_tokens` is what was *sent*; `billable_prompt_tokens` is what the
/// upstream actually charges for. The gap between them is the prompt cache,
/// which in real traffic swallows the overwhelming majority of input. Reporting
/// only the sent figure makes usage look orders of magnitude worse than the
/// bill, so both are carried and `cache_hit_rate` makes the difference explicit.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    /// Requests attempted in this bucket.
    pub requests: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// Every prompt token sent upstream, cache hits included.
    pub prompt_tokens: u64,
    /// Prompt tokens actually charged (cache misses).
    pub billable_prompt_tokens: u64,
    pub completion_tokens: u64,
    /// `prompt + completion` as sent. Not the bill.
    pub total_tokens: u64,
    /// `billable_prompt + completion` — the figure that maps to credits.
    ///
    /// Caveat: records written before `cached_tokens` existed have no cache
    /// data, so they are counted as fully billable. That makes this an upper
    /// bound on cost, not an exact bill.
    pub billable_tokens: u64,
    /// Prompt tokens on records that DID report a cache figure. Only these can
    /// say anything about the cache hit rate.
    pub prompt_tokens_with_cache: u64,
    /// Cache misses among `prompt_tokens_with_cache`.
    pub billable_prompt_tokens_with_cache: u64,
    /// Cache hits as a fraction of `prompt_tokens_with_cache`, 0.0–1.0.
    ///
    /// This is the honest number to show. Dividing total cache hits by total
    /// prompt tokens mixes two populations — records with a cache figure and
    /// records without — and reports a rate well below reality. `None` when no
    /// record in the bucket reported a cache figure.
    pub cache_hit_rate: Option<f64>,
    /// Mean wall-clock latency over successful requests.
    pub avg_latency_ms: Option<u64>,
    pub p50_latency_ms: Option<u64>,
}

impl UsageBucket {
    pub fn finish(&mut self, latencies: &mut Vec<u64>) {
        if self.requests > 0 {
            // Rate over the cache-reporting subset only. Using every prompt
            // token as the denominator would count pre-upgrade records (which
            // carry no cache figure) as 100% misses and drag a real 78% down to
            // 19%.
            self.cache_hit_rate = (self.prompt_tokens_with_cache > 0).then(|| {
                (self.prompt_tokens_with_cache
                    .saturating_sub(self.billable_prompt_tokens_with_cache))
                    as f64
                    / self.prompt_tokens_with_cache as f64
            });
        }
        if !latencies.is_empty() {
            latencies.sort_unstable();
            let sum: u64 = latencies.iter().sum();
            self.avg_latency_ms = Some(sum / latencies.len() as u64);
            self.p50_latency_ms = Some(latencies[latencies.len() / 2]);
        }
        self.total_tokens = self.prompt_tokens + self.completion_tokens;
        self.billable_tokens = self.billable_prompt_tokens + self.completion_tokens;
    }

    /// Share of prompt tokens that came from records reporting a cache figure.
    ///
    /// Tells a caller how much to trust `cache_hit_rate`: 1.0 means the rate
    /// covers all traffic, 0.23 means most of the day was not measured.
    pub fn cache_coverage(&self) -> Option<f64> {
        (self.prompt_tokens > 0)
            .then(|| self.prompt_tokens_with_cache as f64 / self.prompt_tokens as f64)
    }
}

/// One row of the per-platform breakdown.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformUsage {
    /// `workbuddy` / `qoder` / `traework` / …
    pub platform: String,
    #[serde(flatten)]
    pub usage: UsageBucket,
    /// Per-model detail inside this platform, busiest first.
    pub models: Vec<ModelUsage>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    /// Route name without the platform prefix.
    pub model: String,
    #[serde(flatten)]
    pub usage: UsageBucket,
}

/// One day in the "Token 与调用趋势" chart.
///
/// Four token figures rather than one because the chart is a stacked bar: the
/// whole point is showing *how much of the traffic was cache re-reads*. A single
/// total would collapse exactly the distinction the chart exists to make.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    /// Beijing-local `YYYY-MM-DD`. The platform resets at Beijing midnight, so
    /// bucketing by any other zone splits one platform day across two bars.
    pub date: String,
    /// Cache reads (prompt tokens served from cache).
    pub cached_tokens: u64,
    /// Cache misses on requests that reported a cache figure — the part that
    /// actually bills.
    pub new_prompt_tokens: u64,
    /// Prompt tokens from records written before `cached_tokens` existed. Kept
    /// separate so the "cache write" stack never silently includes data that
    /// was simply never measured.
    pub unmeasured_prompt_tokens: u64,
    pub completion_tokens: u64,
    /// `request count` — drawn as the dashed line, on the right-hand axis.
    pub requests: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// Mean wall-clock latency over successful requests that day.
    pub avg_latency_ms: Option<u64>,
}

/// Aggregate response for the usage endpoint.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    /// Inclusive start of the reporting window, epoch ms. `None` = no lower bound.
    pub since: Option<i64>,
    /// Inclusive end of the reporting window, epoch ms.
    pub until: i64,
    #[serde(flatten)]
    pub total: UsageBucket,
    /// Every platform seen in the window, largest token use first.
    pub platforms: Vec<PlatformUsage>,
    /// "Today" and "last 30 days" rollups, same shape as the overview cards.
    pub stats: UsageStats,
    /// Live model health (success rate, latency, quarantine), per route.
    pub health: Vec<crate::credits::gateway::ModelHealthReport>,
    /// True when any aggregation hit its row cap, making the numbers a floor.
    pub truncated: bool,
}

/// Per-account claim outcome for one day, so a failed account can be found and
/// claimed by hand instead of being invisible behind a platform total.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimAccountRow {
    pub account_id: String,
    /// Display name (nickname / label), falling back to a short id.
    pub name: String,
    /// `claimed` | `already_claimed` | `pending` | `unavailable` | `failed` | `none`
    pub status: String,
    /// Credits actually added today by this account.
    pub amount: f64,
    /// Reward name as reported by the upstream, e.g. "每日签到".
    pub reward: Option<String>,
    /// Upstream explanation for a non-success status.
    pub message: Option<String>,
}

/// Claim rollup for one platform.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformClaims {
    pub platform: String,
    /// Accounts configured for this platform (enabled or not).
    pub accounts: u64,
    /// Accounts that produced any claim record today.
    pub recorded: u64,
    /// Accounts that successfully added credits today.
    pub claimed: u64,
    /// Accounts already claimed before today (nothing to do, counted as OK).
    pub already_claimed: u64,
    /// Accounts with nothing to claim upstream.
    pub unavailable: u64,
    /// Claims whose result the upstream has not confirmed.
    pub pending: u64,
    /// Accounts with no record at all — usually not yet attempted.
    pub missing: u64,
    /// Credits added today.
    pub credits: f64,
    /// `claimed / (claimed + unavailable + pending + missing)` — how many
    /// accounts actually got their credits. `alreadyClaimed` is excluded because
    /// those accounts already have the credits.
    pub success_rate: Option<f64>,
    /// Every account, worst status first, so problems appear at the top.
    pub rows: Vec<ClaimAccountRow>,
}

/// Today's claim picture across every platform.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimReport {
    pub today_claimed_credits: f64,
    pub platforms: Vec<PlatformClaims>,
}

/// Accurate today/total figures for the overview cards.
///
/// The dashboard also ships `requests` and `rewards` lists, but those are capped
/// at 100 rows for display. Deriving totals from them silently under-reports once
/// there are more than 100 events in a day — with 60+ accounts one morning of
/// check-ins is already past that. Everything in here is aggregated in SQL over
/// the full window instead.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    /// Requests started since local midnight.
    pub today_requests: u64,
    /// Total requests ever recorded.
    pub total_requests: u64,
    /// Prompt tokens sent today (cache hits included).
    pub today_prompt_tokens: u64,
    /// Prompt tokens actually billable today.
    pub today_billable_prompt_tokens: u64,
    /// Prompt tokens on today's requests that reported a cache figure.
    ///
    /// Only this subset can produce an honest cache hit rate: records written
    /// before the `cached_tokens` field existed carry no cache data, and
    /// treating them as full misses is what made a real ~78% show up as 19%.
    pub today_prompt_tokens_with_cache: u64,
    /// Cache misses among `today_prompt_tokens_with_cache`.
    pub today_billable_prompt_tokens_with_cache: u64,
    /// Today's cache hit rate over the reporting subset, 0.0–1.0.
    ///
    /// `None` when nothing reported a cache figure. Compare with
    /// `today_cache_coverage` to judge how much of the day it describes.
    pub today_cache_hit_rate: Option<f64>,
    /// Share of today's prompt tokens that reported a cache figure, 0.0–1.0.
    /// 1.0 means the rate covers all traffic; 0.23 means most of the day was
    /// not measured (the field was added mid-day).
    pub today_cache_coverage: Option<f64>,
    pub today_completion_tokens: u64,
    /// Credits actually *spent* today, summed from the per-request `credit`
    /// figure the upstream reports. This is the real cost number; token counts
    /// are only a proxy. `today_credit_coverage` says what share of today's
    /// requests reported it, because TRAE and Qoder never do.
    pub today_spent_credits: f64,
    /// Requests today that reported a credit value.
    pub today_credit_coverage: u64,
    /// Credits spent over the rolling 30-day window.
    pub month_spent_credits: f64,
    /// Credits actually claimed today, summed over every claim.
    pub today_claimed_credits: f64,
    /// How many distinct accounts claimed something today.
    pub today_claiming_accounts: u64,
    /// Rewards seen today that are claimable but not yet taken.
    pub today_pending_claims: u64,
    pub today_succeeded: u64,
    pub today_failed: u64,
    /// Mean latency over successful requests today.
    pub today_avg_latency_ms: Option<u64>,
    /// Rolling 30-day window, for the month card.
    pub month_prompt_tokens: u64,
    pub month_billable_prompt_tokens: u64,
    pub month_completion_tokens: u64,
    pub month_requests: u64,
    pub month_claimed_credits: f64,
    /// True when the aggregation hit its row cap, so the caller knows the
    /// numbers are a lower bound rather than an exact total.
    pub truncated: bool,
    /// Per-platform claim breakdown for today, including which individual
    /// accounts failed so they can be claimed by hand.
    #[serde(default)]
    pub claims: Vec<PlatformClaims>,
    /// Per-platform request/token totals **for today**. The overview's platform
    /// table must not be derived from the `requests` list, which is capped at
    /// 100 rows and therefore reports "100" for the busiest platform and 0 for
    /// the rest.
    #[serde(default)]
    pub platforms: Vec<PlatformUsage>,
    /// 这批数字是什么时候算出来的（epoch ms）。界面上标出来，避免拿几分钟
    /// 前的快照当实时值去核对——之前就因为这个和官方页面对不上。
    #[serde(default)]
    pub sampled_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub accounts: Vec<Account>,
    pub settings: ProxySettings,
    pub gateway: GatewayStatus,
    pub rewards: Vec<RewardOutcome>,
    pub requests: Vec<RequestSummary>,
    pub refreshing_account_ids: Vec<String>,
    /// Full-window totals; use these for any card that shows a number.
    pub usage: UsageStats,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_settings_enable_balance_refresh_without_enabling_claims() {
        let old: ProxySettings = serde_json::from_str(r#"{"port":17860,"autoClaim":false}"#).unwrap();
        assert!(old.auto_refresh); assert!(!old.auto_claim);
        let mut disabled = old; disabled.auto_refresh = false;
        let roundtrip: ProxySettings = serde_json::from_str(&serde_json::to_string(&disabled).unwrap()).unwrap();
        assert!(!roundtrip.auto_refresh);
    }

    #[test]
    fn old_model_snapshots_keep_new_capabilities_unknown() {
        let saved = serde_json::json!({
            "id":"glm-5.3-flash", "name":"GLM-5.3-Flash", "canonicalId":null,
            "supportsTools":true, "contextWindow":1000000,
            "verified":false, "verificationNote":null
        });
        let model: ModelInfo = serde_json::from_value(saved).unwrap();
        assert_eq!(model.context_window, Some(1000000));
        assert!(model.upstream_model_ids.is_empty());
        assert!(model.context_window_options.is_empty());
        assert_eq!(model.max_output_tokens, None);
        assert_eq!(model.supports_images, None);
        assert_eq!(model.supports_reasoning, None);
        assert!(model.reasoning_effort_options.is_empty());
        assert_eq!(model.default_reasoning_effort, None);
        let exported = serde_json::to_value(model).unwrap();
        assert!(exported["maxOutputTokens"].is_null());
        assert!(exported["supportsImages"].is_null());
        assert!(exported["supportsReasoning"].is_null());
        assert!(exported["defaultReasoningEffort"].is_null());
    }
}
