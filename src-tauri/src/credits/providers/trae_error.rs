//! TRAE 签到错误分类与冷却。
//!
//! 与开源实现 TraeWorkAssistant `tasks/trae_checkin.rs::classify_error` 逐分支对齐：
//!
//! | error_type     | 触发条件                              | 冷却        |
//! |----------------|---------------------------------------|-------------|
//! | PlanLimit      | HTTP 200 + code 1005                  | 12 小时     |
//! | SoftRate       | HTTP 429                              | 60 秒       |
//! | SessionDead    | HTTP 401                              | 永久        |
//! | AuthRejected   | message 含 "not able to authenticate"  | 永久        |
//! | DeviceSigned   | message 含 "今日已经签到"/"设备已签到" | 至次日 00:05 |
//! | NotFound       | HTTP 404                              | 60 秒       |
//! | Server         | HTTP 5xx                              | 600 秒（前 2 次仅计数） |
//! | Client         | HTTP 4xx                              | 600 秒（前 2 次仅计数） |
//! | BusinessError  | 其他非 0 业务码                        | 300 秒      |
//! | Unknown        | 传输层异常（status=0）                 | 不冷却      |
//!
//! 为什么要分类：不分类就只会"失败了立刻重试"，而 SessionDead / AuthRejected
//! 重试一万次也不会好，反而把账号的风控评分刷坏；DeviceSigned 在当日 00:00
//! 之前无论怎么试都不会成功。冷却表让调度跳过这些账号，把请求留给真正可签的。

use std::collections::HashMap;

/// 签到失败类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorType {
    PlanLimit,
    SoftRate,
    SessionDead,
    AuthRejected,
    DeviceSigned,
    NotFound,
    Server,
    Client,
    BusinessError,
    Unknown,
}

impl ErrorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlanLimit => "PlanLimit",
            Self::SoftRate => "SoftRate",
            Self::SessionDead => "SessionDead",
            Self::AuthRejected => "AuthRejected",
            Self::DeviceSigned => "DeviceSigned",
            Self::NotFound => "NotFound",
            Self::Server => "Server",
            Self::Client => "Client",
            Self::BusinessError => "BusinessError",
            Self::Unknown => "Unknown",
        }
    }

    /// 是否需要写入冷却表。Unknown 只记录不冷却。
    pub fn worth_cooldown(self) -> bool {
        self != Self::Unknown
    }
}

/// 判定「设备当日已签」：服务端按设备每日只计一次，换设备号也可能无效。
///
/// 覆盖三种真实文案：
///   - `今日已经签到`
///   - `设备已签到`
///   - `设备今日签到名额已被占用（9004/9095）` ← 本机实测最常见
///   - `already checked in`
pub fn is_device_signed(message: &str) -> bool {
    message.contains("今日已经签到")
        || message.contains("设备已签到")
        || message.contains("名额已被占用")
        || message.contains("already checked in")
}

/// 判定「服务端拒绝认证」：凭证级失效，重试与短冷却都无法自愈。
pub fn is_auth_rejected(message: &str) -> bool {
    message.to_lowercase().contains("not able to authenticate you")
}

/// 主分类函数：按 HTTP 状态码与业务码归类。
pub fn classify(http_status: u16, code: Option<i64>) -> ErrorType {
    if http_status == 200 && code == Some(1005) {
        return ErrorType::PlanLimit;
    }
    if http_status == 429 {
        return ErrorType::SoftRate;
    }
    if http_status == 401 {
        return ErrorType::SessionDead;
    }
    if http_status == 404 {
        return ErrorType::NotFound;
    }
    if (500..600).contains(&http_status) {
        return ErrorType::Server;
    }
    if (400..500).contains(&http_status) {
        return ErrorType::Client;
    }
    if code.is_some_and(|c| c != 0) {
        return ErrorType::BusinessError;
    }
    ErrorType::Unknown
}

/// 综合分类：先看业务文案（更能说明真实原因），再看状态码。
pub fn classify_with_message(http_status: u16, code: Option<i64>, message: &str) -> ErrorType {
    if is_auth_rejected(message) {
        return ErrorType::AuthRejected;
    }
    if is_device_signed(message) {
        return ErrorType::DeviceSigned;
    }
    classify(http_status, code)
}

/// 冷却秒数。`-1` 表示永久，`0` 表示不冷却。
pub fn cooldown_secs(kind: ErrorType) -> i64 {
    match kind {
        ErrorType::PlanLimit => 12 * 3600,
        ErrorType::SoftRate => 60,
        ErrorType::NotFound => 60,
        ErrorType::SessionDead | ErrorType::AuthRejected => -1,
        // DeviceSigned 由调用方换成"到次日 00:05"，此处给 12 小时兜底
        ErrorType::DeviceSigned => 12 * 3600,
        ErrorType::Server | ErrorType::Client => 600,
        ErrorType::BusinessError => 300,
        ErrorType::Unknown => 0,
    }
}

/// 单账号冷却状态。
#[derive(Clone, Debug, Default)]
pub struct Cooldown {
    pub error_type: String,
    /// Unix 秒；`0` 表示仅计数不冷却，`9_999_999_999` 表示永久。
    pub until: i64,
    pub error_count: u32,
    pub reason: String,
}

/// 内存冷却表（调用方负责持久化）。
#[derive(Clone, Debug, Default)]
pub struct Cooldowns {
    entries: HashMap<String, Cooldown>,
}

