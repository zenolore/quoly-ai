use super::types::*;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;

/// One aggregated row from `request_health`.
pub struct RequestHealthRow {
    /// Route as stored, e.g. `workbuddy/deepseek-v4.1-flash`.
    pub model: String,
    pub ok: u64,
    pub fail: u64,
    /// Client-aborted or timed-out requests. Reported separately because they
    /// are not evidence of the model being unhealthy.
    pub cancelled: u64,
    pub avg_latency_ms: Option<u64>,
}

#[derive(Clone)]
pub struct CreditStore {
    pub conn: Arc<Mutex<Connection>>,
}

impl CreditStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Result<Self, String> {
        let store = Self { conn };
        migrate_account_regions(&mut store.conn.lock())?;
        store
            .conn
            .lock()
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS credit_accounts (
                id TEXT PRIMARY KEY, platform TEXT NOT NULL, region TEXT NOT NULL DEFAULT 'cn',
                user_id TEXT NOT NULL, metadata TEXT NOT NULL, UNIQUE(platform, region, user_id)
            );
            CREATE TABLE IF NOT EXISTS credit_snapshots (
                account_id TEXT PRIMARY KEY, snapshot TEXT NOT NULL, updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS credit_rewards (
                id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT NOT NULL,
                outcome TEXT NOT NULL, created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS credit_proxy_settings (
                id INTEGER PRIMARY KEY CHECK(id=1), settings TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS credit_requests (
                id TEXT PRIMARY KEY, account_id TEXT NOT NULL, summary TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            -- One row per growth-center pass per account. Kept separate from
            -- credit_rewards because a growth pass reports many things (travel
            -- claimed, tasks accepted, boxes opened) that are not a single reward,
            -- and the daily report needs to distinguish a pass that ran and earned
            -- zero from a day the pass never reached.
            CREATE TABLE IF NOT EXISTS credit_growth_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                outcome TEXT NOT NULL,
                credits INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS credit_growth_runs_at
                ON credit_growth_runs(created_at);
            -- Audit trail for buddy activation. This one is kept separately and
            -- never pruned, because the operation reports a client telemetry event
            -- on the user's behalf: who was activated, when, what the server
            -- answered, and how the streak moved all need to stay reviewable.
            CREATE TABLE IF NOT EXISTS credit_activations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                outcome TEXT NOT NULL,
                detail TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS credit_activations_at
                ON credit_activations(created_at);
        ",
            )
            .map_err(db_error)?;
        Ok(store)
    }

    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        let conn = self.conn.lock();
        let mut query = conn.prepare("SELECT a.metadata,s.snapshot FROM credit_accounts a LEFT JOIN credit_snapshots s ON s.account_id=a.id ORDER BY a.platform,a.region,a.user_id").map_err(db_error)?;
        let rows = query
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(db_error)?;
        let mut accounts = Vec::new();
        for row in rows {
            let (meta, snapshot) = row.map_err(db_error)?;
            let mut account: Account = serde_json::from_str(&meta).map_err(json_error)?;
            account.snapshot = snapshot
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(json_error)?;
            accounts.push(account);
        }
        Ok(accounts)
    }

    pub fn account(&self, id: &str) -> Result<Account, String> {
        self.accounts()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| "账号不存在".into())
    }

    pub fn save_account(&self, account: &Account) -> Result<(), String> {
        let mut meta = account.clone();
        meta.snapshot = None;
        let metadata = serde_json::to_string(&meta).map_err(json_error)?;
        let mut conn = self.conn.lock();
        let tx = conn.transaction().map_err(db_error)?;
        tx.execute("INSERT INTO credit_accounts(id,platform,region,user_id,metadata) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata WHERE credit_accounts.platform=excluded.platform AND credit_accounts.region=excluded.region AND credit_accounts.user_id=excluded.user_id", params![account.id, account.platform.key(), account.region.key(), account.user_id, metadata]).map_err(db_error)?;
        if tx.changes() != 1 { return Err("不能更改已保存账号的身份或区域".into()); }
        if let Some(snapshot) = &account.snapshot {
            tx.execute("INSERT INTO credit_snapshots(account_id,snapshot,updated_at) VALUES(?1,?2,?3) ON CONFLICT(account_id) DO UPDATE SET snapshot=excluded.snapshot,updated_at=excluded.updated_at", params![account.id, serde_json::to_string(snapshot).map_err(json_error)?, snapshot.fetched_at]).map_err(db_error)?;
        }
        tx.commit().map_err(db_error)
    }

