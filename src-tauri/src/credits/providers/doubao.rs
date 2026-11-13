//! 豆包平台适配器 —— 转发到本机豆包网关。
//!
//! 和其他四个平台的根本区别：这里**不直连上游**，而是把请求转给
//! `127.0.0.1:9091`（豆包网关，Python + FastAPI）处理。那个进程负责：
//!
//!   · 账号池（多账号轮换、冷却、失效判定）
//!   · Cookie 维护（30 天过期、被服务端踢下线）
//!   · 每日签到（必须靠 Playwright 驱动真实浏览器）
//!   · 上游协议细节（SSE 帧格式、风控错误码）
//!
//! 为什么这么拆，而不是全搬进 Rust：
//!   1. 签到依赖 Playwright/Chromium，Rust 侧没有等价能力，搬不动
//!   2. 豆包风控频繁、活动页经常改版，Python 改起来比 Rust 快得多
//!   3. **崩溃隔离**：豆包网关挂掉时这里只会看到"连不上"，
//!      而它在独立进程里，绝不会把主网关一起带走
//!
//! 账号以什么形式存在 AiToolkit 里：
//!   不存 Cookie（那些在豆包网关的 accounts/*.json 里），只存网关地址和密钥。
//!   所以一个 AiToolkit 账号 == 一个豆包网关实例。
//!   `access_token` 放 API Key，`extra.gateway_url` 放网关地址。

use super::*;
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;

/// 默认网关地址。用户可以改，所以在 `extra.gateway_url` 里覆盖。
const DEFAULT_GATEWAY: &str = "http://127.0.0.1:9091";

/// 单次请求的硬超时。
///
/// 特意比通用平台短（那边是 180 秒）。豆包两种表现很鲜明：要么几秒就出字，
/// 要么直接返回风控错误。挂满三分钟的情况极罕见，而长时间挂起会让客户端
/// 以为整个网关死了 —— 宁可早点失败让它重试。
const CHAT_TIMEOUT: Duration = Duration::from_secs(90);

/// 探活/列模型的超时。这些是本地调用，应该很快。
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// 从账号里解析出网关地址和密钥。
fn endpoint(c: &Credentials) -> (String, String) {
    let url = c
        .extra
        .get("gateway_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_GATEWAY)
        .trim_end_matches('/')
        .to_string();
    (url, c.access_token.trim().to_string())
}

/// 把网关的错误响应翻译成用户能看懂的说明。
///
/// 豆包网关的错误有个特点：503 多半是"账号池全挂了"，而不是"服务不存在"。
/// 直接抛原始 JSON 出去，用户只会看到一堆 code。这里做一次归因。
fn explain(status: u16, body: &str) -> ProviderError {
    let detail = body.trim();
    let hint = if status == 401 || status == 403 {
        "网关 API Key 不对，检查账号里填的密钥"
    } else if status == 404 {
        "网关地址可能写错了（应以 /v1 结尾，端口默认 9091）"
    } else if status == 503 {
        match detail {
            d if d.contains("no_account") || d.contains("all accounts failed") => {
                "豆包账号全部不可用：可能都被限流，或有账号需要重新登录。\
                 打开豆包网关的 /admin 面板查看，重新登录后即可恢复"
            }
            _ => "豆包网关暂时不可用",
        }
    } else if status == 504 {
        "豆包响应超时：可能是上游风控在排队，稍后重试"
    } else if status >= 500 {
        "豆包网关内部错误"
    } else if status == 413 {
        "请求体过大"
    } else {
        "豆包请求失败"
    };
    let shown = if detail.is_empty() {
        String::new()
    } else {
        // 只截一段，避免把整页 HTML 塞进错误消息
        let cut: String = detail.chars().take(240).collect();
        format!("（{cut}）")
    };
    ProviderError::new(status, format!("{hint}{shown}"))
}

/// 建一个走本地网关的 HTTP 客户端。
///
/// `no_proxy()` 很关键：系统里如果配了 HTTP_PROXY，访问 127.0.0.1 也会被
/// 劫持成 502。这个坑今天在排查用量接口时踩过。
fn client(timeout: Duration) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()
        .map_err(|e| ProviderError::new(500, format!("创建 HTTP 客户端失败：{e}")))
}

