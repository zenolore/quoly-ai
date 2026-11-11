# 第三方声明 / Third-Party Notices

Quoly AI 基于开源项目修改而来。本文件列出所有上游来源，依据各自许可证的要求予以署名。

Quoly AI is derived from open-source projects. This file credits all upstream
sources as required by their respective licenses.

---

## 上游项目 / Upstream project

### AI Toolkit

- 仓库 / Repository: https://github.com/whyfail/AI-Toolkit
- 作者 / Author: 吴磊 (whyfail)
- 许可证 / License: MIT
- 用途 / Usage: Quoly AI 的整体架构（MCP 管理、Skills 管理、积分与代理面板、
  本地模型网关）基于该项目修改。

```
MIT License

Copyright (c) 2026 吴磊 (whyfail)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### TraeWorkAssistant

- 仓库 / Repository: https://github.com/smart-open/TraeWorkAssistant
- 许可证 / License: MIT
- 用途 / Usage: TRAE 登录流程的协议形状（IDE client id、授权页参数、
  `x-cloudide-token` 交换头、IDE_PC DeviceInfo、callback-echo 身份）参考该项目的
  抓包实现；本项目的 Rust 代码为独立实现，未复制其源码。

### cpa-multi-plugins

- 许可证 / License: MIT
- 版权 / Copyright: 2026 mmqz
- 用途 / Usage: 设备号派生规则（`sha256(uid)` 前 8 字节映射到 16 位十进制）
  与 9074 换号重试策略参考该项目。

---

## 使用的开源组件 / Open-source components

前端与后端的直接依赖（Tauri、React、tokio、reqwest 等）各自的许可证见
`node_modules` 与 `src-tauri/Cargo.toml`；发行版打包时由构建工具自动汇总。

### 平台标识 / Platform marks

界面上展示的各 AI 平台图标（WorkBuddy、TRAE、Qoder、MiniMax 等）版权归各自
所有方，仅用于标识所对接的服务，不表示任何形式的官方关联或背书。

The platform icons shown in the UI are trademarks of their respective owners.
They are used solely to identify the services this tool integrates with and do
not imply any official affiliation or endorsement.

---

## Quoly AI 自身的修改 / Changes made in Quoly AI

- 替换应用图标与品牌标识
- 调整默认展示（今日领取详情默认展开）
- 修正 TRAE 签到请求头（小写 `x-device-id` + 完整设备头）与设备号轮换重试
- 补充 MiniMax 模型目录兜底清单
- 端口与更新源改指向本项目

以上修改同样以 MIT 许可证发布，详见 `LICENSE`。