    pub fn delete_account(&self, id: &str) -> Result<(), String> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction().map_err(db_error)?;
        for table in ["credit_snapshots", "credit_rewards", "credit_requests"] {
            tx.execute(&format!("DELETE FROM {table} WHERE account_id=?1"), [id])
                .map_err(db_error)?;
        }
        tx.execute("DELETE FROM credit_accounts WHERE id=?1", [id])
            .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }

    pub fn settings(&self) -> Result<ProxySettings, String> {
        let raw: Option<String> = self
            .conn
            .lock()
            .query_row(
                "SELECT settings FROM credit_proxy_settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        raw.map(|s| serde_json::from_str(&s).map_err(json_error))
            .unwrap_or_else(|| Ok(ProxySettings::default()))
    }

    pub fn save_settings(&self, settings: &ProxySettings) -> Result<(), String> {
        if settings.port == 0 {
            return Err("端口必须在 1 到 65535 之间".into());
        }
        self.conn.lock().execute("INSERT INTO credit_proxy_settings(id,settings) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET settings=excluded.settings", [serde_json::to_string(settings).map_err(json_error)?]).map_err(db_error)?;
        Ok(())
    }

    /// Append one activation attempt to the audit trail (never pruned).
    pub fn record_activation(&self, account_id: &str, outcome: &str, detail: &serde_json::Value, at: i64) -> Result<(), String> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO credit_activations(account_id,outcome,detail,created_at) VALUES(?1,?2,?3,?4)",
                params![account_id, outcome, serde_json::to_string(detail).map_err(json_error)?, at],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Whether this account was already activated since `since_ms`.
    ///
    /// The daily ceiling lives here rather than in memory so a restart cannot
    /// cause a second event on the same day.
    pub fn activated_since(&self, account_id: &str, since_ms: i64) -> Result<bool, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare("SELECT 1 FROM credit_activations WHERE account_id=?1 AND created_at>=?2 AND outcome IN ('reported','activated') LIMIT 1")
            .map_err(db_error)?;
        let mut rows = query.query(params![account_id, since_ms]).map_err(db_error)?;
        Ok(rows.next().map_err(db_error)?.is_some())
    }

    /// Recent activations, newest first, for the UI and for reviewing a pilot.
    pub fn activations(&self, limit: usize) -> Result<Vec<(String, String, serde_json::Value, i64)>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare("SELECT account_id,outcome,detail,created_at FROM credit_activations ORDER BY created_at DESC LIMIT ?1")
            .map_err(db_error)?;
        let rows = query
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            let (account_id, outcome, detail, created_at) = row.map_err(db_error)?;
            out.push((
                account_id,
                outcome,
                serde_json::from_str(&detail).unwrap_or(serde_json::Value::Null),
                created_at,
            ));
        }
        Ok(out)
    }

    /// Persist one growth-center pass so the run survives an app restart.
    ///
    /// Storing the whole outcome (not just the credit total) is what lets the UI
    /// answer "what did it actually do yesterday" instead of only "how many
    /// credits": the steps list names each reward collected.
    pub fn record_growth(&self, account_id: &str, outcome: &serde_json::Value, credits: i64, at: i64) -> Result<(), String> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO credit_growth_runs(account_id,outcome,credits,created_at) VALUES(?1,?2,?3,?4)",
                params![account_id, serde_json::to_string(outcome).map_err(json_error)?, credits, at],
            )
            .map_err(db_error)?;
        // Keep the table bounded: a year of daily runs across a few dozen accounts
        // is small, but an unbounded table would grow without limit once the pass
        // starts running on a timer.
        let _ = self.conn.lock().execute(
            "DELETE FROM credit_growth_runs WHERE created_at < ?1",
            params![at - 90 * 24 * 3600 * 1000],
        );
        Ok(())
    }

    /// Growth runs inside [from, to), newest first.
    pub fn growth_runs(&self, from: i64, to: i64, limit: usize) -> Result<Vec<(String, serde_json::Value, i64, i64)>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare("SELECT account_id,outcome,credits,created_at FROM credit_growth_runs WHERE created_at >= ?1 AND created_at < ?2 ORDER BY created_at DESC LIMIT ?3")
            .map_err(db_error)?;
        let rows = query
            .query_map(params![from, to, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            let (account_id, outcome, credits, created_at) = row.map_err(db_error)?;
            out.push((
                account_id,
                serde_json::from_str(&outcome).unwrap_or(serde_json::Value::Null),
                credits,
                created_at,
            ));
        }
        Ok(out)
    }

    pub fn record_reward(&self, outcome: &RewardOutcome) -> Result<(), String> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO credit_rewards(account_id,outcome,created_at) VALUES(?1,?2,?3)",
                params![
                    outcome.account_id,
                    serde_json::to_string(outcome).map_err(json_error)?,
                    outcome.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn record_request(&self, summary: &RequestSummary) -> Result<(), String> {
        self.conn
            .lock()
            .execute(
                "INSERT INTO credit_requests(id,account_id,summary,created_at) VALUES(?1,?2,?3,?4)",
                params![
                    summary.id,
                    summary.account_id,
                    serde_json::to_string(summary).map_err(json_error)?,
                    summary.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn rewards(&self) -> Result<Vec<RewardOutcome>, String> {
        self.history("credit_rewards", "outcome")
    }

    /// Claim outcomes inside `[since, until]`, newest first.
    ///
    /// The dashboard list is capped at 100 rows, which is fine for *displaying*
    /// recent activity but silently wrong for *aggregating* — with 60+ accounts
    /// a single morning's check-ins overflow it, so "today's claimed credits"
    /// came out an order of magnitude too low. Reports use this instead.
    pub fn rewards_between(
        &self,
        since: Option<i64>,
        until: i64,
        limit: usize,
    ) -> Result<Vec<RewardOutcome>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare(
                "SELECT outcome FROM credit_rewards
                 WHERE created_at <= ?1 AND (?2 IS NULL OR created_at >= ?2)
                 ORDER BY created_at DESC LIMIT ?3",
            )
            .map_err(db_error)?;
        let rows = query
            .query_map(rusqlite::params![until, since, limit as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row.map_err(db_error)?).map_err(json_error)?);
        }
        Ok(out)
    }

    /// Total rows in the two history tables, so a report can tell a caller
    /// whether the row cap actually clipped the answer.
    pub fn history_counts(&self) -> Result<(u64, u64), String> {
        let conn = self.conn.lock();
        let requests: i64 = conn
            .query_row("SELECT COUNT(*) FROM credit_requests", [], |r| r.get(0))
            .map_err(db_error)?;
        let rewards: i64 = conn
            .query_row("SELECT COUNT(*) FROM credit_rewards", [], |r| r.get(0))
            .map_err(db_error)?;
        Ok((requests as u64, rewards as u64))
    }

    /// Aggregate request outcomes per model, over the whole history.
    ///
    /// The `summary` column holds JSON, so the fields are extracted with SQLite's
    /// JSON functions instead of plain column arithmetic. Grouping on `model`
    /// keeps this to one row per route however many requests exist.
    ///
    /// Routes that never once succeeded are omitted. Those are mistyped or
    /// nonexistent model names (`glm-4.6v`, `default`, `auto`, …) that the
    /// upstream rejects with HTTP 400 in ~130 ms; listing them as "0% healthy"
    /// buries the models the user actually runs under dozens of dead names.
    pub fn request_health(&self) -> Result<Vec<RequestHealthRow>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare(
                "SELECT
                     json_extract(summary,'$.model') AS model,
                     SUM(CASE WHEN json_extract(summary,'$.status')='success' THEN 1 ELSE 0 END) AS ok,
                     SUM(CASE WHEN json_extract(summary,'$.status')='error' THEN 1 ELSE 0 END),
                     SUM(CASE WHEN json_extract(summary,'$.status') IN ('cancelled','aborted','timeout') THEN 1 ELSE 0 END),
                     SUM(CASE WHEN json_extract(summary,'$.status')='success'
                              THEN IFNULL(json_extract(summary,'$.elapsedMs'),0) ELSE 0 END)
                 FROM credit_requests
                 WHERE json_extract(summary,'$.model') IS NOT NULL
                 GROUP BY model
                 HAVING ok > 0",
            )
            .map_err(db_error)?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            let (model, ok, fail, cancelled, latency_sum) = row.map_err(db_error)?;
            if model.is_empty() {
                continue;
            }
            let ok = ok.max(0) as u64;
            out.push(RequestHealthRow {
                model,
                ok,
                fail: fail.max(0) as u64,
                cancelled: cancelled.max(0) as u64,
                // Average only over successes: a failed request's duration says
                // nothing about how fast the model normally answers.
                avg_latency_ms: (ok > 0).then(|| (latency_sum.max(0) as u64) / ok),
            });
        }
        Ok(out)
    }

    /// Last successful check-in timestamp per account, keyed by account id.
    ///
    /// Used to order the daily claim fairly. TRAE caps check-in per *device*, and
    /// one machine holds one registered device id, so only one account can claim
    /// per day. Without ordering, the first account in list order would win that
    /// slot every single day and the rest would never be paid. Sorting by how
    /// long ago each account last claimed hands the slot to whoever has waited
    /// longest, so the accounts rotate day by day.
    ///
    /// Only a real payout counts. "already_claimed" is excluded on purpose: it
    /// also covers "this device has nothing for you", so counting it would let an
    /// account that never actually earned anything look freshly paid and keep
    /// losing its turn.
    pub fn last_reward_at(&self) -> Result<std::collections::HashMap<String, i64>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare(
                "SELECT account_id, MAX(created_at) FROM credit_rewards
                 WHERE outcome LIKE '%\"status\":\"success\"%'
                 GROUP BY account_id",
            )
            .map_err(db_error)?;
        let rows = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_error)?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (id, at) = row.map_err(db_error)?;
            out.insert(id, at);
        }
        Ok(out)
    }

    /// Claim outcomes recorded for one account inside the [start, end) window.
    pub fn rewards_for_account_today(
        &self,
        account_id: &str,
        bounds: (i64, i64),
    ) -> Result<Vec<RewardOutcome>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare(
                "SELECT outcome FROM credit_rewards
                 WHERE account_id=?1 AND created_at>=?2 AND created_at<?3",
            )
            .map_err(db_error)?;
        let rows = query
            .query_map(params![account_id, bounds.0, bounds.1], |r| r.get::<_, String>(0))
            .map_err(db_error)?;
        let mut outcomes = Vec::new();
        for row in rows {
            let raw = row.map_err(db_error)?;
            if let Ok(outcome) = serde_json::from_str::<RewardOutcome>(&raw) {
                outcomes.push(outcome);
            }
        }
        Ok(outcomes)
    }
    pub fn requests(&self) -> Result<Vec<RequestSummary>, String> {
        self.history("credit_requests", "summary")
    }

    /// Requests inside `[since, until]`, newest first.
    ///
    /// Separate from `requests()` because that one is capped at 100 rows for the
    /// dashboard list — far too few to aggregate usage from. `limit` bounds the
    /// work done for a single-report request; callers that need everything pass
    /// the default ceiling and narrow the window instead.
    pub fn requests_between(
        &self,
        since: Option<i64>,
        until: i64,
        limit: usize,
    ) -> Result<Vec<RequestSummary>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare(
                "SELECT summary FROM credit_requests
                 WHERE created_at <= ?1 AND (?2 IS NULL OR created_at >= ?2)
                 ORDER BY created_at DESC LIMIT ?3",
            )
            .map_err(db_error)?;
        let rows = query
            .query_map(rusqlite::params![until, since, limit as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(db_error)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row.map_err(db_error)?).map_err(json_error)?);
        }
        Ok(out)
    }

    fn history<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
        column: &str,
    ) -> Result<Vec<T>, String> {
        let conn = self.conn.lock();
        let mut query = conn
            .prepare(&format!(
                "SELECT {column} FROM {table} ORDER BY created_at DESC LIMIT 100"
            ))
            .map_err(db_error)?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?;
        rows.map(|row| serde_json::from_str(&row.map_err(db_error)?).map_err(json_error))
            .collect()
    }
}