/// 登录：不需要走认证流程，只要网关活着就算成功。
///
/// 语义是"这个网关实例在我这里注册为一个账号"。真正的 Cookie 维护
/// 全在网关那边，AiToolkit 不持有任何豆包凭证。
pub(super) async fn authenticate(
    _client: &ProviderClient,
    c: &Credentials,
) -> Result<(String, String, Credentials), ProviderError> {
    let (base, key) = endpoint(c);
    if key.is_empty() {
        return Err(ProviderError::new(400, "请填写豆包网关的 API Key"));
    }
    let http = client(PROBE_TIMEOUT)?;
    let resp = http
        .get(format!("{base}/v1/models"))
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| ProviderError::new(502, format!("连不上豆包网关 {base}：{e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(explain(status, &body));
    }
    // 用一个稳定的标识当 user_id：网关地址本身。同一个网关重复添加会识别为同一账号。
    let user_id = base.clone();
    Ok((user_id, "豆包网关".into(), c.clone()))
}

/// 刷新：豆包没有 OAuth refresh 那套东西。
///
/// 这里只做"探活"，顺便把网关报告的账号健康情况回填到 credentials，
/// 让上层能显示"还有几个号能用"。真正失效的号只能去网关面板重新登录，
/// 不是这里能自动修的 —— 所以不做假的自动续期，老实反映现状。
pub(super) async fn refresh(
    _client: &ProviderClient,
    c: &Credentials,
) -> Result<Credentials, ProviderError> {
    let (base, key) = endpoint(c);
    let http = client(PROBE_TIMEOUT)?;
    let resp = http
        .get(format!("{base}/admin/api/health"))
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| ProviderError::new(502, format!("连不上豆包网关：{e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(explain(status, &body));
    }
    let mut next = c.clone();
    if let Ok(health) = serde_json::from_str::<Value>(&body) {
        next.extra["health"] = health.clone();
        next.extra["usable_accounts"] = health
            .get("accounts_usable")
            .cloned()
            .unwrap_or(json!(0));
    }
    Ok(next)
}

/// 拉取账号快照：余额用"可用账号数"，模型清单从网关拿。
///
/// 为什么不显示豆包的额度点数当余额：豆包的额度单位是"点"，和别家的
/// credits 不是一个量纲，混在一起会让总余额失去意义。所以余额位置放
/// "几个号能用"这个更实用的指标，额度点数原样保留在 quota 文本里。
pub(super) async fn inspect(
    _client: &ProviderClient,
    c: &Credentials,
) -> Result<ProviderSnapshot, ProviderError> {
    let (base, key) = endpoint(c);
    let http = client(PROBE_TIMEOUT)?;

    let health = http
        .get(format!("{base}/admin/api/health"))
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|e| ProviderError::new(502, format!("连不上豆包网关：{e}")))?;
    let health_status = health.status().as_u16();
    let health_body = health.text().await.unwrap_or_default();
    if health_status != 200 {
        return Err(explain(health_status, &health_body));
    }
    let health_json: Value = serde_json::from_str(&health_body).unwrap_or_else(|_| json!({}));

    let models = match list_models(&base, &key).await {
        Ok(v) => v,
        Err(_) => vec![],
    };

    let total = health_json
        .get("accounts_total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let usable = health_json
        .get("accounts_usable")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let needs_relogin: Vec<String> = health_json
        .get("needs_relogin")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut diagnostic = None;
    if total > 0 && usable == 0 {
        diagnostic = Some(format!(
            "所有 {total} 个豆包账号当前都不可用，请到网关面板重新登录"
        ));
    } else if !needs_relogin.is_empty() {
        diagnostic = Some(format!("以下账号需要重新登录：{}", needs_relogin.join("、")));
    }

    Ok(ProviderSnapshot {
        balance: Some(usable as f64),
        unit: "个可用账号".into(),
        quota: Some(format!("{usable}/{total} 可用")),
        packs: vec![],
        rewards: vec![],
        models,
        fetched_at: now_ms(),
        stale: false,
        diagnostic,
        sync: SnapshotSync::default(),
    })
}

/// 从网关拉模型清单。
async fn list_models(base: &str, key: &str) -> Result<Vec<ModelInfo>, ProviderError> {
    let http = client(PROBE_TIMEOUT)?;
    let resp = http
        .get(format!("{base}/v1/models"))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| ProviderError::new(502, format!("连不上豆包网关：{e}")))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status != 200 {
        return Err(explain(status, &body));
    }
    let parsed: Value = serde_json::from_str(&body)
        .map_err(|e| ProviderError::new(502, format!("网关返回的不是合法 JSON：{e}")))?;
    let mut out = Vec::new();
    for entry in parsed.get("data").and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        // 网关返回的是 "doubao/doubao-quick" 这种带前缀的形式，
        // 这里把前缀剥掉，因为我们自己的 route_key 还会再加一层。
        let bare = id.strip_prefix("doubao/").unwrap_or(id).to_string();
        out.push(ModelInfo {
            canonical_id: None,
            upstream_id: Some(id.to_string()),
            upstream_model_ids: vec![id.to_string()],
            id: bare.clone(),
            name: bare,
            // 豆包这条路不支持 tool_calls（上游限制），如实标注，
            // 免得客户端带 tools 过来白跑一趟。
            supports_tools: false,
            context_window: None,
            context_window_options: vec![],
            max_output_tokens: None,
            supports_images: None,
            supports_reasoning: None,
            reasoning_effort_options: vec![],
            default_reasoning_effort: None,
            verified: false,
            verification_note: None,
        });
    }
    Ok(out)
}

