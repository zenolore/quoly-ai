pub mod activate;
pub mod account_file;
pub mod growth;
pub mod gateway;
pub mod providers;

mod secrets;
mod refresh;
pub mod store;
pub(crate) mod trail;
pub mod types;

use futures::StreamExt;
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AsyncMutex, Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use types::*;

const GATEWAY_API_KEY_ID: &str = "gateway-api-key";

pub struct CreditManager {
    pub store: store::CreditStore,
    pub provider: providers::ProviderClient,
    pub gateway: gateway::GatewayManager,
    secrets: secrets::SecretStore,
    account_gates: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    import_gate: AsyncMutex<()>,
    key_gate: Mutex<()>,
    cancellation: CancellationToken,
    refresh_schedule: Mutex<refresh::RefreshSchedule>,
    refresh_slots: Semaphore,
    refresh_wake: Notify,
    scheduler_started: AtomicBool,
    event_sink: Mutex<Option<tauri::AppHandle>>,
}

/// One day of growth totals, for the history table.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthDay {
    /// YYYY-MM-DD in Beijing local time.
    pub date: String,
    pub credits: i64,
    /// Distinct accounts that ran, so an all-zero day is visibly "did not run".
    pub accounts: usize,
    pub runs: usize,
    pub tasks_claimed: i64,
    pub travel_claimed: i64,
    pub buddy_boxes: i64,
}

/// Whether one account can take part in growth automation yet.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthAccountState {
    pub id: String,
    pub label: String,
    pub has_buddy: bool,
    pub energy: i64,
    pub travel_state: String,
    pub claimed_tasks: i64,
    pub pending_tasks: i64,
    pub ready: bool,
    /// Why the account cannot run, shown in the UI instead of a silent skip.
    pub hint: String,
}

