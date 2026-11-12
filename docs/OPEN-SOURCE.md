# Quoly AI 开源说明

> 本文说明 Quoly AI 开源的内容范围、目的、数据边界与安全约定。
> 如果你准备安装、二次开发或转发本项目，请先读完这一页。

最后更新：2026-11-11

---

## 一、为什么开源

这个项目最初是作者自己用来管自己的 AI 额度的工具，不是为发布而写。

作者同时订了 WorkBuddy、TRAE、Qoder、MiniMax 四个平台，加起来一百多个账号。
每天要在四个客户端之间切来切去点签到，额度用完还得手动换号，token 花在哪也说不清。
把这些问题逐个解决掉之后，工具就长成了现在这个样子。

**开源的原因有三个：**

1. **省别人的时间。** 有同样困扰的人不用再把每个坑踩一遍——TRAE 的 9074
   到底是限流还是设备号被记住、WorkBuddy 的签到分和任务分怎么区分、
   Qoder 的模型目录为什么和网关不一致，这些都写在代码注释里了。
2. **让数据条款可验证。** 这个程序要读取你的账号凭据才能工作。与其让你相信
   一句「我们不上传数据」，不如把代码摊开——你可以自己确认凭据存在哪、
   请求发往哪。
3. **接受监督。** 涉及多账号自动化的工具，如果只有闭源版本可供下载，
   使用者无法判断它到底做了什么。开源是一种约束。

---

## 二、开源了什么

### 完整源码

| 部分 | 位置 | 说明 |
|---|---|---|
| 前端界面 | `src/` | React + TypeScript，含积分面板、代理面板、MCP/Skills 管理 |
| 后端内核 | `src-tauri/src/` | Rust + Tauri 2，含四平台适配器、本地网关、凭据管理 |
| 平台适配 | `src-tauri/src/credits/providers/` | WorkBuddy / TRAE / Qoder / MiniMax 各自的协议实现 |
| 构建脚本 | `scripts/`、`.github/workflows/` | 数据统计脚本、跨平台发布流水线 |

**没有任何模块被剥离或混淆。** 你下载到的源码就是编译出安装包的那份源码。

### 文档

- `README.md` / `README_EN.md` — 产品定位、功能、真实使用数据
- `docs/credits-proxy.md` — 本地网关的接入方式与路由规则
- `docs/security-hardening.md` — 安全加固措施与验证边界
- `docs/agent-adapters.md` — 25 种 AI 编程工具的配置适配说明
- `NOTICE.md` — 全部上游项目的署名
- 本文件 — 开源范围与数据边界

---

## 三、没有开源什么

这一节比上一节更重要。

### 1. 任何账号凭据

**仓库里没有一个可用的 API Key、Token 或账号。**

开源前做了逐文件扫描，覆盖 195 个文本文件，检查项包括：

```
sk-          OpenAI / 图像生成密钥
ghp_         GitHub Token
nvapi-       NVIDIA API Key
ait_         Quoly AI 网关密钥
ark-         火山引擎密钥
eyJ...       任意 JWT
refresh_token / api_key= / secret=  形式的赋值
```

**结果：0 处命中。**

扫描脚本本身也在仓库里（`scripts/readme-stats.py` 的数据来源说明可作参考），
你可以用同样的方法复查。

### 2. 用户数据

运行时的所有数据都在你的电脑上，**不在仓库里**：

| 数据 | 位置 | 是否入库 |
|---|---|---|
| 用量台账（每次请求的 token/耗时/积分） | `~/.ai-toolkit/ai-toolkit.db` | ❌ 否 |
| 崩溃日志 | `~/.ai-toolkit/crash-trail.log` | ❌ 否 |
| 技能安装记录 | `~/.ai-toolkit/detected.json` | ❌ 否 |
| 账号凭据 | 系统钥匙串（见下节） | ❌ 否 |

`.gitignore` 排除了 `*.db` 与 `*.log`，即使误放也不会被提交。

### 3. 作者的发布密钥

签名安装包用的私钥不写在代码里，由 GitHub Actions 从仓库 Secrets 读取：

```yaml
TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
```

**你 fork 之后无法用这个密钥签名，这是刻意的。**

---

## 四、你的 Key 存在哪

这是最常被问的问题，直接说结论：

### 账号凭据：系统钥匙串

程序不把 Token 写进任何明文文件，而是交给操作系统保管：

| 系统 | 存储位置 | 实现 |
|---|---|---|
| Windows | 凭据管理器（Credential Manager） | `CredReadW` / `CredWriteW` |
| macOS | 钥匙串（Keychain） | Security.framework |
| Linux | Secret Service（GNOME Keyring / KWallet） | D-Bus |