/// 余额：用可用账号数表示（理由见 inspect 的注释）。
pub(super) async fn balance(
    _client: &ProviderClient,
    c: &Credentials,
) -> Result<ProviderSnapshot, ProviderError> {
    inspect(_client, c).await
}

/// 只拉模型清单（inspect 的拆分版本，供分步刷新用）。
pub(super) async fn models(
    _client: &ProviderClient,
    c: &Credentials,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let (base, key) = endpoint(c);
    list_models(&base, &key).await
}

/// 奖励：豆包签到由网关自己做，这里不重复实现。
///
/// 返回空列表而不是报错 —— 报错会让整个账号显示为异常状态，
/// 而"没在这里签到"本身不是错。
pub(super) async fn rewards(
    _client: &ProviderClient,
    _c: &Credentials,
) -> Result<Vec<Reward>, ProviderError> {
    Ok(vec![])
}

/// 领取：签到现在由网关的调度器（9:00 / 21:00）负责。
///
/// 这里不实现，是因为签到需要 Playwright，而 AiToolkit 是 Rust 后端，
/// 没有浏览器运行时。硬要在这里做，等于把浏览器塞进主程序，
/// 风险和体积都不划算。
///
/// 返回空列表而不是 Err：报错会让整个账号在面板上显示成异常状态，
/// 而"这里没有可领的东西"并不是故障。真正的签到状态在网关面板看。
pub(super) async fn claim(
    _client: &ProviderClient,
    _a: &Account,
    _c: &Credentials,
) -> Result<Vec<RewardOutcome>, ProviderError> {
    Ok(vec![])
}

/// 对话：把请求原样转给本地网关，再把返回的 SSE 转成我们的格式。
///
/// 关键点：网关本身就是 OpenAI 兼容协议，所以这里几乎不用转换，
/// 只要处理流式透传和错误归因。
pub(super) async fn chat(
    _client: &ProviderClient,
    c: &Credentials,
    m: &ModelInfo,
    body: Value,
) -> Result<ProviderChat, ProviderError> {
    let (base, key) = endpoint(c);
    let http = client(CHAT_TIMEOUT)?;

    // 网关的模型名要带 doubao/ 前缀，和它 /v1/models 的返回保持一致
    let upstream_model = m
        .upstream_id
        .clone()
        .unwrap_or_else(|| format!("doubao/{}", m.id));

    let mut payload = body.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("model".into(), json!(upstream_model));
    }

    let resp = http
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(&key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ProviderError::new(504, "豆包响应超时，可能正在排队或被风控")
            } else {
                ProviderError::new(502, format!("连不上豆包网关：{e}"))
            }
        })?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(explain(status, &body));
    }

    // 直接把字节流转成 ChatChunk 流。网关吐的已经是 OpenAI SSE，
    // 每个 data: 行就是一个完整 chunk，不需要再解析上游私有格式。
    let stream = async_stream_chunks(resp);
    Ok(ProviderChat { stream })
}

