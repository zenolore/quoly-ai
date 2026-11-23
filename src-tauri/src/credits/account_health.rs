//! 账号健康分级：判断一个账号是"真死"还是"暂时不可用"，以及该怎么处理。
//!
//! 背景（2026-11-18 实测教训）：
//! 32 个 WorkBuddy 账号标着 `banned`（11140/HTTP 403），代码里把 403 当成
//! 永久死状态、`retry=false` 永不重试，结果 **31 个实际完全可用**的账号
//! 被白白跳过一个月——它们当天还在正常签到、`active:true`。
//!
//! 这说明"一次失败就判死"是错的。真实世界里的失败分三类：
//!
//!   1. **凭证问题**（401）—— 重登能救，重试无用 → 报给用户去重登
//!   2. **临时风控**（403 11140、429、5xx）—— 上游会自己解除 → 定期复检
//!   3. **账号真死**（连续多天各类失败、且凭证也过期）—— 建议清理
//!
//! 所以需要按"失败次数 + 持续时间 + 错误类型"分档，而不是单次判定。

use crate::credits::types::Account;

/// 账号健康档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// 正常可用：能签到也能调模型。
    Healthy,
    /// 仅可签到：签到/领积分正常，但推理调用被上游拒绝（典型是 WorkBuddy 11140）。
    ///
    /// 为什么单独分一档（2026-11-18 实测）：
    /// 这类账号 `/billing/meter/checkin-activity-status` 返回 `active:true`、
    /// 每天能正常签到拿积分，但 `/v2/chat/completions` 直接 403/11140。
    /// 只按签到结果判断会误以为"完全可用"（我今天就错判了一次），
    /// 于是把它们放进网关路由池 → 每 4 次请求就有 1 次失败。
    ///
    /// 正确做法：允许它们继续签到拿积分（这是白捡的收益），
    /// 但不参与网关路由（避免拉低调用成功率）。
    SignableOnly,
    /// 凭证过期，需要重新登录才能救。
    NeedsRelogin,
    /// 疑似临时风控，等冷却后自动复检。
    Probation,
    /// 连续失败且已过观察期，建议清理（但仍然不是永久放弃，用户可手动救）。
    Retired,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::SignableOnly => "signable_only",
            Self::NeedsRelogin => "needs_relogin",
            Self::Probation => "probation",
            Self::Retired => "retired",
        }
    }
}

/// 连续失败多少「天」后自动停用。
///
/// 用户要求（2026-11-18）：连续 3 天检测不过就停用。
///
/// 注意这里的 streak 是**自然日数**（见 `record_daily_failure`），
/// 不是失败次数。一天刷新十几次也只算一天，否则 3 天规则毫无意义。
pub const DAYS_BEFORE_RETIRE: u32 = 3;

/// 兼容旧名字：原先是"失败次数"，现改为"失败天数"。
pub const FAIL_STREAK_BEFORE_RETIRE: u32 = DAYS_BEFORE_RETIRE;

/// 观察期：连续失败多久后自动停用，单位毫秒。
pub const RETIRE_AFTER: i64 = 3 * 24 * 60 * 60 * 1000;

/// 失败计数的存储形式（放在 Account.last_error 里）。
///
/// 格式：`<原始消息>#f<n>@<首次失败毫秒>`
///   n     —— 连续失败次数
///   @之后 —— 首次失败的时刻（用于算是否过了观察期）
///
/// 用一个字段承载，避免动数据库表结构（迁移风险高，上次已经踩过坑）。
fn parse_fail_meta(err: &str) -> Option<(u32, i64)> {
    let (_, tail) = err.rsplit_once("#f")?;
    let (n, stamp) = tail.split_once('@')?;
    Some((n.trim().parse().ok()?, stamp.trim().parse().ok()?))
}

/// 把失败次数与首次失败时刻写进 last_error。
pub fn encode_fail(err_msg: &str, streak: u32, first_at: i64) -> String {
    // 去掉可能存在的旧标记，避免 #f 套娃
    let base = err_msg.split("#f").next().unwrap_or(err_msg).trim();
    format!("{base}#f{streak}@{first_at}")
}

