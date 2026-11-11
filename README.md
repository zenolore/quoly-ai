<div align="center">

# Quoly AI

**一个本地网关，统一管理你所有的 AI 编程工具额度**

管理 MCP 服务器 · Skills 技能 · 多平台积分 · 本地模型代理

</div>

---

## 这是什么

Quoly AI 是一款桌面管理器，把散落在各个 AI 编程工具里的额度、配置、技能集中到一处：

- **多账号积分池**：WorkBuddy、TRAE、Qoder、MiniMax 的账号统一管理，自动领积分
- **本地模型网关**：所有账号汇聚成一个 OpenAI 兼容端点，任何客户端都能调
- **MCP / Skills 管理**：25 种 AI 编程工具的 MCP 与技能一键安装
- **用量统计**：每次调用的 token、耗时、积分消耗本地留档

## 支持的平台

| 平台 | 积分领取 | 模型调用 |
|---|---|---|
| WorkBuddy | 签到 + 任务 + 连签 | 38 个模型 |
| TRAE | 每日签到 | 22 个模型 |
| Qoder | 每日签到 | 11 个模型 |
| MiniMax | 每日签到 | 2 个模型 |

## 核心用法

开启本地代理后，任何支持 OpenAI 协议的客户端都可以这样接入：

```
Base URL:  http://127.0.0.1:17860/v1
API Key:   在「积分与代理」页复制
模型名:    平台/模型，例如 workbuddy/deepseek-v4.1-flash
```

调用会按账号余额自动轮换，某个账号额度用完自动切下一个。

## 来源与致谢

Quoly AI 基于 [whyfail/AI-Toolkit](https://github.com/whyfail/AI-Toolkit)（MIT）修改而来，
并参考了以下开源项目的协议实现：

- [smart-open/TraeWorkAssistant](https://github.com/smart-open/TraeWorkAssistant)（MIT）
- cpa-multi-plugins（MIT，Copyright 2026 mmqz）

完整的第三方署名见 [NOTICE.md](NOTICE.md)。

上游项目的版权声明保留在 [LICENSE](LICENSE) 与 NOTICE 中，这是 MIT 许可证的要求。

## 相对上游的主要改动

- 全新品牌标识与图标
- 修正 TRAE 签到：请求头改为小写 `x-device-id` 并补全设备头，解决长期 9074 失败
- TRAE 设备号轮换重试（9074 时换新号，最多 4 轮）
- MiniMax 模型目录拉取失败时使用内置清单兜底
- 「今日领取详情」默认展开
- 更新源与本项目地址绑定，不再指向上游仓库

## 开发

```bash
pnpm install
pnpm tauri dev      # 开发
pnpm tauri build    # 打包
```

需要 Node 20+、Rust 1.77+、Tauri 2 的系统依赖。

## 许可证

MIT，见 [LICENSE](LICENSE)。

## 免责声明

本项目为个人工具，与 WorkBuddy、TRAE、Qoder、MiniMax 等平台的官方无关。
使用多账号自动化功能请遵守各平台服务条款，风险自负。