impl Cooldowns {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_map(entries: HashMap<String, Cooldown>) -> Self {
        Self { entries }
    }

    pub fn as_map(&self) -> &HashMap<String, Cooldown> {
        &self.entries
    }

    /// 该账号当前是否处于冷却中。返回 `(是否冷却, 错误类型)`。
    pub fn blocked(&self, uid: &str, now: i64) -> (bool, &str) {
        match self.entries.get(uid) {
            None => (false, ""),
            Some(e) => {
                if e.until == 0 {
                    return (false, "");
                }
                let active = e.until >= 9_000_000_000 || e.until > now;
                (active, e.error_type.as_str())
            }
        }
    }

    /// 记录一次失败。`Server` / `Client` 前 2 次只计数，第 3 次起才真正冷却。
    pub fn record(&mut self, uid: &str, kind: ErrorType, secs: i64, reason: &str, now: i64) {
        if !kind.worth_cooldown() || secs == 0 {
            self.entries.remove(uid);
            return;
        }
        let prev_count = self.entries.get(uid).map(|e| e.error_count).unwrap_or(0);
        let entry = if secs < 0 {
            Cooldown {
                error_type: kind.as_str().into(),
                until: 9_999_999_999,
                error_count: 0,
                reason: reason.into(),
            }
        } else if matches!(kind, ErrorType::Server | ErrorType::Client) {
            let count = prev_count + 1;
            if count < 3 {
                Cooldown {
                    error_type: kind.as_str().into(),
                    until: 0,
                    error_count: count,
                    reason: reason.into(),
                }
            } else {
                Cooldown {
                    error_type: kind.as_str().into(),
                    until: now + secs,
                    error_count: 0,
                    reason: reason.into(),
                }
            }
        } else {
            Cooldown {
                error_type: kind.as_str().into(),
                until: now + secs,
                error_count: 0,
                reason: reason.into(),
            }
        };
        self.entries.insert(uid.into(), entry);
    }

    /// 成功后清除该账号冷却。
    pub fn clear(&mut self, uid: &str) {
        self.entries.remove(uid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 分类与开源实现一致() {
        assert_eq!(classify(200, Some(1005)), ErrorType::PlanLimit);
        assert_eq!(classify(429, None), ErrorType::SoftRate);
        assert_eq!(classify(401, None), ErrorType::SessionDead);
        assert_eq!(classify(404, None), ErrorType::NotFound);
        assert_eq!(classify(502, None), ErrorType::Server);
        assert_eq!(classify(403, None), ErrorType::Client);
        assert_eq!(classify(200, Some(7)), ErrorType::BusinessError);
        assert_eq!(classify(0, None), ErrorType::Unknown);
    }

    #[test]
    fn 业务文案优先于状态码() {
        assert_eq!(
            classify_with_message(200, Some(0), "not able to authenticate you"),
            ErrorType::AuthRejected
        );
        assert_eq!(
            classify_with_message(200, Some(9004), "设备今日签到名额已被占用"),
            ErrorType::DeviceSigned
        );
        assert_eq!(
            classify_with_message(200, Some(0), "今日已经签到"),
            ErrorType::DeviceSigned
        );
    }

    #[test]
    fn 冷却时长与开源实现一致() {
        assert_eq!(cooldown_secs(ErrorType::PlanLimit), 12 * 3600);
        assert_eq!(cooldown_secs(ErrorType::SoftRate), 60);
        assert_eq!(cooldown_secs(ErrorType::SessionDead), -1);
        assert_eq!(cooldown_secs(ErrorType::AuthRejected), -1);
        assert_eq!(cooldown_secs(ErrorType::Server), 600);
        assert_eq!(cooldown_secs(ErrorType::BusinessError), 300);
        assert_eq!(cooldown_secs(ErrorType::Unknown), 0);
    }

    #[test]
    fn server_错误前两次只计数第三次才冷却() {
        let mut cd = Cooldowns::new();
        let now = 1_000_000;
        cd.record("u1", ErrorType::Server, 600, "x", now);
        assert!(!cd.blocked("u1", now).0, "第 1 次不应冷却");
        cd.record("u1", ErrorType::Server, 600, "x", now);
        assert!(!cd.blocked("u1", now).0, "第 2 次不应冷却");
        cd.record("u1", ErrorType::Server, 600, "x", now);
        assert!(cd.blocked("u1", now).0, "第 3 次应冷却");
        assert!(!cd.blocked("u1", now + 601).0, "过期后解除");
    }

    #[test]
    fn 永久冷却不会过期() {
        let mut cd = Cooldowns::new();
        cd.record("u2", ErrorType::SessionDead, -1, "dead", 1_000_000);
        assert!(cd.blocked("u2", 1_000_000).0);
        assert!(cd.blocked("u2", i64::MAX - 1).0, "永久冷却永不过期");
        cd.clear("u2");
        assert!(!cd.blocked("u2", 1_000_000).0, "清除后解除");
    }

    #[test]
    fn unknown_不写冷却() {
        let mut cd = Cooldowns::new();
        cd.record("u3", ErrorType::Unknown, 0, "net", 1_000_000);
        assert!(!cd.blocked("u3", 1_000_000).0);
        assert!(cd.as_map().is_empty());
    }
}