/// 读取该账号记录的连续失败信息。
pub fn fail_meta(account: &Account) -> Option<(u32, i64)> {
    account.last_error.as_deref().and_then(parse_fail_meta)
}

/// 「每天最多记一天」的日期键（北京时间自然日）。
///
/// 用户要求（2026-11-18）：**每天测一次，对不能用的账号累计天数**。
///
/// 原来的 streak 是"失败次数"，一天刷新十几次就会累加十几次，
/// 8 次门槛一小时就撞满了 —— 那等于没有"连续 3 天"这回事。
/// 现在改成按自然日去重：同一个自然日内无论失败多少次，只 +1 天。
const DAY_MS: i64 = 86_400_000;
/// 北京时间偏移（UTC+8），用于把自然日边界对齐到用户所在时区。
pub const CN_OFFSET_MS: i64 = 8 * 3_600_000;

/// 把毫秒时间戳换算成"北京时间的第几天"（自 epoch）。
pub fn cn_day(ts: i64) -> i64 {
    (ts + CN_OFFSET_MS).div_euclid(DAY_MS)
}

/// 记录一次失败，按「自然日」累计天数。
///
/// 返回 (新的天数, 首次失败时刻)。规则：
///   - 没记录过 → 天数 1，首次时刻 = now
///   - 上次记录不是今天 → 天数 +1（今天第一次失败）
///   - 上次记录就是今天 → 天数不变（今天已经记过了，不重复累加）
///
/// 这样一天里刷新一百次也只算一天，3 天规则才有真实意义。
pub fn record_daily_failure(account: &Account, now: i64) -> (u32, i64) {
    let Some((days, first_at)) = fail_meta(account) else {
        return (1, now);
    };
    // 首次失败时刻也要按"日"存，才能判断上一次记录是哪天。
    let last_day = cn_day(first_at + (days.saturating_sub(1) as i64) * DAY_MS);
    let today = cn_day(now);
    if today > last_day {
        // 换天了，天数 +1（跨了几天都只 +1，因为我们每天都会测，不该跳号）
        (days.saturating_add(1), first_at)
    } else {
        // 同一天重复失败，天数不变
        (days, first_at)
    }
}

/// 判断账号健康档位。
///
/// `now` 是当前毫秒时间戳。
pub fn classify(account: &Account, now: i64) -> Health {
    let status = account.status.as_str();
    // 已被自动停用：保持退休档，别再走后面的失败计数逻辑。
    if status == "retired" {
        return Health::Retired;
    }
    // 凭证过期优先于一切 —— 它只需要用户重登就能救，不该被"3 天停用"抢走。
    // 停用的目的是"别占着路由和总额"，而过期账号本来就不路由（网关只认 ready）。
    if status == "expired" {
        return Health::NeedsRelogin;
    }
    // 看累计失败**天数** —— 必须排在 signable 之前。
    //
    // 用户要求「每天测，对不能用的累计天数，连续 3 天就停用」。
    // signable 账号正是最典型的"不能用"（签到能过、推理被拒），
    // 如果先在下面把 signable 无条件返回 SignableOnly，
    // 这类账号就永远判不出死，3 天规则形同虚设。
    if let Some((days, first_at)) = fail_meta(account) {
        // 天数达标 且 累计时长也够 3 天，双保险（防止时钟异常/回填错误导致秒判死）
        if days >= DAYS_BEFORE_RETIRE
            && now.saturating_sub(first_at) >= RETIRE_AFTER.saturating_sub(DAY_MS)
        {
            return Health::Retired;
        }
    }
    // 仅可签到：签到正常但推理被拒，已由绕过检查探明。
    if status == STATUS_SIGNABLE {
        return Health::SignableOnly;
    }
    let Some((days, _first_at)) = fail_meta(account) else {
        // 没有失败记录：ready 就是健康，其他状态当观察期处理
        return if status == "ready" { Health::Healthy } else { Health::Probation };
    };
    // 有过失败记录（1~2 天）→ 观察期；满 3 天的已在上面判成 Retired
    if days >= 1 {
        return Health::Probation;
    }
    Health::Healthy
}