- **服务名**：`com.ai-toolkit.credits`
- **条目名**：每个账号一个，按账号 ID 区分
- **查看方式**：Windows 控制面板 → 凭据管理器 → Windows 凭据，搜 `ai-toolkit`

凭据条目以 `ait-secret-v1:` 开头，超过系统单条长度上限时会自动切成多段
存储（每段一个子条目），读回时再拼装，对使用者透明。

### 本地网关密钥：同样的钥匙串

「积分与代理」页面上那串 `ait_` 开头的密钥，由本机随机生成，只存在钥匙串里。
它不是账号 Token，只在 `127.0.0.1:17860` 这个本地端口上有效，**离开本机就调不通**。

### 请求发往哪里

程序只与四类地址通信：

| 目标 | 用途 |
|---|---|
| `api.trae.cn` / `api.trae.com.cn` | TRAE 签到与额度查询 |
| `www.codebuddy.cn` | WorkBuddy 签到与模型目录 |
| `qoder.com` 及其 API 域名 | Qoder 签到与模型目录 |
| `agent.minimax.cn` | MiniMax 签到与推理 |

**没有任何「上报」「统计」「遥测」端点。** 程序不发用户数据给作者，
作者也没有能接收这些数据的服务器。

本地网关只监听 `127.0.0.1`，不对局域网或公网开放。

---

## 五、二次开发须知

### 许可证：MIT

你可以自由使用、修改、商用、闭源再发布。唯一的要求是**保留版权声明**。

`LICENSE` 文件里同时包含两段版权：

```
Copyright (c) 2026 吴磊 (whyfail)     ← 上游 AI Toolkit
Copyright (c) 2026 zenolore           ← 本项目修改
```

**删掉任何一段都违反 MIT 协议。** 这是本仓库唯一不可协商的地方。

### 上游来源

| 项目 | 用途 | 许可证 |
|---|---|---|
| [whyfail/AI-Toolkit](https://github.com/whyfail/AI-Toolkit) | 整体架构 | MIT |
| [smart-open/TraeWorkAssistant](https://github.com/smart-open/TraeWorkAssistant) | TRAE 登录协议形状参考 | MIT |
| cpa-multi-plugins | 设备号派生规则 | MIT（2026 mmqz） |

细节见 `NOTICE.md`。**如果你的分支要发布，请把 NOTICE 一起带上。**

### 平台商标

界面里的平台图标（WorkBuddy、TRAE、Qoder、MiniMax 等）版权归各自所有方，
仅用于标识所对接的服务。**不表示任何官方关联或背书**，也没有获得这些平台的授权。

### 商用前请想清楚

MIT 允许你卖这个软件，但有两个现实问题：

1. **平台条款。** 多账号自动化可能违反 WorkBuddy / TRAE / Qoder / MiniMax 的
   服务条款。被封号的风险由使用者承担，与作者无关。
2. **你的用户会找你。** 卖出去就意味着要维护、要支持、要为平台改版跟进。
   作者只保证「我自己在用，坏了我会修」，不提供任何 SLA。

---

## 六、这个项目怎么维护

| 项 | 现状 |
|---|---|
| 维护者 | 作者本人（业余时间） |
| 更新节奏 | 不固定，取决于自己用到了什么问题 |
| Issue 响应 | 尽力而为，不承诺时限 |
| 破坏性变更 | 会写进 `CHANGELOG.md` |
| 数据统计 | 由 `scripts/readme-stats.py` 从本机台账生成，每周更新 |

**作者每天都在用这个程序**——最近一周 5.5 万次请求、99 亿 token 都走它。
所以核心链路不会长期坏着，但边缘功能（比如你用的某个冷门平台适配）可能要等。

---

## 七、安全承诺的边界

可以承诺的：

- ✅ 不上传账号凭据
- ✅ 不采集使用行为
- ✅ 网关只监听本机回环地址
- ✅ 源码与安装包同源

**不能承诺的：**

- ❌ 绝对无漏洞。这是个人项目，没有经过第三方安全审计。
- ❌ 平台永远不改协议。四个平台随时可能改接口，适配会滞后。
- ❌ 账号永远不被封。多账号操作本身有风险。

---

## 八、发现安全问题怎么办

如果你在代码里发现了凭据泄露、注入漏洞或其他安全问题：

1. **不要开公开 Issue**（会放大影响面）
2. 通过 [GitHub Security Advisory](../../security/advisories/new) 私下报告
3. 或者在官网 [quoly-ai.luforge.com](https://quoly-ai.luforge.com) 留言说明

确认后会在 `CHANGELOG.md` 记录修复，并按你的意愿署名致谢。

---

<div align="center">

[返回 README](../README.md) · [官网](https://quoly-ai.luforge.com) · [问题反馈](../../issues)

</div>
