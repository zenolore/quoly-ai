<div align="center">

<img src="assets/brand/icon-1024-transparent.png" width="128" alt="Quoly AI">

# Quoly AI

**把四个 AI 平台的额度合成一个端点，写代码时不再被额度打断**

[![Platform](https://img.shields.io/badge/平台-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)]()
[![Built with](https://img.shields.io/badge/built%20with-Tauri%202-orange)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Version](https://img.shields.io/badge/version-2.3.0-brightgreen)](https://github.com/zenolore/quoly-ai/releases)
[![Stars](https://img.shields.io/github/stars/zenolore/quoly-ai?style=social)](https://github.com/zenolore/quoly-ai/stargazers)

**简体中文** · [English](README_EN.md)

[官网](https://quoly-ai.luforge.com) · [下载](https://github.com/zenolore/quoly-ai/releases) · [问题反馈](https://github.com/zenolore/quoly-ai/issues) · [更新日志](CHANGELOG.md)

**作者本人每天都在用。最近 7 天跑了 5.5 万次请求、消耗 99 亿 token ——
这个项目的每个功能都是从真实使用里磨出来的。**

</div>

---

## 它解决什么问题

如果你同时订了 WorkBuddy、TRAE、Qoder、MiniMax，大概率遇到过这些事：

- 写代码写到一半，额度用完，被迫停下来等第二天
- 四个平台各有一堆账号，每天要挨个点签到领积分
- 每个客户端都要单独配一遍 API，换个工具就重来一次
- 不知道额度花在哪了，token 到底用了多少

**Quoly AI 把这些全收进一个桌面程序里。**

## 用起来是什么体验

**开启本地代理后，任何客户端只填一次：**

```bash
Base URL:  http://127.0.0.1:17860/v1
API Key:   在「积分与代理」页一键复制
模型:      workbuddy/deepseek-v4.1-flash
```

**然后你就有 73 个模型可用**，分布在 101 个账号上。某个账号额度耗尽或被限流时，请求会自动切到下一个——平台在限流回复里给出的恢复时刻会被记住，那个账号在恢复前不再参与选择。

**每天早上**，它自动把所有平台的签到和任务积分领完，你不用管。

## 实际在用的数据

作者自己在生产环境跑了一周：

| 指标 | 数字 |
|---|---|
| 最近 7 天请求 | **55,671 次** |
| 最近 7 天 token | **99 亿**（日均 14 亿） |
| 单日峰值 | 30 亿 token / 16,889 次请求（2026-11-09） |
| 接入账号 | **101 个**（WorkBuddy 56 / Qoder 21 / TRAE 18 / MiniMax 6） |
| 可用模型 | **73 个** |
| TRAE 签到 | **5/18 成功**（28%，其余账号平台侧无签到活动） |

这些数字每天都在变，因为这个项目还在每天用、每天改。

## 四个平台，一个入口

| 平台 | 账号数 | 积分领取 | 模型 |
|---|---|---|---|
| WorkBuddy | 56 | 签到 + 任务接取领取 + 连签 + 补签 + 抽奖 + 旅行 | 38 |
| Qoder | 21 | 每日签到 | 11 |
| TRAE | 18 | 每日签到 | 22 |
| MiniMax | 6 | 每日签到 | 2 |

## 还有什么

- **MCP 与 Skills 管理** — 25 种 AI 编程工具的配置一键写入，不用手工改 JSON
- **用量台账** — 每次请求的 token、耗时、积分消耗都记在本地 SQLite，随时能查历史
- **账号自动轮换** — 单账号并发 2、池内全局 16，限流账号按平台给的时刻自动冷却
- **长请求不中断** — 推理模型首字延迟最长等 180 秒，不会因为生成慢被中途掐断
- **零上传** — 所有账号凭据加密存在系统钥匙串，不上传任何服务器
- **跨平台** — Windows / macOS / Linux，同一个 Tauri 2 内核

## 下载

从 [Releases](https://github.com/zenolore/quoly-ai/releases) 取最新安装包：

- **Windows**：`Quoly AI_2.3.0_x64-setup.exe`（10.2 MB）
- **macOS**：`.dmg`（Apple Silicon / Intel）
- **Linux**：`.AppImage` / `.deb`

覆盖安装即可升级，账号与数据保存在 `%USERPROFILE%\.ai-toolkit\`，不会丢失。

也可以从源码构建，见下面的开发说明。

## 开发

```bash
pnpm install
pnpm tauri dev      # 开发模式
pnpm tauri build    # 打包安装包
```

需要 Node 20+、Rust 1.77+，以及 Tauri 2 的系统依赖。

## 数据与安全

这个程序要读你的账号凭据才能工作，所以它把凭据存在哪、请求发往哪，都写成文档公开：

- **凭据存在系统钥匙串**（Windows 凭据管理器 / macOS 钥匙串 / Linux Secret Service），
  不写明文文件
- **网关只监听 `127.0.0.1`**，不对局域网或公网开放
- **没有任何上报或遥测端点**，程序不向作者发送任何数据

完整说明见 **[docs/OPEN-SOURCE.md](docs/OPEN-SOURCE.md)** —— 包括开源了什么、
没开源什么、Key 存在哪、二次开发的注意事项。

## 来源与致谢

Quoly AI 基于 [whyfail/AI-Toolkit](https://github.com/whyfail/AI-Toolkit)（MIT）修改而来，
并参考了以下项目的协议实现：

- [smart-open/TraeWorkAssistant](https://github.com/smart-open/TraeWorkAssistant)（MIT）
- [L0NE-6/Trae-AutoCheckin](https://github.com/L0NE-6/Trae-AutoCheckin)（MIT）—— TRAE 设备号语义的实测结论
- cpa-multi-plugins（MIT，Copyright 2026 mmqz）

完整署名见 [NOTICE.md](NOTICE.md)。上游版权声明保留在 [LICENSE](LICENSE) 中，
这是 MIT 许可证的要求。

### 相对上游的主要改动

- 全新品牌标识与图标（透明圆角）
- **TRAE 签到修复**：请求头改为小写 `x-device-id` 并补全设备头，解决长期 9074 失败；
  按 `uid` 派生专属设备号，18 个账号的签到成功率从 0 提到 5 个稳定可签
- **WorkBuddy 活动全量接入**：签到、任务接取、任务领取、连签、补签卡、抽奖、旅行领取
- **限流换号**：解析平台 `6004` 回复里的重置时刻，被限流的账号恢复前不再参与选号
- **长请求超时**：响应头等待 60 → 180 秒，推理模型不再被中途掐断
- 「今日领取详情」四个平台默认折叠，概览页精简

## 许可证

MIT，见 [LICENSE](LICENSE)。

## 免责声明

本项目为个人工具，与 WorkBuddy、TRAE、Qoder、MiniMax 等平台的官方无关。
多账号自动化功能请遵守各平台服务条款，风险自负。

---

<div align="center">

**如果这个项目帮到了你，点个 ⭐ Star 支持作者继续维护**

[![Star History Chart](https://api.star-history.com/svg?repos=zenolore/quoly-ai&type=Date)](https://star-history.com/#zenolore/quoly-ai&Date)

</div>
