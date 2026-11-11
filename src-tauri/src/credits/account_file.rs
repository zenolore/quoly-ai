//! 批量导入买来的账号文件。
//!
//! 卖家导出的结构有几种变体，这里统一解析成 `Credentials`：
//!
//!   A. 顶层数组，每项 `auth_raw.auth.accessToken`（单号导出）
//!   B. 顶层对象 `{format:"workbuddy-accounts", accounts:[...]}`，每项
//!      `accessToken` / `uid` 直接躺在顶层（整包导出）
//!   C. 单个对象，同 A
//!   D. 裸凭据，就是 AiToolkit 自己的 `Credentials` JSON 形状
//!
//! 解析完先做静态校验（token 形如 JWT、uid 存在、有效期未过），把能看出来的
//! 问题在**写库之前**报出来。真正的有效性交给 `add_account`——它会调一次
//! 平台接口拿 userId，失败就不落库。

use serde::Serialize;
use serde_json::{json, Value};

use super::types::{Credentials, CreditRegion};

/// 一个待导入的账号。`label_suffix` 让调用方决定是否加「（购买）」这类标记。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedAccount {
    pub nickname: String,
    pub user_id: String,
    pub access_token: String,
    #[serde(skip)]
    pub credentials: Credentials,
    /// 静态检查发现的问题，非空就不该导入。
    pub problems: Vec<String>,
    /// 已经在库里的账号 id（按 userId 匹配），用于提示会覆盖。
    pub existing_id: Option<String>,
}

/// 手写 Debug 而不派生：`Credentials` 里有 token，直接派生会在任何一次
/// `{:?}` 里把凭据打进日志。这里只输出长度和 userId。
impl std::fmt::Debug for ParsedAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedAccount")
            .field("nickname", &self.nickname)
            .field("user_id", &self.user_id)
            .field("token_len", &self.access_token.len())
            .field("problems", &self.problems)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseReport {
    pub format: String,
    pub accounts: Vec<ParsedAccount>,
    pub duplicates: usize,
    pub skipped: Vec<String>,
}

/// 判断是不是一段 JWT（三段点分）。只做形状检查，不验签。
fn looks_like_jwt(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() > 4)
}

/// 从 JWT 的 payload 里读 exp（秒）。读不到就返回 None，不算错误。
fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    // base64url 无填充，补上再解码
    let mut s = payload.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    let raw = base64_decode(&s)?;
    let value: Value = serde_json::from_slice(&raw).ok()?;
    value.get("exp").and_then(Value::as_i64)
}