fn migrate_account_regions(conn: &mut Connection) -> Result<(), String> {
    let has_accounts: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='credit_accounts')",
        [], |row| row.get(0),
    ).map_err(db_error)?;
    if !has_accounts { return Ok(()); }
    let has_region = conn.prepare("PRAGMA table_info(credit_accounts)").map_err(db_error)?
        .query_map([], |row| row.get::<_, String>(1)).map_err(db_error)?
        .collect::<Result<Vec<_>, _>>().map_err(db_error)?.iter().any(|name| name == "region");
    if has_region { return Ok(()); }

    // VACUUM INTO includes WAL contents; copying the database file would not.
    if let Some(db_path) = conn.path().filter(|path| !path.is_empty() && *path != ":memory:") {
        let directory = std::path::Path::new(db_path).parent()
            .ok_or_else(|| "积分数据库路径无效".to_string())?.join("backups");
        let backup = directory.join(format!("credit-regions-v1-{}.db", uuid::Uuid::new_v4()));
        crate::core::private_fs::prepare_private_file(&backup)
            .map_err(|_| "无法创建积分区域迁移备份".to_string())?;
        if let Err(error) = conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()]) {
            let _ = std::fs::remove_file(&backup);
            return Err(db_error(error));
        }
        crate::core::private_fs::secure_file(&backup)
            .map_err(|_| "无法保护积分区域迁移备份".to_string())?;
    }
    let tx = conn.transaction().map_err(db_error)?;
    tx.execute_batch("CREATE TABLE credit_accounts_regions_v1 (
        id TEXT PRIMARY KEY, platform TEXT NOT NULL, region TEXT NOT NULL DEFAULT 'cn',
        user_id TEXT NOT NULL, metadata TEXT NOT NULL, UNIQUE(platform,region,user_id)
    );
    INSERT INTO credit_accounts_regions_v1(id,platform,region,user_id,metadata)
        SELECT id,platform,'cn',user_id,metadata FROM credit_accounts;
    DROP TABLE credit_accounts;
    ALTER TABLE credit_accounts_regions_v1 RENAME TO credit_accounts;").map_err(db_error)?;
    tx.commit().map_err(db_error)
}