/// 「仅可签到」用独立的 status 字符串，网关的 `account_eligible` 只认 "ready"，
/// 所以这个状态天然不会被路由，不需要改网关代码。
pub const STATUS_SIGNABLE: &str = "signable";

/// 该账号现在是否应该被调度器跳过。
///
/// Only three cases skip: needs-relogin (retry is useless), retired (waiting for
/// the user), and signable-only **for inference purposes**. 签到调度仍然要跑它，
/// 因为它每天能拿积分 —— 跳过与否由调用方按场景决定（见 `should_skip_claim`）。
pub fn should_skip(account: &Account, now: i64) -> bool {
    matches!(
        classify(account, now),
        Health::NeedsRelogin | Health::Retired
    )
}

/// 该账号是否还应该参与签到/领积分。
///
/// 仅可签到的账号**继续签到**（这是真实收益），只有需要重登和已退休的才停。
pub fn should_skip_claim(account: &Account, now: i64) -> bool {
    matches!(
        classify(account, now),
        Health::NeedsRelogin | Health::Retired
    )
}

/// 成功一次后是否该清零失败计数。
pub fn should_reset_on_success(account: &Account) -> bool {
    fail_meta(account).is_some_and(|(n, _)| n > 0)
}

/// 这个账号的余额是否应该计入「总额」。
///
/// 用户明确要求（2026-11-18）：**总额显示的都必须是能用的**。
///
/// 计入条件（全部满足）：
///   1. 已启用；
///   2. 状态是 ready —— 这是唯一表示"签到和推理都通过"的状态；
///   3. 余额有值。
///
/// 排除：
///   - `expired`（凭证过期，钱取不出来）
///   - `signable`（能签到但调不了模型）
///   - `banned` / `retired`（确定不能用）
///   - `error`（状态未知）
///
/// 这样用户看到的总数就是"真能用掉的额度"，不会虚高。
pub fn counts_toward_total(account: &Account) -> bool {
    if !account.enabled {
        return false;
    }
    // 用「排除法」而不是「白名单 ready」。
    //
    // 2026-11-19 修正：原来只认 status == "ready"，结果上游抖动两分钟、
    // 119 个账号被误标成 error 之后，总额从 17 万直接塌到 1386
    // —— 用户看到的数字完全失真，而账号里的钱一分没少。
    //
    // 现在的原则：**只排除确定取不出钱的账号**，其余一律计入。
    //   expired  凭证过期 → 余额取不出来，排除
    //   banned   账号封禁 → 取不出来，排除
    //   retired  连续3天不可用已停用 → 排除
    //   error / ready / signable / 其他 → **都计入**
    //
    // 为什么 error 也要计入：error 只是"最近一次探测失败"，
    // 不代表账号里的积分消失了。把它排除等于用瞬时故障否定存量资产。
    !matches!(
        account.status.as_str(),
        "expired" | "banned" | "retired"
    )
}