/// 把网关的响应体逐行读成 ChatChunk。
///
/// 这里没有复用 `providers/stream.rs`：那个是为"各平台私有 SSE 格式"
/// 写的，要做格式嗅探、跨平台字段映射、minimax 的工具调用累积等等。
/// 豆包网关给的已经是标准 OpenAI 格式，用那套反而绕远且容易误判。
fn async_stream_chunks(
    resp: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<ChatChunk, ProviderError>> + Send>> {
    let mut bytes = resp.bytes_stream();
    Box::pin(async_stream::stream! {
        let mut buf = Vec::<u8>::new();
        while let Some(chunk) = bytes.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(ProviderError::new(502, format!("读取豆包响应失败：{e}")));
                    return;
                }
            };
            buf.extend_from_slice(&chunk);
            // 按行切分，最后一段可能不完整，留在 buf 里等下一块
            loop {
                let Some(pos) = buf.iter().position(|b| *b == b'\n') else {
                    break;
                };
                let line = buf.drain(..=pos).collect::<Vec<u8>>();
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                let Some(payload) = text.strip_prefix("data:") else {
                    continue;
                };
                let payload = payload.trim();
                if payload == "[DONE]" {
                    return;
                }
                match serde_json::from_str::<Value>(payload) {
                    Ok(value) => {
                        // 网关把错误也塞在 data 里（流式无法改状态码了）
                        if let Some(err) = value.get("error") {
                            let msg = err
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("豆包返回错误");
                            yield Err(explain(502, msg));
                            return;
                        }
                        yield Ok(ChatChunk {
                            data: value,
                            upstream_model: None,
                        });
                    }
                    Err(_) => continue,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(url: &str, key: &str) -> Credentials {
        Credentials {
            region: CreditRegion::Cn,
            access_token: key.into(),
            refresh_token: None,
            user_id: None,
            device_id: None,
            expires_at: None,
            extra: json!({ "gateway_url": url }),
        }
    }

    #[test]
    fn endpoint_falls_back_to_default_and_strips_trailing_slash() {
        let (url, key) = endpoint(&creds("http://127.0.0.1:9091/", "k"));
        assert_eq!(url, "http://127.0.0.1:9091");
        assert_eq!(key, "k");

        let (url, _) = endpoint(&Credentials {
            region: CreditRegion::Cn,
            access_token: "k".into(),
            refresh_token: None,
            user_id: None,
            device_id: None,
            expires_at: None,
            extra: json!({}),
        });
        assert_eq!(url, DEFAULT_GATEWAY);
    }

    #[test]
    fn errors_get_actionable_hints_instead_of_raw_codes() {
        let e = explain(503, r#"{"detail":"all accounts failed: [710022004]"}"#);
        assert!(e.message.contains("重新登录"), "503 应指向重新登录：{}", e.message);

        let e = explain(401, "");
        assert!(e.message.contains("API Key"));

        let e = explain(404, "");
        assert!(e.message.contains("9091"));

        let e = explain(504, "");
        assert!(e.message.contains("超时"));
    }

    #[test]
    fn long_error_bodies_are_truncated() {
        let huge = "x".repeat(5000);
        let e = explain(500, &huge);
        // 不能把整页内容塞进错误消息
        assert!(e.message.chars().count() < 400, "错误消息过长：{}", e.message.chars().count());
    }

    /// 连不上网关时必须给出可操作的提示，而不是笼统的"失败"。
    ///
    /// 故意指向一个不存在的端口：这是最常见的真实故障（网关没启动），
    /// 用户需要看到"连不上 127.0.0.1:xxxx"才能自己解决。
    #[tokio::test]
    async fn unreachable_gateway_reports_the_address() {
        let c = creds("http://127.0.0.1:1", "dummy");
        let err = match authenticate(&test_client(), &c).await {
            Ok(_) => panic!("连不上的地址不该成功"),
            Err(e) => e,
        };
        assert_eq!(err.status, 502);
        assert!(
            err.message.contains("127.0.0.1:1"),
            "错误里应带上具体地址，实际是：{}",
            err.message
        );
    }

    /// 空 Key 要在发请求之前就拦下来。
    #[tokio::test]
    async fn empty_api_key_is_rejected_before_any_request() {
        let c = creds("http://127.0.0.1:1", "   ");
        let err = match authenticate(&test_client(), &c).await {
            Ok(_) => panic!("空 Key 不该通过"),
            Err(e) => e,
        };
        assert_eq!(err.status, 400);
        assert!(err.message.contains("API Key"));
    }

    /// 测试用：不走真实网络的 ProviderClient。
    fn test_client() -> ProviderClient {
        ProviderClient::new()
    }
}
