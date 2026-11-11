<div align="center">

# Quoly AI

**One local gateway for every AI coding subscription you own**

MCP servers · Skills · Multi-account credits · Local model proxy

</div>

---

## What it is

Quoly AI is a desktop manager that pulls the credits, config and skills scattered
across your AI coding tools into one place:

- **Multi-account credit pools** for WorkBuddy, TRAE, Qoder and MiniMax, with automatic daily claims
- **Local model gateway** that merges every account into a single OpenAI-compatible endpoint
- **MCP / Skills management** for 25 AI coding tools
- **Usage tracking** — tokens, latency and credit spend logged locally per request

## Supported platforms

| Platform | Credit claims | Models |
|---|---|---|
| WorkBuddy | check-in + tasks + streaks | 38 |
| TRAE | daily check-in | 22 |
| Qoder | daily check-in | 11 |
| MiniMax | daily check-in | 2 |

## Quick start

Start the local proxy, then point any OpenAI-compatible client at it:

```
Base URL:  http://127.0.0.1:17860/v1
API Key:   copy it from the Credits & Proxy page
Model:     platform/model, e.g. workbuddy/deepseek-v4.1-flash
```

Requests rotate across accounts by remaining balance.

## Credits

Quoly AI is derived from [whyfail/AI-Toolkit](https://github.com/whyfail/AI-Toolkit) (MIT)
and references protocol work from:

- [smart-open/TraeWorkAssistant](https://github.com/smart-open/TraeWorkAssistant) (MIT)
- cpa-multi-plugins (MIT, Copyright 2026 mmqz)

Full attribution lives in [NOTICE.md](NOTICE.md). Upstream copyright notices are
preserved in [LICENSE](LICENSE) as the MIT license requires.

## Changes from upstream

- New brand identity and icons
- TRAE check-in fix: lowercase `x-device-id` plus the full device header set
- TRAE device-id rotation on 9074 (up to four rounds)
- Built-in MiniMax model list when the live catalog is unavailable
- "Today's claims" panel expanded by default
- Updater and repository links point at this project

## Development

```bash
pnpm install
pnpm tauri dev      # develop
pnpm tauri build    # package
```

Requires Node 20+, Rust 1.77+, and the Tauri 2 system dependencies.

## License

MIT — see [LICENSE](LICENSE).

## Disclaimer

Not affiliated with WorkBuddy, TRAE, Qoder or MiniMax. Multi-account automation
must comply with each platform's terms of service.