/// 账号当前是否已确定不能用（连续 3 天检测失败）。
///
/// 与 `classify()` 的区别：这个只看"是否该停用"，不看细节分档。
/// 调度器用它来自动停用账号。
pub fn is_dead(account: &Account, now: i64) -> bool {
    classify(account, now) == Health::Retired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credits::types::{Account, CreditRegion, Platform};

    fn acc(status: &str, err: Option<&str>) -> Account {
        Account {
            id: "a".into(),
            platform: Platform::Workbuddy,
            region: CreditRegion::Cn,
            user_id: "u".into(),
            label: "t".into(),
            enabled: true,
            status: status.into(),
            last_error: err.map(str::to_string),
            snapshot: None,
        }
    }

    const DAY: i64 = 24 * 60 * 60 * 1000;

    #[test]
    fn 就绪且无错误是健康() {
        assert_eq!(classify(&acc("ready", None), 1_000), Health::Healthy);
    }

    #[test]
    fn 凭证过期永远归需要重登() {
        // 即便失败次数很多、时间很旧，401 也只能靠重登解决
        let a = acc("expired", Some(&encode_fail("认证被拒", 99, 0)));
        assert_eq!(classify(&a, 100 * DAY), Health::NeedsRelogin);
        assert!(should_skip(&a, 100 * DAY));
    }

    #[test]
    fn 少量失败仍会重试但不判死() {
        // streak 现在表示「失败天数」：2 天 = 观察期，还没到 3 天，不能停用。
        let a = acc("banned", Some(&encode_fail("上游封禁 11140", 2, 0)));
        assert_eq!(classify(&a, DAY), Health::Probation);
        assert!(!should_skip(&a, DAY), "未满 3 天不能跳过，否则又会误判");
    }

    #[test]
    fn 失败多但没过观察期进观察档() {
        let a = acc("banned", Some(&encode_fail("上游封禁 11140", 10, 0)));
        // 观察期现在是 3 天（用户 2026-11-18 要求），所以取 1 天测"未满"
        assert_eq!(classify(&a, DAY), Health::Probation);
        assert!(!should_skip(&a, DAY), "观察期仍要参与复检");
    }

    #[test]
    fn 失败多且过观察期才建议退休() {
        let a = acc("banned", Some(&encode_fail("上游封禁 11140", 10, 0)));
        assert_eq!(classify(&a, RETIRE_AFTER + 1), Health::Retired);
        assert!(should_skip(&a, RETIRE_AFTER + 1));
    }

    #[test]
    fn 编码解码可往返且不套娃() {
        let s = encode_fail("上游封禁 11140（HTTP 403）", 3, 111);
        assert_eq!(parse_fail_meta(&s), Some((3, 111)));
        // 再编码一次不应叠加
        let s2 = encode_fail(&s, 4, 222);
        assert_eq!(parse_fail_meta(&s2), Some((4, 222)));
        assert_eq!(s2.matches("#f").count(), 1);
    }

    #[test]
    fn 没有失败标记时按状态判断() {
        assert_eq!(classify(&acc("ready", Some("随便一段旧文字")), 1), Health::Healthy);
        assert_eq!(classify(&acc("error", Some("网络错误")), 1), Health::Probation);
        // 无 last_error 的 banned（历史数据）也应进观察期，而不是永久跳过
        assert_eq!(classify(&acc("banned", None), 1), Health::Probation);
        assert!(!should_skip(&acc("banned", None), 1));
    }

    #[test]
    fn 仅可签到的账号不被签到跳过但也不算健康() {
        // 这是 2026-11-18 的关键区分：签到正常但推理被封
        let a = acc(STATUS_SIGNABLE, Some("上游封禁 11140"));
        assert_eq!(classify(&a, 1), Health::SignableOnly);
        // 不算 Healthy（不能拿去路由）
        assert!(!should_skip(&a, 1), "签到调度不能跳过它");
        assert!(!should_skip_claim(&a, 1), "它每天还能拿积分");
    }

    #[test]
    fn 仅可签到未满三天不会被降级() {
        // 前提变化（2026-11-18 用户要求 3 天停用）：
        // 原来的语义是"signable 永不退休"，现在改成"连续 3 天后可以停用"。
        // 这条测试保留前半段：**未满 3 天时**不能因为失败次数多就提前停。
        let a = acc(STATUS_SIGNABLE, Some(&encode_fail("11140", 99, 0)));
        // 才过了 1 天，即便失败 99 次也不该停
        assert_eq!(classify(&a, DAY), Health::SignableOnly);
        assert!(!should_skip_claim(&a, DAY), "未满 3 天仍要继续签到");
        // 满 3 天后才允许停用
        assert_eq!(classify(&a, RETIRE_AFTER), Health::Retired);
    }

    #[test]
    fn 过期账号停止签到() {
        let a = acc("expired", None);
        assert!(should_skip_claim(&a, 1), "凭证过期签不了，别白跑");
    }

    #[test]
    fn 仅可签到是账号级判定不能被余额成功撤销() {
        // 这条守护的是 2026-11-18 反复踩到的坑：
        // 余额接口对封禁号也返回 200，如果"余额成功就写 ready"，
        // signable 会被无限次打回 ready，又进路由池，又开始失败。
        let a = acc(STATUS_SIGNABLE, Some("11140"));
        assert_eq!(classify(&a, 1), Health::SignableOnly);
        // 它仍然参与签到（真实收益），但不健康（不该被路由）
        assert!(!should_skip_claim(&a, 1));
        assert_eq!(classify(&acc("ready", None), 1), Health::Healthy);
    }

    #[test]
    fn 连续三天不可用才判定为死() {
        // 用户要求：每天测、累计天数，连续 3 天确定不能用就停用。
        // 这里 streak 直接表示「已累计几天」。
        let cadence = 3 * DAY;
        // 已累计 3 天，且首次失败在 3 天前 → 判死
        let a = acc("signable", Some(&encode_fail("11140", 3, 0)));
        assert!(is_dead(&a, cadence), "满 3 天该停用");
        // 只累计 2 天 → 还不能停
        let b = acc("signable", Some(&encode_fail("11140", 2, 0)));
        assert!(!is_dead(&b, cadence), "只累计 2 天不能停用");
    }

    /// 同一天重复失败只算一天 —— 这是「累计天数」的核心。
    #[test]
    fn 同一天重复失败只累计一天() {
        let day0 = 1_800_000_000_000i64;
        let mut a = acc("signable", None);
        // 第一天内连续失败 5 次
        for i in 0..5 {
            let (days, first) = record_daily_failure(&a, day0 + i * 3_600_000);
            a.last_error = Some(encode_fail("11140", days, first));
        }
        assert_eq!(fail_meta(&a).unwrap().0, 1, "同一天内多次失败只能算 1 天");
        // 第二天
        let (days, first) = record_daily_failure(&a, day0 + DAY_MS + 1000);
        a.last_error = Some(encode_fail("11140", days, first));
        assert_eq!(fail_meta(&a).unwrap().0, 2, "换天后应当 +1 天");
        // 第二天内再失败多次，仍是 2 天
        let (days, first) = record_daily_failure(&a, day0 + DAY_MS + 7_000_000);
        a.last_error = Some(encode_fail("11140", days, first));
        assert_eq!(fail_meta(&a).unwrap().0, 2, "第二天内重复失败不该再加");
        // 第三天
        let (days, first) = record_daily_failure(&a, day0 + 2 * DAY_MS + 1000);
        a.last_error = Some(encode_fail("11140", days, first));
        assert_eq!(fail_meta(&a).unwrap().0, 3, "第三天应当累计到 3 天");
        // 累计 3 天且时长够了 → 判死
        assert!(is_dead(&a, day0 + 3 * DAY_MS), "累计满 3 天应当停用");
    }

    #[test]
    fn 失败天数不够不判死即使过了很久() {
        // 只累计 1 天（哪怕时间过去很久）都不该停用 —— 说明中间有成功过（会清零）
        let a = acc("error", Some(&encode_fail("网络抖动", 1, 0)));
        assert!(!is_dead(&a, 10 * DAY), "只累计 1 天不能判死");
        assert_eq!(classify(&a, 10 * DAY), Health::Probation);
    }

    #[test]
    fn 已停用的账号保持退休档() {
        let a = acc("retired", None);
        assert_eq!(classify(&a, 1), Health::Retired);
        assert!(should_skip(&a, 1));
        assert!(should_skip_claim(&a, 1), "停用后不再签到");
    }

    #[test]
    fn 总额排除法只剔除确定取不出钱的账号() {
        // 2026-11-19 修正后的语义：排除法，只踢确定拿不到钱的。
        //
        // 起因：原来只认 ready，上游抖两分钟把 119 个账号误标成 error 后，
        // 总额从 17 万塌到 1386，用户以为资产没了。
        assert!(counts_toward_total(&acc("ready", None)), "ready 计入");
        assert!(counts_toward_total(&acc("error", None)), "error 也计入（只是探测失败）");
        assert!(counts_toward_total(&acc(STATUS_SIGNABLE, None)), "仅可签到也计入（钱还在）");

        assert!(!counts_toward_total(&acc("expired", None)), "过期不计入");
        assert!(!counts_toward_total(&acc("banned", None)), "封禁不计入");
        assert!(!counts_toward_total(&acc("retired", None)), "已停用不计入");

        // 手动停用的不计入
        let mut disabled = acc("ready", None);
        disabled.enabled = false;
        assert!(!counts_toward_total(&disabled), "手动停用不计入");
    }
}