/// 极简 base64 解码，避免为这一处引入依赖。
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let v = lookup[byte as usize];
        if v == 255 {
            return None;
        }
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// 从任意一层里取出 accessToken / refreshToken / userId 这些字段。
/// 卖家导出的字段名不完全统一，这里把见过的写法都覆盖上。
fn pick<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(v) = obj.get(*key).and_then(Value::as_str) {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn parse_one(item: &Value) -> Option<ParsedAccount> {
    // token 可能在顶层，也可能在 auth_raw.auth 里
    let auth = item
        .get("auth_raw")
        .and_then(|v| v.get("auth"))
        .unwrap_or(item);

    let access = pick(auth, &["accessToken", "access_token"])
        .or_else(|| pick(item, &["accessToken", "access_token"]))?;
    let refresh = pick(auth, &["refreshToken", "refresh_token"])
        .or_else(|| pick(item, &["refreshToken", "refresh_token"]))
        .map(str::to_owned);
    let uid = pick(item.get("auth_raw").unwrap_or(item), &["uid", "userId", "user_id"])
        .or_else(|| pick(item, &["uid", "userId", "user_id"]))
        .or_else(|| pick(auth, &["uid", "userId", "user_id"]));

    let nickname = pick(item, &["nickname", "label", "name"])
        .map(str::to_owned)
        .or_else(|| uid.map(|u| u.chars().take(8).collect()))
        .unwrap_or_else(|| "未命名".into());

    let device = pick(item, &["deviceId", "device_id"]).map(str::to_owned);

    let mut problems = Vec::new();
    if !looks_like_jwt(access) {
        problems.push("accessToken 不是有效的 JWT 形状".to_string());
    }
    if let Some(exp) = jwt_expiry(access) {
        let now = chrono::Utc::now().timestamp();
        if exp <= now {
            problems.push("accessToken 已过期".to_string());
        }
    }
    if refresh.is_none() {
        problems.push("缺少 refreshToken，过期后无法续期".to_string());
    }
    let uid = match uid {
        Some(u) if !u.is_empty() => u.to_owned(),
        _ => {
            problems.push("缺少 uid / userId".to_string());
            String::new()
        }
    };

    Some(ParsedAccount {
        nickname,
        user_id: uid,
        access_token: access.to_owned(),
        credentials: Credentials {
            region: CreditRegion::Cn,
            access_token: access.to_owned(),
            refresh_token: refresh,
            user_id: None, // 交给 add_account 从平台取，避免和真实 userId 不一致
            device_id: device,
            expires_at: jwt_expiry(access).map(|s| s * 1000),
            extra: json!({}),
        },
        problems,
        existing_id: None,
    })
}

/// 解析文件内容。`existing` 是库里已有的 userId → 账号 id 映射。
pub fn parse(content: &str, existing: &std::collections::HashMap<String, String>) -> Result<ParseReport, String> {
    let root: Value = serde_json::from_str(content).map_err(|e| format!("不是有效的 JSON：{e}"))?;

    let (items, format): (Vec<&Value>, &str) = if let Some(list) = root.as_array() {
        (list.iter().collect(), "数组（单号导出）")
    } else if let Some(list) = root.get("accounts").and_then(Value::as_array) {
        (list.iter().collect(), "整包（accounts 数组）")
    } else if root.is_object() {
        (vec![&root], "单个对象")
    } else {
        return Err("无法识别的 JSON 结构".into());
    };

    let mut report = ParseReport {
        format: format.into(),
        ..Default::default()
    };
    let mut seen = std::collections::HashSet::new();

    for item in items {
        let Some(mut account) = parse_one(item) else {
            report.skipped.push("缺少 accessToken".into());
            continue;
        };
        // 同一个号在文件里出现多次，只留第一份
        if !account.user_id.is_empty() && !seen.insert(account.user_id.clone()) {
            report.duplicates += 1;
            continue;
        }
        account.existing_id = existing.get(&account.user_id).cloned();
        report.accounts.push(account);
    }

    if report.accounts.is_empty() {
        return Err("文件里没有可导入的账号".into());
    }
    Ok(report)
}

/// 给 label 加上标记后缀。已有的不重复加。
pub fn tagged_label(nickname: &str, suffix: &str) -> String {
    if suffix.is_empty() || nickname.ends_with(suffix) {
        nickname.to_owned()
    } else {
        format!("{nickname}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 造一个指定过期时间的 JWT（签名段是假的，这里只验形状和 exp）。
    fn jwt(exp_offset_secs: i64) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let exp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + exp_offset_secs;
        let payload = format!("{{\"exp\":{exp}}}");
        // 头部用固定串即可，不需要真的合法——这里只验形状和 exp。
        format!("eyJhbGciOiJSUzI1NiJ9.{}.c2lnbmF0dXJl", base64_url(&payload))
    }

    fn base64_url(input: &str) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = input.as_bytes();
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            if chunk.len() > 1 {
                out.push(T[(n >> 6) as usize & 63] as char);
            }
            if chunk.len() > 2 {
                out.push(T[n as usize & 63] as char);
            }
        }
        out
    }

    #[test]
    fn accepts_wrapped_accounts_format() {
        // 整包导出：accounts 数组，token 在顶层
        let content = format!(
            r#"{{"format":"workbuddy-accounts","accounts":[
                {{"nickname":"a1","accessToken":"{}","refreshToken":"r","uid":"u1"}},
                {{"nickname":"a2","accessToken":"{}","refreshToken":"r","uid":"u2"}}
            ]}}"#,
            jwt(86400),
            jwt(86400)
        );
        let report = parse(&content, &HashMap::new()).unwrap();
        assert_eq!(report.accounts.len(), 2);
        assert_eq!(report.format, "整包（accounts 数组）");
        assert!(report.accounts.iter().all(|a| a.problems.is_empty()));
    }

    #[test]
    fn accepts_array_with_nested_auth() {
        // 单号导出：数组 + auth_raw.auth
        let content = format!(
            r#"[{{"nickname":"b1","auth_raw":{{"uid":"u9","auth":{{"accessToken":"{}","refreshToken":"r"}}}}}}]"#,
            jwt(86400)
        );
        let report = parse(&content, &HashMap::new()).unwrap();
        assert_eq!(report.accounts.len(), 1);
        assert_eq!(report.accounts[0].user_id, "u9");
    }

    #[test]
    fn flags_expired_missing_uid_and_bad_token() {
        let content = format!(
            r#"[
                {{"nickname":"expired","accessToken":"{}","uid":"u1"}},
                {{"nickname":"nouid","accessToken":"{}"}},
                {{"nickname":"bad","accessToken":"nope","uid":"u3"}},
                {{"nickname":"good","accessToken":"{}","refreshToken":"r","uid":"u4"}}
            ]"#,
            jwt(-3600),
            jwt(86400),
            jwt(86400)
        );
        let report = parse(&content, &HashMap::new()).unwrap();
        assert_eq!(report.accounts.len(), 4);

        let expired = report.accounts.iter().find(|a| a.nickname == "expired").unwrap();
        assert!(expired.problems.iter().any(|p| p.contains("已过期")));

        let nouid = report.accounts.iter().find(|a| a.nickname == "nouid").unwrap();
        assert!(nouid.problems.iter().any(|p| p.contains("uid")));

        let bad = report.accounts.iter().find(|a| a.nickname == "bad").unwrap();
        assert!(bad.problems.iter().any(|p| p.contains("JWT")));

        let good = report.accounts.iter().find(|a| a.nickname == "good").unwrap();
        assert!(good.problems.is_empty(), "{:?}", good.problems);
    }

    #[test]
    fn dedupes_same_user_within_one_file() {
        let content = format!(
            r#"[{{"nickname":"x","accessToken":"{}","refreshToken":"r","uid":"same"}},
                {{"nickname":"x2","accessToken":"{}","refreshToken":"r","uid":"same"}}]"#,
            jwt(86400),
            jwt(86400)
        );
        let report = parse(&content, &HashMap::new()).unwrap();
        assert_eq!(report.accounts.len(), 1);
        assert_eq!(report.duplicates, 1);
    }

    #[test]
    fn marks_existing_accounts() {
        let mut existing = HashMap::new();
        existing.insert("known".to_string(), "abc-123".to_string());
        let content = format!(
            r#"[{{"nickname":"k","accessToken":"{}","refreshToken":"r","uid":"known"}}]"#,
            jwt(86400)
        );
        let report = parse(&content, &existing).unwrap();
        assert_eq!(report.accounts[0].existing_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn rejects_non_json_and_empty() {
        assert!(parse("not json", &HashMap::new()).is_err());
        assert!(parse("[]", &HashMap::new()).is_err());
        assert!(parse(r#"{"foo":1}"#, &HashMap::new()).is_err());
    }

    #[test]
    fn label_suffix_is_idempotent() {
        assert_eq!(tagged_label("abc", "（购买）"), "abc（购买）");
        assert_eq!(tagged_label("abc（购买）", "（购买）"), "abc（购买）");
        assert_eq!(tagged_label("abc", ""), "abc");
    }
}