/// Format a millisecond timestamp (already shifted to Beijing) as YYYY-MM-DD.
fn format_day(shifted_ms: i64) -> String {
    const DAY: i64 = 24 * 3600 * 1000;
    let days = shifted_ms.div_euclid(DAY);
    // Civil-from-days: day count since 1970 -> Y/M/D, no leap-year tables needed.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

impl CreditManager {
    pub fn new(conn: Arc<Mutex<rusqlite::Connection>>) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            store: store::CreditStore::new(conn)?,
            provider: providers::ProviderClient::new(),
            gateway: gateway::GatewayManager::new(),
            secrets: secrets::SecretStore::new(),
            account_gates: Mutex::new(HashMap::new()),
            import_gate: AsyncMutex::new(()),
            key_gate: Mutex::new(()),
            cancellation: CancellationToken::new(),
            refresh_schedule: Mutex::new(refresh::RefreshSchedule::new(now_ms())),
            refresh_slots: Semaphore::new(3),
            refresh_wake: Notify::new(),
            scheduler_started: AtomicBool::new(false),
            event_sink: Mutex::new(None),
        }))
    }

    #[cfg(test)]
    pub fn test() -> Arc<Self> {
        Arc::new(Self {
            store: store::CreditStore::new(Arc::new(Mutex::new(
                rusqlite::Connection::open_in_memory().unwrap(),
            )))
            .unwrap(),
            provider: providers::ProviderClient::new(),
            gateway: gateway::GatewayManager::new(),
            secrets: secrets::SecretStore::memory(),
            account_gates: Mutex::new(HashMap::new()),
            import_gate: AsyncMutex::new(()),
            key_gate: Mutex::new(()),
            cancellation: CancellationToken::new(),
            refresh_schedule: Mutex::new(refresh::RefreshSchedule::new(now_ms())),
            refresh_slots: Semaphore::new(3),
            refresh_wake: Notify::new(),
            scheduler_started: AtomicBool::new(false),
            event_sink: Mutex::new(None),
        })
    }

    fn gate(&self, id: &str) -> Arc<AsyncMutex<()>> {
        self.account_gates
            .lock()
            .entry(id.into())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }
    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        self.store.accounts()
    }
    /// A user action may retry a cancelled/denied OS credential request.
    /// Background refreshes and proxy traffic must not clear this pause.
    pub fn retry_secret_access(&self) -> Result<(), String> {
        self.secrets.retry_access()?;
        self.refresh_schedule.lock().resume();
        Ok(())
    }
    pub fn retry_gateway_key_access(&self) -> Result<(), String> {
        self.secrets
            .retry_access_for(GATEWAY_API_KEY_ID)
            .map_err(|error| format!("本地代理密钥不可用：{error}"))
    }
    /// Only the explicit start command may request OS authorization.
    pub async fn start_proxy_for_user(self: &Arc<Self>, port: u16) -> Result<GatewayStatus, String> {
        self.retry_gateway_key_access()?;
        self.gateway.start(self.clone(), port).await
    }
    pub fn credential(&self, account: &Account) -> Result<Credentials, String> {
        let raw = self
            .secrets
            .get(&account.id)?
            .ok_or_else(|| "账号凭据不存在，请重新授权".to_string())?;
        let credentials: Credentials = serde_json::from_str(&raw)
            .map_err(|_| "账号凭据格式无效，请重新授权".to_string())?;
        if credentials.region != account.region {
            return Err("账号与凭据区域不一致，请重新授权".into());
        }
        Ok(credentials)
    }
    fn save_credentials(&self, id: &str, credentials: &Credentials) -> Result<(), String> {
        self.secrets.set(
            id,
            &serde_json::to_string(credentials).map_err(|_| "凭据编码失败".to_string())?,
        )
    }

    pub async fn credentials_for(&self, account: &Account) -> Result<Credentials, String> {
        let credentials = self.credential(account)?;
        if credentials
            .expires_at
            .is_none_or(|expires| expires > now_ms() + 60_000)
        {
            return Ok(credentials);
        }
        let gate = self.gate(&account.id);
        let _guard = gate.lock().await;
        self.credentials_unlocked(account, false)
            .await
            .map_err(|error| error.message)
    }
    pub async fn force_refresh_credentials(
        &self,
        account: &Account,
    ) -> Result<Credentials, String> {
        let gate = self.gate(&account.id);
        let _guard = gate.lock().await;
        self.credentials_unlocked(account, true)
            .await
            .map_err(|error| error.message)
    }
    async fn credentials_unlocked(
        &self,
        account: &Account,
        force: bool,
    ) -> Result<Credentials, providers::ProviderError> {
        let credentials = self
            .credential(account)
            .map_err(|message| {
                let mut error = providers::ProviderError::new(500, message);
                error.pause_auto = true;
                error
            })?;
        if force
            || credentials
                .expires_at
                .is_some_and(|expires| expires <= now_ms() + 60_000)
        {
            let refreshed = self
                .provider
                .refresh_credentials(account.platform, &credentials)
                .await.map_err(|mut error| {
                    // An expired token with no refresh token is also a terminal
                    // credential failure, even without an upstream rejection.
                    if matches!(error.status, 401 | 403) { error.pause_auto = true; }
                    error
                })?;
            if refreshed.region != account.region {
                return Err(providers::ProviderError::new(502, "平台刷新返回了不匹配的账号区域"));
            }
            self.save_credentials(&account.id, &refreshed)
                .map_err(|message| {
                    let mut error = providers::ProviderError::new(500, message);
                    error.pause_auto = true;
                    error
                })?;
            Ok(refreshed)
        } else {
            Ok(credentials)
        }
    }

    pub async fn add_account(
        self: &Arc<Self>,
        platform: Platform,
        label: String,
        credentials: Credentials,
    ) -> Result<Account, String> {
        if credentials.access_token.trim().is_empty() || credentials.access_token.len() > 65536 {
            return Err("请输入有效的账号凭据".into());
        }
        let credentials = if platform == Platform::MinimaxCode
            && credentials.expires_at.is_some_and(|expires| expires <= now_ms() + 60_000) {
            self.provider.refresh_credentials(platform, &credentials).await.map_err(|error| error.message)?
        } else { credentials };
        let requested_region = credentials.region;
        let authentication = self.provider.authenticate(platform, &credentials).await;
        let (user_id, default_label, credentials) = match authentication {
            Err(error)
                if platform == Platform::Traework
                    && error.rejected
                    && matches!(error.status, 401 | 403)
                    && credentials.refresh_token.is_some() =>
            {
                let refreshed = self
                    .provider
                    .refresh_credentials(platform, &credentials)
                    .await
                    .map_err(|error| error.message)?;
                self.provider
                    .authenticate(platform, &refreshed)
                    .await
                    .map_err(|error| error.message)?
            }
            result => result.map_err(|error| error.message)?,
        };
        let _import = self.import_gate.lock().await;
        if credentials.region != requested_region {
            return Err("平台认证返回了不匹配的账号区域".into());
        }
        let existing = self
            .accounts()?
            .into_iter()
            .find(|a| a.platform == platform && a.region == credentials.region && a.user_id == user_id);
        let id = existing
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let gate = self.gate(&id);
        let _guard = gate.lock().await;
        let previous = self.secrets.get_for_update(&id)?;
        self.save_credentials(&id, &credentials)?;
        let account = Account {
            id: id.clone(),
            platform,
            region: credentials.region,
            user_id,
            label: if label.trim().is_empty() {
                default_label
            } else {
                label.trim().chars().take(100).collect()
            },
            enabled: existing.as_ref().is_none_or(|a| a.enabled),
            status: "ready".into(),
            last_error: None,
            snapshot: None,
        };
        if let Err(error) = self.store.save_account(&account) {
            self.secrets.restore(&id, previous.as_deref())?;
            return Err(error);
        }
        drop(_guard);
        drop(_import);
        self.refresh_account(&id).await
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let gate = self.gate(id);
        let _guard = gate.lock().await;
        let mut account = self.store.account(id)?;
        account.enabled = enabled;
        self.store.save_account(&account)
    }
    pub async fn delete_account(&self, id: &str) -> Result<(), String> {
        let gate = self.gate(id);
        let _guard = gate.lock().await;
        self.store.account(id)?;
        // Keep a recoverable copy before deletion in case the database rejects it.
        let previous = self.secrets.get(id)?;
        self.secrets.delete(id)?;
        if let Err(error) = self.store.delete_account(id) {
            if let Some(previous) = previous {
                self.secrets.set(id, &previous)?;
            }
            return Err(error);
        }
        Ok(())
    }

    pub async fn refresh_account(self: &Arc<Self>, id: &str) -> Result<Account, String> {
        self.refresh_scope(id, refresh::Scope::FULL, true).await
    }

    pub async fn refresh(self: &Arc<Self>, id: Option<&str>) -> Result<Vec<Account>, String> {
        let accounts = if let Some(id) = id {
            vec![self.store.account(id)?]
        } else {
            self.accounts()?.into_iter().filter(|a| a.enabled).collect()
        };
        let results = futures::stream::iter(accounts)
            .map(|a| async move { self.refresh_account(&a.id).await })
            .buffer_unordered(3)
            .collect::<Vec<_>>()
            .await;
        results.into_iter().collect()
    }

    pub async fn claim(self: &Arc<Self>, id: Option<&str>) -> Result<Vec<RewardOutcome>, String> {
        let mut accounts = if let Some(id) = id {
            vec![self.store.account(id)?]
        } else {
            self.accounts()?.into_iter().filter(|a| a.enabled).collect()
        };
        // Claim serially rather than concurrently. Some platforms cap check-in
        // per *device* (not per account), so firing three accounts at once makes
        // the winner nondeterministic: whichever request lands first takes the
        // slot and the rest are rejected. Serial order keeps today's slot on the
        // first eligible account in list order, which is predictable.
        //
        // Predictable is not the same as fair, though. TRAE allows one check-in
        // per device per day and this machine has one registered device id, so
        // the first account in list order would take the only slot every day and
        // the others would never be paid. Order by how long each account has
        // gone without a successful claim instead, so the slot rotates: the
        // account that has waited longest goes first.
        if id.is_none() {
            if let Ok(last) = self.store.last_reward_at() {
                accounts.sort_by_key(|a| last.get(&a.id).copied().unwrap_or(0));
            }
        }
        let mut all = Vec::new();
        for account in accounts {
            if self.cancellation.is_cancelled() {
                break;
            }
            all.extend(self.claim_account(&account.id).await?);
        }
        Ok(all)
    }

    /// Activate accounts that have no buddy, one event each, spaced apart.
    ///
    /// Pilot-scoped on purpose: `account_ids` limits the blast radius, so three
    /// accounts can be observed for a couple of days before the rest are touched.
    ///
    /// Spacing rule: a per-account daily ceiling (one report per Beijing day, read
    /// back from the audit table so a restart cannot double-send) plus a random
    /// 3-6 minute gap between accounts. The gap is not about rate limits — one
    /// event per account is far below any threshold — it keeps a batch of accounts
    /// from producing an identical, machine-regular request pattern.
    pub async fn activate_accounts(
        self: &Arc<Self>,
        account_ids: Vec<String>,
    ) -> Result<Vec<activate::ActivationResult>, String> {
        let now = now_ms();
        const DAY: i64 = 24 * 3600 * 1000;
        let offset: i64 = 8 * 3600 * 1000;
        let today_start = ((now + offset) / DAY) * DAY - offset;

        let mut out = Vec::new();
        for (index, id) in account_ids.iter().enumerate() {
            if self.cancellation.is_cancelled() {
                break;
            }
            let account = self.store.account(id)?;
            if account.platform != types::Platform::Workbuddy {
                continue;
            }
            // Daily ceiling from the audit trail, not from memory.
            if self.store.activated_since(id, today_start).unwrap_or(false) {
                out.push(activate::ActivationResult {
                    account_id: id.clone(),
                    label: account.label.clone(),
                    outcome: "skipped_today".into(),
                    note: "今天已上报过，跳过（每账号每天 1 次）".into(),
                    ..Default::default()
                });
                continue;
            }

            // Random 3-6 minute gap before each account except the first, so a
            // batch never looks like a single scripted sweep.
            if index > 0 {
                let wait_secs = 180 + (now_ms() % 181);
                tokio::time::sleep(std::time::Duration::from_secs(wait_secs as u64)).await;
            }

            let result = match self.credentials_for(&account).await {
                Ok(cred) => activate::activate(&self.provider, &cred, id, &account.label).await,
                Err(e) => activate::ActivationResult {
                    account_id: id.clone(),
                    label: account.label.clone(),
                    outcome: "failed".into(),
                    note: e.chars().take(80).collect(),
                    ..Default::default()
                },
            };
            if let Ok(detail) = serde_json::to_value(&result) {
                let _ = self.store.record_activation(id, &result.outcome, &detail, now_ms());
            }
            out.push(result);
        }
        Ok(out)
    }

    /// Recent activation attempts (audit view).
    pub fn activations(&self, limit: usize) -> Result<Vec<(String, String, serde_json::Value, i64)>, String> {
        self.store.activations(limit.clamp(1, 200))
    }

    /// Per-day token / request totals for the trend chart, oldest first.
    ///
    /// Same Beijing-local day boundaries as `growth_daily` — one platform day
    /// must never be split across two bars. Unlike `growth_daily` the caller
    /// already reverses for display, so this returns chronological order: a
    /// chart plots left-to-right, and reversing at the call site is how
    /// off-by-one bugs get in.
    ///
    /// Deliberately reads `requests_between` over the whole window in ONE pass
    /// rather than inside the day loop. With 30 days x ~13k rows/day a per-day
    /// query would be 30 scans; one scan bucketed in memory is the same work
    /// the overview cards already do.
    ///
    /// Token accounting mirrors `usage_stats` exactly — `billable = prompt -
    /// cached` only when a cache figure was reported, otherwise the full prompt
    /// is charged. Anything else would make the chart disagree with the card
    /// directly above it, which is the bug this whole screen exists to avoid.
    pub fn usage_daily(&self, days: u32) -> Result<Vec<crate::credits::types::UsageDay>, String> {
        use crate::credits::types::UsageDay;
        use std::collections::BTreeMap;

        let days = days.clamp(1, 90);
        const DAY: i64 = 86_400_000;
        // Shift by +8h so truncating to a day boundary yields Beijing midnight.
        const OFFSET: i64 = 8 * 3_600_000;
        let today_start = ((now_ms() + OFFSET) / DAY) * DAY - OFFSET;
        let window_start = today_start - (days as i64 - 1) * DAY;

        // Pre-seed every day so a day with no traffic renders as an empty bar
        // rather than disappearing (which reads as "the gateway was down").
        let mut buckets: BTreeMap<i64, UsageDay> = BTreeMap::new();
        for index in 0..days as i64 {
            let start = window_start + index * DAY;
            buckets.insert(
                start,
                UsageDay {
                    date: format_day(start + OFFSET),
                    ..Default::default()
                },
            );
        }

        let rows = self
            .store
            .requests_between(Some(window_start), today_start + DAY, USAGE_ROW_CAP)?;

        let mut latencies: std::collections::HashMap<i64, Vec<u64>> =
            std::collections::HashMap::new();
        for request in &rows {
            let key = ((request.created_at + OFFSET) / DAY) * DAY - OFFSET;
            let Some(day) = buckets.get_mut(&key) else {
                continue;
            };
            let prompt = request.input_tokens.unwrap_or(0);
            let completion = request.output_tokens.unwrap_or(0);
            day.requests += 1;
            day.completion_tokens += completion;
            match request.cached_tokens {
                Some(cached) => {
                    let cached = cached.min(prompt);
                    day.cached_tokens += cached;
                    day.new_prompt_tokens += prompt - cached;
                }
                // No cache figure: charge the whole prompt, and keep it in its
                // own bucket so the chart never implies a measured cache write.
                None => day.unmeasured_prompt_tokens += prompt,
            }
            if request.status == "success" {
                day.succeeded += 1;
                if request.elapsed_ms > 0 {
                    latencies.entry(key).or_default().push(request.elapsed_ms);
                }
            } else {
                day.failed += 1;
            }
        }

        for (key, bucket) in buckets.iter_mut() {
            if let Some(values) = latencies.get(key) {
                if !values.is_empty() {
                    let sum: u64 = values.iter().sum();
                    bucket.avg_latency_ms = Some(sum / values.len() as u64);
                }
            }
        }
        Ok(buckets.into_values().collect())
    }

    /// Per-day totals for the last `days` days, newest first.
    ///
    /// Days are Beijing-local: the platform resets rewards on Beijing midnight,
    /// so bucketing by any other zone would split one platform day across two
    /// report rows. Empty days are included so a gap in the chart means "the pass
    /// did not run" rather than "no data was kept".
    pub fn growth_daily(&self, days: u32) -> Result<Vec<GrowthDay>, String> {
        let now = now_ms();
        const DAY: i64 = 24 * 3600 * 1000;
        // Shift by +8h so truncating to a day boundary yields Beijing midnight.
        let offset: i64 = 8 * 3600 * 1000;
        let today_start = ((now + offset) / DAY) * DAY - offset;

        let mut out = Vec::new();
        for index in 0..days {
            let from = today_start - index as i64 * DAY;
            let runs = self.store.growth_runs(from, from + DAY, 5000)?;
            let mut credits = 0i64;
            let mut accounts = std::collections::BTreeSet::new();
            let mut tasks_claimed = 0i64;
            let mut travel_claimed = 0i64;
            let mut boxes = 0i64;
            for (account_id, outcome, earned, _) in &runs {
                credits += earned;
                accounts.insert(account_id.clone());
                tasks_claimed += outcome.get("tasksClaimed").and_then(serde_json::Value::as_i64).unwrap_or(0);
                travel_claimed += outcome.get("travelClaimed").and_then(serde_json::Value::as_i64).unwrap_or(0);
                boxes += outcome.get("buddyBoxes").and_then(serde_json::Value::as_i64).unwrap_or(0);
            }
            out.push(GrowthDay {
                date: format_day(from + offset),
                credits,
                accounts: accounts.len(),
                runs: runs.len(),
                tasks_claimed,
                travel_claimed,
                buddy_boxes: boxes,
            });
        }
        Ok(out)
    }

    /// Account readiness for growth automation.
    ///
    /// An account with no buddy instance cannot accept tasks at all — the service
    /// rejects them with `prerequisite not met: first_buddy`. That needs one real
    /// conversation in the desktop client, so it is reported rather than retried.
    pub async fn growth_accounts(&self) -> Result<Vec<GrowthAccountState>, String> {
        let mut out = Vec::new();
        for account in self.accounts()?.into_iter().filter(|a| a.enabled) {
            if account.platform != types::Platform::Workbuddy {
                continue;
            }
            let mut state = GrowthAccountState {
                id: account.id.clone(),
                label: account.label.clone(),
                has_buddy: false,
                energy: 0,
                travel_state: String::new(),
                claimed_tasks: 0,
                pending_tasks: 0,
                ready: false,
                hint: String::new(),
            };
            if let Ok(cred) = self.credentials_for(&account).await {
                if let Ok(snap) = growth::snapshot(&self.provider, &cred).await {
                    state.energy = snap.energy;
                    state.travel_state = snap.travel_state.clone();
                    state.claimed_tasks = snap.tasks.iter().filter(|t| t.status == "claimed").count() as i64;
                    state.pending_tasks = snap.tasks.iter().filter(|t| t.status == "not_accepted").count() as i64;
                    // A buddy implies at least one of these is set; the snapshot has
                    // no explicit has_buddy, so derive it from energy or a buddy task.
                    state.has_buddy = snap.energy > 0 || state.claimed_tasks > 0;
                }
            }
            state.ready = state.has_buddy;
            state.hint = if state.has_buddy {
                String::new()
            } else {
                "需在客户端新建 1 个任务并发起一次对话，领养 Buddy 后才能领取成长奖励".into()
            };
            out.push(state);
        }
        Ok(out)
    }

    /// Run the growth-center automation for one account or all enabled ones.
    ///
    /// Serial, for the same reason check-in is serial: several steps here are
    /// per-account daily-capped, and firing them concurrently makes which account
    /// wins nondeterministic. Growth work is also heavier (up to a dozen calls per
    /// account), so a burst would look like scripted traffic.
    pub async fn run_growth(
        self: &Arc<Self>,
        id: Option<&str>,
    ) -> Result<Vec<growth::GrowthOutcome>, String> {
        let accounts = if let Some(id) = id {
            vec![self.store.account(id)?]
        } else {
            self.accounts()?.into_iter().filter(|a| a.enabled).collect()
        };
        let mut out = Vec::new();
        for account in accounts {
            if self.cancellation.is_cancelled() {
                break;
            }
            if account.platform != types::Platform::Workbuddy {
                continue;
            }
            let outcome = match self.credentials_for(&account).await {
                Ok(cred) => match growth::run(&self.provider, &cred).await {
                    Ok(o) => o,
                    Err(e) => growth::GrowthOutcome {
                        steps: vec![format!("失败：{}", e.message.chars().take(80).collect::<String>())],
                        ..Default::default()
                    },
                },
                Err(e) => growth::GrowthOutcome {
                    steps: vec![format!("凭据不可用：{}", e.chars().take(60).collect::<String>())],
                    ..Default::default()
                },
            };
            out.push(growth::GrowthOutcome {
                // The account is needed by the UI to attribute each row; carried in
                // the first step to avoid widening the struct for one field.
                steps: {
                    let mut v = vec![format!("账号 {}", account.label)];
                    v.extend(outcome.steps.clone());
                    v
                },
                ..outcome.clone()
            });
            // Persist regardless of whether anything was earned: a "ran, got
            // nothing" row is information (it distinguishes an idle account from an
            // account the pass never reached), and the daily report depends on it.
            let earned = outcome.total_credits();
            if let Ok(value) = serde_json::to_value(&outcome) {
                let _ = self.store.record_growth(&account.id, &value, earned, now_ms());
            }
        }
        Ok(out)
    }

    /// Read-only growth snapshot for one account.
    pub async fn growth_snapshot(&self, id: &str) -> Result<growth::GrowthSnapshot, String> {
        let account = self.store.account(id)?;
        if account.platform != types::Platform::Workbuddy {
            return Err("成长中心仅支持 WorkBuddy 账号".into());
        }
        let cred = self.credentials_for(&account).await?;
        growth::snapshot(&self.provider, &cred)
            .await
            .map_err(|e| e.message)
    }

    async fn claim_account(self: &Arc<Self>, id: &str) -> Result<Vec<RewardOutcome>, String> {
        let gate = self.gate(id);
        let guard = gate.lock().await;
        let account = self.store.account(id)?;
        if !account.enabled {            return Ok(Vec::new());
        }
        let result = match self.credentials_unlocked(&account, false).await {
            Ok(credentials) => {
                let mut result = self.provider.claim(&account, &credentials).await;
                if result
                    .as_ref()
                    .is_err_and(|e| e.rejected && matches!(e.status, 401 | 403))
                {
                    result = match self.credentials_unlocked(&account, true).await {
                        Ok(refreshed) => self.provider.claim(&account, &refreshed).await,
                        Err(error) => Err(error),
                    };
                }
                result.map_err(|e| e.message)
            }
            Err(error) => Err(error.message),
        };
        let outcomes = result.unwrap_or_else(|error| {
            vec![RewardOutcome {
                account_id: id.into(),
                reward_id: "claim".into(),
                name: "领取奖励".into(),
                status: "failed".into(),
                amount: None,
                kind: "credits".into(),
                message: error,
                created_at: now_ms(),
            }]
        });
        for outcome in &outcomes {
            self.store.record_reward(outcome)?;
        }
        drop(guard);
        // A delayed/rate-limited balance read must not turn a recorded claim into
        // an apparent claim failure that invites the user to submit it again.
        let updated = self.refresh_account(id).await.unwrap_or_else(|_| account.clone());
        if outcomes.iter().any(|outcome| outcome.status == "pending"
            || (matches!(outcome.status.as_str(), "claimed" | "success")
                && account.snapshot.as_ref().and_then(|snapshot| snapshot.balance)
                    == updated.snapshot.as_ref().and_then(|snapshot| snapshot.balance))) {
            self.refresh_schedule.lock().account(&updated).recheck_claim(now_ms() + refresh::CLAIM_RECHECK_DELAY);
            self.refresh_wake.notify_one();
        }
        Ok(outcomes)
    }

    pub fn gateway_key(&self) -> Result<String, String> {
        let _guard = self.key_gate.lock();
        if let Some(key) = self.secrets.get(GATEWAY_API_KEY_ID)? {
            return Ok(key);
        }
        let key = new_key();
        self.secrets.set(GATEWAY_API_KEY_ID, &key)?;
        Ok(key)
    }
    pub fn record_request(&self, summary: RequestSummary) -> Result<(), String> {
        self.store.record_request(&summary)
    }
    pub async fn invalidate_model(
        &self,
        id: &str,
        model_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        let gate = self.gate(id);
        let _guard = gate.lock().await;
        let mut account = self.store.account(id)?;
        if let Some(snapshot) = &mut account.snapshot {
            if let Some(model) = snapshot.models.iter_mut().find(|m| m.id == model_id) {
                model.verified = false;
                model.verification_note = Some(reason.into());
            }
        }
        self.store.save_account(&account)
    }
    pub fn mark_stale(self: &Arc<Self>, id: &str) {
        #[cfg(test)]
        {
            // Offline fixtures must never dispatch their synthetic tokens to real providers.
            if let Ok(mut account) = self.store.account(id) {
                if let Some(snapshot) = &mut account.snapshot {
                    snapshot.stale = true;
                    let _ = self.store.save_account(&account);
                }
            }
        }
        self.refresh_schedule.lock().accounts.entry(id.into()).or_default()
            .dirty(now_ms() + refresh::PROXY_DELAY);
        self.refresh_wake.notify_one();
        self.emit_credits_changed();
    }
    /// userId → 账号 id。导入时用来判断「这号库里已经有了」。
    pub fn existing_user_ids(&self) -> Result<std::collections::HashMap<String, String>, String> {
        let mut map = std::collections::HashMap::new();
        for account in self.accounts()? {
            map.insert(account.user_id.clone(), account.id.clone());
        }
        Ok(map)
    }

    /// 逐个导入解析好的账号。
    ///
    /// 已存在的走 `add_account` 更新凭据并改标签；新的直接新增。每个都单独
    /// 报结果——批量导入里一个号失效不该拖垮其余。
    pub async fn import_parsed(
        self: &Arc<Self>,
        parsed: account_file::ParseReport,
        suffix: &str,
    ) -> Result<Vec<crate::commands::credits::ImportOutcome>, String> {
        use crate::commands::credits::ImportOutcome;

        let mut out = Vec::new();
        for item in parsed.accounts {
            let label = account_file::tagged_label(&item.nickname, suffix);

            // 静态检查就不合格的直接记账，不去碰平台接口
            if !item.problems.is_empty() {
                out.push(ImportOutcome {
                    nickname: item.nickname,
                    label,
                    ok: false,
                    message: item.problems.join("；"),
                });
                continue;
            }

            // add_account 自己按 platform+region+user_id 找已有账号并复用 id，
            // 所以这里不需要区分新增还是更新。
            match self
                .add_account(types::Platform::Workbuddy, label.clone(), item.credentials)
                .await
            {
                Ok(_) => out.push(ImportOutcome {
                    nickname: item.nickname,
                    label,
                    ok: true,
                    message: "已校验并保存".into(),
                }),
                Err(message) => out.push(ImportOutcome {
                    nickname: item.nickname,
                    label,
                    ok: false,
                    message,
                }),
            }
        }
        Ok(out)
    }

    pub fn dashboard(&self) -> Result<Dashboard, String> {
        let settings = self.store.settings()?;
        let mut accounts = self.accounts()?;
        let schedule = self.refresh_schedule.lock();
        for account in &mut accounts {
            if schedule.accounts.get(&account.id).is_some_and(|state| state.has_pending_balance()) {
                if let Some(snapshot) = &mut account.snapshot { snapshot.stale = true; }
            }
        }
        Ok(Dashboard {
            accounts,
            gateway: self.gateway.status(settings.port),
            settings,
            rewards: self.store.rewards()?,
            requests: self.store.requests()?,
            usage: self.usage_stats().map(|mut stats| {
                stats.sampled_at = crate::credits::types::now_ms();
                stats
            }).unwrap_or_default(),
            refreshing_account_ids: schedule.accounts.iter()
                .filter(|(_, state)| state.running.is_some()).map(|(id, _)| id.clone()).collect(),
        })
    }
    pub async fn save_settings(&self, settings: ProxySettings) -> Result<ProxySettings, String> {
        if self.gateway.status(settings.port).running
            && settings.port != self.store.settings()?.port
        {
            return Err("请先停止代理再修改端口".into());
        }
        self.store.save_settings(&settings)?;
        self.refresh_wake.notify_one();
        self.emit_credits_changed();
        Ok(settings)
    }

    pub async fn probe_proxy(&self) -> Result<String, String> {
        let status = self.gateway.status(self.store.settings()?.port);
        if !status.running {
            return Err("请先开启代理".into());
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|_| "连接测试初始化失败".to_string())?;
        let response = client
            .get(format!("{}/models", status.base_url))
            .bearer_auth(self.gateway_key()?)
            .send()
            .await
            .map_err(|_| "无法连接本地代理".to_string())?;
        if !response.status().is_success() {
            return Err("代理认证测试失败".into());
        }
        let models: serde_json::Value = response
            .json()
            .await
            .map_err(|_| "代理返回格式错误".to_string())?;
        let count = models["data"].as_array().map_or(0, Vec::len);
        Ok(format!("代理连接正常，当前有 {count} 个可用模型入口"))
    }

    pub fn start_scheduler(self: &Arc<Self>) {
        if self.scheduler_started.swap(true, Ordering::SeqCst) { return; }
        self.start_refresh_scheduler();
        let manager = self.clone();
        tokio::spawn(async move {
            let mut last_day: Option<i64> = None;
            // Retry cadence for accounts whose daily claim ended the day as
            // "claimable" (upstream 9074 "too many participants" and similar).
            // 10min, 30min, 1h, 2h, 4h ... capped hourly until 23:00 Beijing.
            let mut retry_attempt: usize = 0;
            let retry_delays_ms = [600_000i64, 1_800_000, 3_600_000, 7_200_000, 14_400_000];
            let mut next_retry_at: Option<i64> = None;
            // The first tick is immediate, so startup catch-up follows the same schedule.
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = manager.cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        let now = now_ms();
                        let (day, due) = claim_schedule(now);
                        if due && last_day != Some(day) && manager.store.settings().is_ok_and(|s| s.auto_claim) {
                            last_day = Some(day);
                            retry_attempt = 0;
                            next_retry_at = None;
                            tokio::select! {
                                _ = manager.cancellation.cancelled() => break,
                                _ = manager.claim(None) => {},
                            }
                            // Growth pass follows check-in on the same tick: several
                            // growth steps depend on the check-in having run first
                            // (streak day counted, energy credited), so running them
                            // concurrently would drop rewards.
                            if manager.store.settings().is_ok_and(|s| s.auto_growth) {
                                tokio::select! {
                                    _ = manager.cancellation.cancelled() => break,
                                    result = manager.run_growth(None) => {
                                        if let Ok(outcomes) = result {
                                            let total: i64 = outcomes.iter().map(|o| o.total_credits()).sum();
                                            if total > 0 {
                                                crate::credits::trail::mark(&format!("growth auto +{total}"));
                                            }
                                        }
                                    },
                                }
                            }
                            // Schedule the catch-up pass; it no-ops when every
                            // enabled account really claimed.
                            next_retry_at = Some(now + retry_delays_ms[0]);
                        } else if let Some(due_at) = next_retry_at {
                            if now >= due_at && manager.store.settings().is_ok_and(|s| s.auto_claim) {
                                if let Ok(accounts) = manager.accounts() {
                                    let day_bounds = claim_day_bounds(now);
                                    let unclaimed: Vec<&str> = accounts
                                        .iter()
                                        .filter(|a| a.enabled)
                                        .filter(|a| manager.unclaimed_today(a, &day_bounds))
                                        .map(|a| a.id.as_str())
                                        .collect();
                                    if unclaimed.is_empty() || retry_attempt + 1 >= retry_delays_ms.len() + 4 {
                                        next_retry_at = None;
                                    } else {
                                        retry_attempt += 1;
                                        let delay = retry_delays_ms[retry_attempt.min(retry_delays_ms.len() - 1)];
                                        next_retry_at = Some(now + delay);
                                        if !unclaimed.is_empty() {
                                            for id in unclaimed {
                                                tokio::select! {
                                                    _ = manager.cancellation.cancelled() => break,
                                                    _ = manager.claim(Some(id)) => {},
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    next_retry_at = None;
                                }
                            }
                        }
                    }
                }
            }
        });
    }
    pub async fn shutdown(&self) {
        self.cancellation.cancel();
        let _ = self.gateway.stop().await;
    }
}

