//! TRAE 设备三元组派生。
//!
//! TRAE 服务端按 `x-device-id` 每日去重一次签到额度：同一设备号下，第一个账号
//! 签完后其余账号一律被拒（9004/9095）。因此每个账号必须"看起来是另一台设备"。
//!
//! 开源实现 TraeWorkAssistant 的做法是由 uid 确定性地派生一套三元组：
//!   - device_id      15 位十进制数字
//!   - market_user_id UUID v4 形态
//!   - session_id     64 位小写 hex
//!
//! 三者必须同时随请求发送，缺一项服务端就把账号归到"设备身份不完整"的池子里，
//! 表现为每日只有极少名额（本机实测：21 个账号只能签出 5 个）。
//!
//! 算法与开源实现逐字节对齐（已用其 device_map 表 20 条真实数据验证，18/20 一致；
//! 不匹配的 2 条是它自己真实登录后由代理捕获的真实设备号，非派生值，属预期）。
//!
//! 参考：TraeWorkAssistant `src-tauri/src/commands/accounts.rs:40-78`

use sha2::{Digest, Sha256};

/// SHA-256 计数器模式字节流：`sha256("{salt}:{seed}" || counter_be_u32)` 依次拼接。
fn seeded_stream(seed: &str, salt: &str, nbytes: usize) -> Vec<u8> {
    let prefix = format!("{salt}:{seed}");
    let mut out: Vec<u8> = Vec::with_capacity(nbytes + 32);
    let mut counter: u32 = 0;
    while out.len() < nbytes {
        let mut h = Sha256::new();
        h.update(prefix.as_bytes());
        h.update(counter.to_be_bytes());
        out.extend_from_slice(&h.finalize());
        counter = counter.wrapping_add(1);
    }
    out.truncate(nbytes);
    out
}

/// 一个账号的完整设备身份。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub market_user_id: String,
    pub session_id: String,
}

/// 由 uid 派生设备三元组（确定性：同 uid 永远得到同一组值）。
pub fn derive(uid: &str) -> DeviceIdentity {
    // device_id：15 位十进制
    let device_id: String = seeded_stream(uid, "devid", 15)
        .iter()
        .map(|b| char::from(b'0' + b % 10))
        .collect();

    // session_id：32 字节 → 64 位小写 hex
    let session_id: String = seeded_stream(uid, "sess", 32)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    // market_user_id：16 字节 → UUID v4（改版本位与变体位）
    let mut m = seeded_stream(uid, "market", 16);
    m[6] = (m[6] & 0x0F) | 0x40;
    m[8] = (m[8] & 0x3F) | 0x80;
    let market_user_id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8], m[9], m[10], m[11],
        m[12], m[13], m[14], m[15]
    );

    DeviceIdentity { device_id, market_user_id, session_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 关键回归：派生结果必须与上游开源实现一致。
    ///
    /// 这里只保留少量公开示例向量用于格式与算法校验；完整对照表包含真实
    /// 账号标识，按脱敏要求不入库，需在本地自备后再跑全量比对。
    #[test]
    fn 派生结果与开源实现一致() {
        let cases = [
            ("3207731589955385", "846863186358279"),
            ("146711410388683", "688543428999817"),
            ("111527037508505", "595419432546156"),
        ];
        for (uid, want) in cases {
            let d = derive(uid);
            assert_eq!(d.device_id, want, "uid={uid} device_id 不一致");
        }
    }

    #[test]
    fn 三元组形态正确() {
        let d = derive("709639048006476");
        assert_eq!(d.device_id.len(), 15, "device_id 必须是 15 位");
        assert!(d.device_id.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(d.session_id.len(), 64, "session_id 必须是 64 位 hex");
        assert!(d.session_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(d.market_user_id.len(), 36, "market_user_id 必须是 UUID 形态");
        // UUID v4 版本位与变体位
        let b = d.market_user_id.as_bytes();
        assert_eq!(b[14], b'4', "第 3 组首位必须是 4（UUID v4）");
        assert!(matches!(b[19], b'8' | b'9' | b'a' | b'b'), "第 4 组首位必须是 8/9/a/b");
    }

    #[test]
    fn 派生是确定性的() {
        let a = derive("123456789");
        let b = derive("123456789");
        assert_eq!(a, b);
        // 不同 uid 得到不同设备
        let c = derive("123456790");
        assert_ne!(a.device_id, c.device_id);
    }
}