fn db_error(_: rusqlite::Error) -> String {
    "积分数据库操作失败".into()
}
fn json_error(_: serde_json::Error) -> String {
    "积分数据格式错误".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> CreditStore {
        CreditStore::new(Arc::new(Mutex::new(Connection::open_in_memory().unwrap()))).unwrap()
    }
    fn account() -> Account {
        Account {
            id: "one".into(),
            platform: Platform::Workbuddy,
            region: CreditRegion::Cn,
            user_id: "u1".into(),
            label: "测试".into(),
            enabled: true,
            status: "ready".into(),
            last_error: None,
            snapshot: None,
        }
    }
    #[test]
    fn migration_is_repeatable_and_does_not_touch_existing_tables() {
        let s = store();
        s.conn
            .lock()
            .execute_batch(
                "CREATE TABLE mcp_servers(id TEXT); INSERT INTO mcp_servers VALUES('existing');",
            )
            .unwrap();
        CreditStore::new(s.conn.clone()).unwrap();
        assert_eq!(
            s.conn
                .lock()
                .query_row("SELECT id FROM mcp_servers", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "existing"
        );
        assert_eq!(s.settings().unwrap().port, 17860);
        assert!(!s.settings().unwrap().auto_claim);
    }
    #[test]
    fn account_upsert_deduplication_and_delete() {
        let s = store();
        s.save_account(&account()).unwrap();
        let mut changed = account();
        changed.label = "changed".into();
        s.save_account(&changed).unwrap();
        assert_eq!(s.accounts().unwrap().len(), 1);
        assert_eq!(s.account("one").unwrap().label, "changed");
        let mut duplicate = account();
        duplicate.id = "two".into();
        assert!(s.save_account(&duplicate).is_err());
        s.delete_account("one").unwrap();
        assert!(s.accounts().unwrap().is_empty());
    }
    #[test]
    fn legacy_accounts_keep_ids_and_regions_are_independent() {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        conn.lock().execute_batch("CREATE TABLE credit_accounts(id TEXT PRIMARY KEY,platform TEXT NOT NULL,user_id TEXT NOT NULL,metadata TEXT NOT NULL,UNIQUE(platform,user_id));").unwrap();
        let mut legacy = serde_json::to_value(account()).unwrap();
        legacy.as_object_mut().unwrap().remove("region");
        conn.lock().execute("INSERT INTO credit_accounts VALUES(?1,?2,?3,?4)",
            params!["one", "workbuddy", "u1", legacy.to_string()]).unwrap();
        conn.lock().execute_batch("CREATE TABLE credit_snapshots(account_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL,updated_at INTEGER NOT NULL); CREATE TABLE credit_requests(id TEXT PRIMARY KEY,account_id TEXT NOT NULL,summary TEXT NOT NULL,created_at INTEGER NOT NULL);").unwrap();
        let old_snapshot = serde_json::json!({"balance":123.5,"unit":"credits","quota":null,"packs":[{"id":"original-pack","name":"Original","remaining":123.5,"expiresAt":null}],"rewards":[],"models":[],"fetchedAt":42,"stale":false,"diagnostic":null});
        let old_request = serde_json::json!({"id":"original-request","accountId":"one","platform":"workbuddy","model":"workbuddy/fixture","elapsedMs":10,"inputTokens":3,"outputTokens":4,"status":"success","createdAt":42});
        conn.lock().execute("INSERT INTO credit_snapshots VALUES('one',?1,42)", [old_snapshot.to_string()]).unwrap();
        conn.lock().execute("INSERT INTO credit_requests VALUES('original-request','one',?1,42)", [old_request.to_string()]).unwrap();
        let s = CreditStore::new(conn.clone()).unwrap();
        assert_eq!(s.account("one").unwrap().region, CreditRegion::Cn);
        assert_eq!(s.account("one").unwrap().snapshot.unwrap().balance, Some(123.5));
        assert_eq!(s.account("one").unwrap().snapshot.unwrap().packs[0].unit, "credits");
        assert_eq!(s.requests().unwrap()[0].id, "original-request");
        assert_eq!(s.requests().unwrap()[0].region, CreditRegion::Cn);
        let mut intl = account();
        intl.id = "international".into(); intl.region = CreditRegion::Intl;
        s.save_account(&intl).unwrap();
        assert_eq!(s.accounts().unwrap().len(), 2);
        intl.id = "duplicate-international".into();
        assert!(s.save_account(&intl).is_err());
        let repeated = CreditStore::new(conn).unwrap();
        assert_eq!(repeated.accounts().unwrap().len(), 2);
        let mut changed_identity = account(); changed_identity.region = CreditRegion::Intl;
        assert!(repeated.save_account(&changed_identity).is_err());
        assert_eq!(repeated.account("one").unwrap().region, CreditRegion::Cn);
    }

    #[test]
    fn file_migration_keeps_a_private_consistent_pre_migration_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credits.db");
        let conn = Arc::new(Mutex::new(Connection::open(&path).unwrap()));
        conn.lock().execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE credit_accounts(id TEXT PRIMARY KEY,platform TEXT NOT NULL,user_id TEXT NOT NULL,metadata TEXT NOT NULL,UNIQUE(platform,user_id)); CREATE TABLE unrelated(value TEXT); INSERT INTO unrelated VALUES('keep');").unwrap();
        conn.lock().execute("INSERT INTO credit_accounts VALUES(?1,?2,?3,?4)",
            params!["one", "workbuddy", "u1", serde_json::to_string(&account()).unwrap()]).unwrap();
        let s = CreditStore::new(conn.clone()).unwrap();
        CreditStore::new(conn).unwrap();
        let backups: Vec<_> = std::fs::read_dir(directory.path().join("backups")).unwrap().map(Result::unwrap).collect();
        assert_eq!(backups.len(), 1);
        let backup = Connection::open(backups[0].path()).unwrap();
        assert_eq!(backup.query_row("SELECT value FROM unrelated", [], |r| r.get::<_, String>(0)).unwrap(), "keep");
        assert_eq!(backup.query_row("SELECT id FROM credit_accounts", [], |r| r.get::<_, String>(0)).unwrap(), "one");
        assert!(backup.prepare("SELECT region FROM credit_accounts").is_err());
        assert_eq!(s.account("one").unwrap().id, "one");
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(backups[0].metadata().unwrap().permissions().mode() & 0o777, 0o600);
        }
    }
    #[test]
    fn settings_reject_zero_port() {
        assert!(store()
            .save_settings(&ProxySettings {
                port: 0,
                auto_claim: true,
                auto_refresh: true,
                ..Default::default()
            })
            .is_err());
    }
    #[test]
    fn failed_snapshot_write_rolls_back_metadata() {
        let s = store();
        s.save_account(&account()).unwrap();
        s.conn.lock().execute_batch("CREATE TRIGGER reject_snapshot BEFORE INSERT ON credit_snapshots BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
        let mut changed = account();
        changed.label = "partial".into();
        changed.snapshot = Some(ProviderSnapshot {
            balance: Some(12.0),
            unit: "credits".into(),
            quota: None,
            packs: vec![],
            rewards: vec![],
            models: vec![],
            fetched_at: now_ms(),
            stale: false,
            diagnostic: None,
            sync: SnapshotSync::default(),
        });
        assert!(s.save_account(&changed).is_err());
        assert_eq!(s.account("one").unwrap().label, "测试");
        assert!(s.account("one").unwrap().snapshot.is_none());
    }
}