fn new_key() -> String {
    format!(
        "ait_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
fn claim_schedule(now: i64) -> (i64, bool) {
    let beijing = now / 1000 + 8 * 3600;
    (
        beijing.div_euclid(86400),
        // 每天 08:00（北京时间）开始第一轮领取。
        //
        // 原本是 10:30。改早的原因：WorkBuddy 那批号在 08:49 之前就被外部
        // （卖家托管或客户端自动签到）签掉了，我们 10:30 去领只能收到
        // "已签过"，账上记 0 分。08:00 在用户要求的 8–10 点区间里取最早，
        // 抢在前面才有机会把分记在我们自己的账上。
        beijing.rem_euclid(86400) >= 8 * 3600,
    )
}
/// [start, end) epoch-ms window of the current Beijing calendar day.
fn claim_day_bounds(now: i64) -> (i64, i64) {
    let beijing_secs = now / 1000 + 8 * 3600;
    let day_start_secs = beijing_secs.div_euclid(86_400) * 86_400;
    let start = (day_start_secs - 8 * 3600) * 1000;
    (start, start + 86_400 * 1000)
}

/// Row ceiling for one aggregation pass. Callers learn via `truncated` when it
/// was hit, rather than silently getting a partial sum.
const USAGE_ROW_CAP: usize = 200_000;

/// Rank claim statuses so one account's several records collapse to the most
/// meaningful outcome.
fn claim_rank(status: &str) -> u8 {
    match status {
        "claimed" => 5,
        "pending" => 4,
        "already_claimed" => 3,
        "unavailable" => 2,
        "failed" => 1,
        _ => 0,
    }
}

/// Build today's claim picture, broken down by platform and then by account.
///
/// A platform total hides *which* account failed. With 27 accounts spread over
/// three platforms, "TRAE 1/8" is only actionable if the user can see the other
/// seven and retry them by hand.
fn build_claim_report(
    store: &crate::credits::store::CreditStore,
    day_start: i64,
    day_end: i64,
) -> Result<Vec<crate::credits::types::PlatformClaims>, String> {
    use std::collections::HashMap;
    use crate::credits::types::{ClaimAccountRow, PlatformClaims};

    let accounts = store.accounts()?;
    let rewards = store.rewards_between(Some(day_start), day_end, USAGE_ROW_CAP)?;

    struct Won {
        status: String,
        amount: f64,
        reward: Option<String>,
        message: Option<String>,
    }

    let mut per_account: HashMap<String, Won> = HashMap::new();
    for reward in rewards {
        let candidate = Won {
            status: reward.status.clone(),
            // Only a confirmed claim adds credits.
            amount: if reward.status == "claimed" {
                reward.amount.unwrap_or(0.0)
            } else {
                0.0
            },
            reward: Some(reward.name.clone()).filter(|n| !n.is_empty()),
            message: Some(reward.message.clone()).filter(|m| !m.is_empty()),
        };
        match per_account.entry(reward.account_id.clone()) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(candidate);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                let existing = slot.get();
                let (winner, other_amount) = if claim_rank(&candidate.status) > claim_rank(&existing.status) {
                    (candidate, existing.amount)
                } else {
                    (
                        Won {
                            status: existing.status.clone(),
                            amount: existing.amount,
                            reward: existing.reward.clone(),
                            message: existing.message.clone(),
                        },
                        candidate.amount,
                    )
                };
                slot.insert(Won {
                    amount: winner.amount.max(other_amount),
                    ..winner
                });
            }
        }
    }

    let mut rows_by_platform: HashMap<String, Vec<ClaimAccountRow>> = HashMap::new();
    let mut totals: HashMap<String, [u64; 5]> = HashMap::new(); // accounts, claimed, already, unavailable, pending
    let mut credits: HashMap<String, f64> = HashMap::new();

    for account in &accounts {
        let platform = account.platform.route_key(account.region).to_string();
        let won = per_account.get(&account.id);
        let (status, amount, reward, message) = match won {
            Some(w) => (w.status.clone(), w.amount, w.reward.clone(), w.message.clone()),
            None => ("none".to_string(), 0.0, None, None),
        };
        let slot = totals.entry(platform.clone()).or_insert([0; 5]);
        slot[0] += 1;
        match status.as_str() {
            "claimed" => {
                slot[1] += 1;
                *credits.entry(platform.clone()).or_default() += amount;
            }
            "already_claimed" => slot[2] += 1,
            "pending" => slot[4] += 1,
            "unavailable" | "failed" => slot[3] += 1,
            _ => {}
        }
        rows_by_platform.entry(platform).or_default().push(ClaimAccountRow {
            account_id: account.id.clone(),
            name: {
                let label = account.label.trim();
                if label.is_empty() {
                    account.id.chars().take(8).collect()
                } else {
                    label.to_string()
                }
            },
            status,
            amount,
            reward,
            message,
        });
    }

    let mut out: Vec<PlatformClaims> = totals
        .into_iter()
        .map(|(platform, [accounts, claimed, already, unavailable, pending])| {
            let mut rows = rows_by_platform.remove(&platform).unwrap_or_default();
            let recorded = rows.iter().filter(|r| r.status != "none").count() as u64;
            let missing = accounts.saturating_sub(recorded);
            // `already_claimed` is excluded from the denominator: those accounts
            // already hold today's credits, so counting them as failures would
            // make a healthy platform look broken.
            let denominator = claimed + unavailable + pending + missing;
            let success_rate = (denominator > 0).then(|| claimed as f64 / denominator as f64);
            // Problems first — spotting what still needs action is the point.
            rows.sort_by_key(|r| {
                (claim_rank_complaint_order(&r.status), r.name.clone())
            });
            // Take the credit total before `platform` is moved into the struct.
            let platform_credits = credits.remove(&platform).unwrap_or(0.0);
            PlatformClaims {
                platform,
                accounts,
                recorded,
                claimed,
                already_claimed: already,
                unavailable,
                pending,
                missing,
                credits: platform_credits,
                success_rate,
                rows,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        a.success_rate
            .unwrap_or(1.0)
            .partial_cmp(&b.success_rate.unwrap_or(1.0))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.credits.partial_cmp(&a.credits).unwrap_or(std::cmp::Ordering::Equal))
    });
    Ok(out)
}

