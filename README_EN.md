<div align="center">

<img src="assets/brand/icon-1024-transparent.png" width="128" alt="Quoly AI">

# Quoly AI

**Four AI platforms' quotas behind one endpoint — stop running out mid-task**

[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)]()
[![Built with](https://img.shields.io/badge/built%20with-Tauri%202-orange)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Version](https://img.shields.io/badge/version-2.3.0-brightgreen)](https://github.com/zenolore/quoly-ai/releases)
[![Stars](https://img.shields.io/github/stars/zenolore/quoly-ai?style=social)](https://github.com/zenolore/quoly-ai/stargazers)

[简体中文](README.md) · **English**

[Website](https://luforge.com) · [Download](https://github.com/zenolore/quoly-ai/releases) · [Issues](https://github.com/zenolore/quoly-ai/issues) · [Changelog](CHANGELOG.md)

**The author uses this every day. Last 7 days: 55,671 requests, 9.9 billion tokens —
every feature here was ground out of real use, not a demo.**

</div>

---

## What it solves

If you pay for WorkBuddy, TRAE, Qoder and MiniMax at the same time, you've probably hit these:

- You run out of quota mid-refactor and have to stop until tomorrow
- Four platforms, dozens of accounts, and you click claim buttons by hand every day
- Every client needs its own API config; switching tools means starting over
- No idea where your quota went, or how many tokens you actually burned

**Quoly AI puts all of it into one desktop app.**

## What it feels like

**With the local gateway running, any client needs exactly one config:**

```bash
Base URL:  http://127.0.0.1:17860/v1
API Key:   copy from the "Credits & Proxy" page
Model:     workbuddy/deepseek-v4.1-flash
```

**That gives you 73 models** spread across 101 accounts. When an account runs out or gets rate-limited, the request rolls to the next one — and the reset time the platform reports in its limit response is remembered, so that account is skipped until it actually recovers.

**Every morning** it claims the daily check-in and task credits on every platform. You don't touch it.

## Real numbers from production

One week on the author's own machine:

| Metric | Value |
|---|---|
| Requests (7d) | **55,671** |
| Tokens (7d) | **9.9 billion** (~1.4B/day) |
| Peak day | 3.0B tokens / 16,889 requests (2026-11-09) |
| Accounts | **101** (WorkBuddy 56 / Qoder 21 / TRAE 18 / MiniMax 6) |
| Models | **73** |
| TRAE check-in | **5/18 succeed** (28%; the rest have no check-in activity on the platform side) |

These change daily, because the project is still used and changed daily.

## Four platforms, one entry point

| Platform | Accounts | Credit claiming | Models |
|---|---|---|---|
| WorkBuddy | 56 | check-in + task accept/claim + streak + makeup + lottery + travel | 38 |
| Qoder | 21 | daily check-in | 11 |
| TRAE | 18 | daily check-in | 22 |
| MiniMax | 6 | daily check-in | 2 |

## Also included

- **MCP & Skills management** — write configs for 25 AI coding tools in one click, no hand-editing JSON
- **Usage ledger** — every request's tokens, latency and credit cost in local SQLite, full history
- **Account rotation** — 2 concurrent per account, 16 pool-wide; rate-limited accounts cool down until the exact time the platform gave
- **Long requests survive** — waits up to 180s for the first byte, so reasoning models don't get cut off mid-thought
- **Zero upload** — credentials live in the OS keychain, never sent anywhere
- **Cross-platform** — Windows / macOS / Linux, one Tauri 2 core

## Download

Grab the latest from [Releases](https://github.com/zenolore/quoly-ai/releases):

- **Windows**: `Quoly AI_2.3.0_x64-setup.exe` (10.2 MB)
- **macOS**: `.dmg` (Apple Silicon / Intel)
- **Linux**: `.AppImage` / `.deb`

Install over the previous version to upgrade — accounts and data live in `%USERPROFILE%\.ai-toolkit\` and are preserved.

Or build from source; see below.

## Development

```bash
pnpm install
pnpm tauri dev      # dev mode
pnpm tauri build    # build installers
```

Requires Node 20+, Rust 1.77+, and Tauri 2's system dependencies.

## Data & security

This app has to read your account credentials to work, so where it stores them and where it sends requests is documented in the open:

- **Credentials live in the OS keychain** (Windows Credential Manager / macOS Keychain / Linux Secret Service), never plaintext files
- **The gateway binds `127.0.0.1` only** — not the LAN, not the internet
- **No telemetry endpoints at all** — nothing is sent back to the author

Full details in **[docs/OPEN-SOURCE.md](docs/OPEN-SOURCE.md)**: what's open, what isn't, where keys live, and notes for forking.

## Credits

Quoly AI is a fork of [whyfail/AI-Toolkit](https://github.com/whyfail/AI-Toolkit) (MIT) and references protocol work from:

- [smart-open/TraeWorkAssistant](https://github.com/smart-open/TraeWorkAssistant) (MIT)
- [L0NE-6/Trae-AutoCheckin](https://github.com/L0NE-6/Trae-AutoCheckin) (MIT) — empirical findings on TRAE device-ID semantics
- cpa-multi-plugins (MIT, Copyright 2026 mmqz)

Full attribution in [NOTICE.md](NOTICE.md). The upstream copyright notice is preserved in [LICENSE](LICENSE), as MIT requires.

### Changes relative to upstream

- New brand identity and icon (transparent rounded corners)
- **TRAE check-in fixed**: lowercase `x-device-id` header plus the full device header set, ending the long-running 9074 failures; per-`uid` derived device IDs take 18 accounts from 0 to 5 reliably claimable
- **Full WorkBuddy activity coverage**: check-in, task accept, task claim, streak, makeup cards, lottery, travel claim
- **Rate-limit rotation**: parses the reset time out of the platform's `6004` response and skips that account until it recovers
- **Long-request timeout**: header wait 60 → 180s so reasoning models aren't cut off
- Claim detail folds by default; overview page trimmed

## License

MIT, see [LICENSE](LICENSE).

## Disclaimer

A personal tool, unaffiliated with WorkBuddy, TRAE, Qoder or MiniMax. Multi-account automation may violate those platforms' terms of service — use at your own risk.

---

<div align="center">

**If this helped you, a ⭐ Star keeps it maintained**

[![Star History Chart](https://api.star-history.com/svg?repos=zenolore/quoly-ai&type=Date)](https://star-history.com/#zenolore/quoly-ai&Date)

</div>