/// Sort key for the account list: trouble first, resolved states last.
fn claim_rank_complaint_order(status: &str) -> u8 {
    match status {
        "failed" => 0,
        "unavailable" => 1,
        "pending" => 2,
        "none" => 3,
        "claimed" => 4,
        "already_claimed" => 5,
        _ => 6,
    }
}

impl CreditManager {
    /// Full-window usage totals for the overview cards.
    ///
    /// Deliberately does *not* reuse the `requests()` / `rewards()` lists the
    /// dashboard ships for display: those are capped at 100 rows, so once a day
    /// produces more events than that every total derived from them is too low.
    pub fn usage_stats(&self) -> Result<UsageStats, String> {
        let now = crate::credits::types::now_ms();
        let (day_start, day_end) = claim_day_bounds(now);
        let month_start = day_start - 29 * 86_400_000;

        let requests = self.store.requests_between(
            Some(month_start.min(day_start)),
            day_end,
            USAGE_ROW_CAP,
        )?;
        let rewards = self.store.rewards_between(Some(month_start), day_end, USAGE_ROW_CAP)?;
        let (total_requests, _) = self.store.history_counts()?;

        let mut stats = UsageStats {
            total_requests,
            ..Default::default()
        };
        let mut today_latencies: Vec<u64> = Vec::new();
        let mut claiming_accounts = std::collections::HashSet::new();

        // Today's per-platform totals, accumulated in the same pass so the
        // overview table never has to touch the 100-row `requests` list.
        let mut per_platform: std::collections::HashMap<
            String,
            (UsageBucket, Vec<u64>),
        > = std::collections::HashMap::new();

        for request in &requests {
            let is_today = request.created_at >= day_start;
            let prompt = request.input_tokens.unwrap_or(0);
            let completion = request.output_tokens.unwrap_or(0);
            // No cache figure recorded -> charge the whole prompt, never less.
            let billable = match request.cached_tokens {
                Some(cached) => prompt.saturating_sub(cached),
                None => prompt,
            };
            let ok = request.status == "success";

            if is_today {
                stats.today_requests += 1;
                stats.today_prompt_tokens += prompt;
                stats.today_billable_prompt_tokens += billable;
                stats.today_completion_tokens += completion;
                // Subset that reports a cache figure — the only population a
                // hit rate can honestly be computed from.
                if request.cached_tokens.is_some() {
                    stats.today_prompt_tokens_with_cache += prompt;
                    stats.today_billable_prompt_tokens_with_cache += billable;
                }
                if let Some(credit) = request.credit {
                    stats.today_spent_credits += credit;
                    stats.today_credit_coverage += 1;
                }
                if ok {
                    stats.today_succeeded += 1;
                    if request.elapsed_ms > 0 {
                        today_latencies.push(request.elapsed_ms);
                    }
                } else {
                    stats.today_failed += 1;
                }

                let platform = request.platform.route_key(request.region).to_string();
                let entry = per_platform.entry(platform).or_default();
                entry.0.requests += 1;
                entry.0.prompt_tokens += prompt;
                entry.0.billable_prompt_tokens += billable;
                entry.0.completion_tokens += completion;
                if request.cached_tokens.is_some() {
                    entry.0.prompt_tokens_with_cache += prompt;
                    entry.0.billable_prompt_tokens_with_cache += billable;
                }
                if ok {
                    entry.0.succeeded += 1;
                    if request.elapsed_ms > 0 {
                        entry.1.push(request.elapsed_ms);
                    }
                } else {
                    entry.0.failed += 1;
                }
            }
            stats.month_requests += 1;
            if let Some(credit) = request.credit {
                stats.month_spent_credits += credit;
            }
            stats.month_prompt_tokens += prompt;
            stats.month_billable_prompt_tokens += billable;
            stats.month_completion_tokens += completion;
        }

        for reward in &rewards {
            if reward.created_at < month_start {
                continue;
            }
            let is_today = reward.created_at >= day_start;
            match reward.status.as_str() {
                "claimed" => {
                    let amount = reward.amount.unwrap_or(0.0);
                    stats.month_claimed_credits += amount;
                    if is_today {
                        stats.today_claimed_credits += amount;
                        claiming_accounts.insert(reward.account_id.clone());
                    }
                }
                "pending" if is_today => stats.today_pending_claims += 1,
                _ => {}
            }
        }
        stats.today_claiming_accounts = claiming_accounts.len() as u64;

        // 今天新发放的资源包总额——不管是谁领的。
        //
        // 为什么需要这个：`today_claimed_credits` 只统计本程序经手的奖励，
        // 用户装了第二个签到工具时那个工具领的分完全看不见，界面上就会
        // 显得「怎么才这么点」。平台的包里带着发放时间，按自然日一筛就能
        // 拿到"今天到账多少"，作为交叉验证的真值。
        for account in self.store.accounts().unwrap_or_default() {
            let Some(snapshot) = account.snapshot.as_ref() else {
                continue;
            };
            for pack in &snapshot.packs {
                let Some(created) = pack.created_at else {
                    continue;
                };
                if created < day_start || created >= day_end {
                    continue;
                }
                // 只认"新发出的包"：额度上限为 0 或缺失的不计，避免把平台
                // 返回的占位条目算成进账。
                if let Some(total) = pack.total.filter(|n| *n > 0.0) {
                    stats.today_new_pack_credits += total;
                }
            }
        }

        // Rates over the cache-reporting subset, plus the coverage that says how
        // much of today's traffic that subset represents. Together they replace
        // the old total-hits / total-prompt figure, which mixed measured and
        // unmeasured traffic and under-reported the rate by ~4x.
        //
        // Why this pair matters more than the token totals: an agent loop resends
        // the whole conversation every turn, so `today_prompt_tokens` counts the
        // same history over and over. Measured 2026-11-08 — 2.56亿 tokens sent,
        // 98.8M of them (38.7%) served from cache, 1.58亿 actually billable. The
        // sent figure is a traffic volume, not a bill, and the UI labels it as
        // such rather than presenting it as consumption.
        if stats.today_prompt_tokens_with_cache > 0 {
            stats.today_cache_hit_rate = Some(
                (stats.today_prompt_tokens_with_cache
                    .saturating_sub(stats.today_billable_prompt_tokens_with_cache))
                    as f64
                    / stats.today_prompt_tokens_with_cache as f64,
            );
        }
        if stats.today_prompt_tokens > 0 {
            stats.today_cache_coverage =
                Some(stats.today_prompt_tokens_with_cache as f64 / stats.today_prompt_tokens as f64);
        }

        stats.claims = build_claim_report(&self.store, day_start, day_end)?;

        let mut platforms: Vec<PlatformUsage> = per_platform
            .into_iter()
            .map(|(platform, (mut usage, mut latencies))| {
                usage.finish(&mut latencies);
                PlatformUsage { platform, usage, models: Vec::new() }
            })
            .collect();
        platforms.sort_by(|a, b| b.usage.requests.cmp(&a.usage.requests));
        stats.platforms = platforms;

        if !today_latencies.is_empty() {
            let sum: u64 = today_latencies.iter().sum();
            stats.today_avg_latency_ms = Some(sum / today_latencies.len() as u64);
        }
        stats.truncated = requests.len() >= USAGE_ROW_CAP || rewards.len() >= USAGE_ROW_CAP;
        Ok(stats)
    }
}

impl CreditManager {
    /// True when the account has no confirmed claim for the current Beijing
    /// day but its live snapshot still shows a claimable reward: the daily
    /// attempt ran but upstream never confirmed it (Trae 9074 crowding).
    pub(crate) fn unclaimed_today(&self, account: &Account, bounds: &(i64, i64)) -> bool {
        let Some(snapshot) = &account.snapshot else {
            return false;
        };
        let claimable = snapshot
            .rewards
            .iter()
            .any(|reward| reward.status == "claimable");
        if !claimable {
            return false;
        }
        // Stop retrying late in the evening: a claim confirmed after the
        // snapshot refreshed may race the reward list; cap retries at 23:00.
        let hour = (now_ms() / 1000 + 8 * 3600).rem_euclid(86_400) / 3600;
        if hour >= 23 {
            return false;
        }
        // Did this account get any non-pending claim attempt today?
        match self.store.rewards_for_account_today(account.id.as_str(), *bounds) {
            Ok(outcomes) => {
                // Retrying cannot fix these two, so do not burn requests on them:
                // - a confirmed claim (already credited today),
                // - 9095 device quota already used / 9074 device id rejected.
                let settled = outcomes.iter().any(|outcome| {
                    matches!(
                        outcome.status.as_str(),
                        "claimed" | "already_claimed" | "success"
                    ) || outcome.message.contains("设备")
                });
                !settled
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn saved_snapshot_account(id: &str) -> Account {
        Account {
            id: id.into(),
            platform: Platform::Workbuddy,
            region: CreditRegion::Cn,
            user_id: "test-user".into(),
            label: "test".into(),
            enabled: true,
            status: "ready".into(),
            last_error: None,
            snapshot: Some(ProviderSnapshot {
                balance: Some(123.0),
                unit: "credits".into(),
                quota: None,
                packs: vec![],
                rewards: vec![],
                models: vec![],
                fetched_at: 1,
                stale: false,
                diagnostic: None,
                sync: SnapshotSync::default(),
            }),
        }
    }

    #[tokio::test]
    async fn refresh_local_credential_failures_preserve_snapshot_as_error() {
        for (raw, expected) in [
            (None, "账号凭据不存在，请重新授权"),
            (Some("invalid-json"), "账号凭据格式无效，请重新授权"),
        ] {
            let manager = CreditManager::test();
            let account = saved_snapshot_account("local-credential-error");
            manager.store.save_account(&account).unwrap();
            if let Some(raw) = raw {
                manager.secrets.set(&account.id, raw).unwrap();
            }
            let updated = manager.refresh_account(&account.id).await.unwrap();
            assert_eq!(updated.status, "error");
            assert_eq!(updated.last_error.as_deref(), Some(expected));
            assert!(manager.refresh_schedule.lock().due(&updated, now_ms() + 86_400_000, true).is_empty());
            assert!(manager.provider.test_inspection_calls().is_empty());
            let snapshot = updated.snapshot.unwrap();
            assert!(snapshot.stale);
            assert_eq!(snapshot.balance, Some(123.0));
            assert_eq!(snapshot.fetched_at, 1);
            assert_eq!(manager.store.account(&account.id).unwrap().status, "error");
        }
    }

    #[tokio::test]
    async fn refresh_expired_credential_without_refresh_token_is_expired() {
        let manager = CreditManager::test();
        let account = saved_snapshot_account("expired-credential");
        manager.store.save_account(&account).unwrap();
        manager
            .save_credentials(
                &account.id,
                &Credentials {
            region: CreditRegion::Cn,
                    access_token: "synthetic-expired-token".into(),
                    refresh_token: None,
                    user_id: None,
                    device_id: None,
                    expires_at: Some(now_ms() - 1),
                    extra: serde_json::Value::Null,
                },
            )
            .unwrap();
        // WorkBuddy rejects a missing refresh token before building any request.
        let updated = manager.refresh_account(&account.id).await.unwrap();
        assert_eq!(updated.status, "expired");
        assert_eq!(
            updated.last_error.as_deref(),
            Some("此凭证没有刷新令牌，请重新登录")
        );
        let snapshot = updated.snapshot.unwrap();
        assert!(snapshot.stale);
        assert_eq!(snapshot.balance, Some(123.0));
        assert_eq!(snapshot.fetched_at, 1);
        assert_eq!(
            manager.store.account(&account.id).unwrap().status,
            "expired"
        );
    }

    #[tokio::test]
    #[ignore = "uses a saved WorkBuddy account for one short capped inference; requires explicit opt-in"]
    async fn saved_workbuddy_messages_smoke() {
        assert_eq!(
            std::env::var("AI_TOOLKIT_PROXY_DIAGNOSTIC").as_deref(),
            Ok("real-inference")
        );
        let path = dirs::home_dir().unwrap().join(".ai-toolkit/ai-toolkit.db");
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let (metadata, snapshot): (String, String) = connection.query_row(
            "SELECT a.metadata,s.snapshot FROM credit_accounts a JOIN credit_snapshots s ON s.account_id=a.id WHERE a.platform='workbuddy' LIMIT 1",
            [], |row| Ok((row.get(0)?, row.get(1)?))
        ).expect("saved WorkBuddy account available");
        let mut account: Account = serde_json::from_str(&metadata).unwrap();
        account.snapshot = Some(serde_json::from_str(&snapshot).unwrap());
        assert!(account.enabled);
        assert!(account
            .snapshot
            .as_ref()
            .unwrap()
            .models
            .iter()
            .any(|model| model.id == "glm-5.3-flash"));
        let saved = secrets::SecretStore::new()
            .get(&account.id)
            .unwrap()
            .expect("saved system credential available");
        let credentials: Credentials = serde_json::from_str(&saved).unwrap();
        assert!(
            credentials
                .expires_at
                .is_none_or(|expiry| expiry > now_ms() + 60_000),
            "saved credential expired; do not refresh or alter the user's login for this test"
        );
        // All gateway state, keys and request history are isolated in memory.
        // A stale UI access error does not prevent this isolated read/inference
        // diagnostic after the system credential has actually been retrieved.
        account.status = "ready".into();
        account.last_error = None;
        let manager = CreditManager::test();
        manager.store.save_account(&account).unwrap();
        manager.secrets.set(&account.id, &saved).unwrap();
        let status = manager.gateway.start(manager.clone(), 0).await.unwrap();
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let response = http.post(format!("{}/messages", status.base_url))
            .header("x-api-key", manager.gateway_key().unwrap())
            .header("anthropic-version", "2023-06-01")
            .json(&serde_json::json!({"model":"workbuddy/glm-5.3-flash","max_tokens":128,"stream":false,
                "messages":[{"role":"user","content":"Reply exactly OK."}]})).send().await.unwrap();
        let status = response.status();
        let result: serde_json::Value = response.json().await.unwrap();
        manager.gateway.stop().await.unwrap();
        assert!(
            status.is_success(),
            "gateway response: {}",
            result.get("error").unwrap_or(&serde_json::Value::Null)
        );
        assert_eq!(result["type"], "message");
        assert_eq!(result["model"], "workbuddy/glm-5.3-flash");
        assert!(result["content"].as_array().is_some_and(|blocks| blocks
            .iter()
            .any(|block| block["text"].as_str().is_some_and(|text| !text.is_empty()))));
        println!("WorkBuddy Messages inference verified: HTTP {}; model=workbuddy/glm-5.3-flash; stop_reason={}", status.as_u16(), result["stop_reason"]);
    }

    #[test]
    fn scheduler_uses_beijing_ten_thirty() {
        assert_eq!(claim_schedule(3_600_000), (0, false));
        assert_eq!(claim_schedule(8_999_999), (0, false));
        assert_eq!(claim_schedule(9_000_000), (0, true));
        assert_eq!(claim_schedule(57_599_999), (0, true));
        assert_eq!(claim_schedule(57_600_000), (1, false));
        assert_eq!(claim_schedule(95_399_999), (1, false));
        assert_eq!(claim_schedule(95_400_000), (1, true));
    }
    #[test]
    fn gateway_keys_persist_and_do_not_enter_dashboard() {
        let manager = CreditManager::test();
        manager.retry_gateway_key_access().unwrap();
        let key = manager.gateway_key().unwrap();
        assert_eq!(manager.gateway_key().unwrap(), key);
        assert!(!serde_json::to_string(&manager.dashboard().unwrap())
            .unwrap()
            .contains(&key));
        assert!(manager.accounts().unwrap().is_empty());
    }
    #[tokio::test]
    async fn database_delete_failure_restores_credentials() {
        let manager = CreditManager::test();
        let account = Account {
            id: "restore-test".into(),
            platform: Platform::Qoder,
            region: CreditRegion::Cn,
            user_id: "test".into(),
            label: "test".into(),
            enabled: true,
            status: "ready".into(),
            last_error: None,
            snapshot: None,
        };
        manager.store.save_account(&account).unwrap();
        manager.secrets.set(&account.id, "private-token").unwrap();
        manager.store.conn.lock().execute_batch("CREATE TRIGGER reject_delete BEFORE DELETE ON credit_accounts BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
        assert!(manager.delete_account(&account.id).await.is_err());
        assert_eq!(
            manager.secrets.get(&account.id).unwrap().as_deref(),
            Some("private-token")
        );
        assert!(manager.store.account(&account.id).is_ok());
    }
}
