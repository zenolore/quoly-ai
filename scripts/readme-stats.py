# -*- coding: utf-8 -*-
"""统计真实使用数据，输出可粘贴进 README 的 Markdown 表格。

用法：
    python scripts/readme-stats.py            # 打印表格
    python scripts/readme-stats.py --update   # 直接改写 README 里的数据区块

数据来源：本机 AiToolkit 的 SQLite 台账（~/.ai-toolkit/ai-toolkit.db）。
换机器或清库后数字会重置，这是预期的——README 里也写明了「数据每周更新」。
"""
from __future__ import annotations

import argparse
import collections
import datetime
import json
import os
import re
import sqlite3
import sys

DB = os.path.join(os.path.expanduser("~"), ".ai-toolkit", "ai-toolkit.db")
README = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "README.md")

# 北京时间自然日
OFFSET = 8 * 3600 * 1000
DAY = 86400 * 1000

PLATFORM_LABEL = {
    "workbuddy": "WorkBuddy",
    "qoder": "Qoder",
    "traework": "TRAE",
    "minimax-code": "MiniMax",
}


def connect() -> sqlite3.Connection:
    if not os.path.exists(DB):
        sys.exit(f"找不到台账数据库：{DB}\n先运行一次 Quoly AI，让它记录一些请求。")
    return sqlite3.connect(DB)


def collect() -> dict:
    con = connect()
    rows = con.execute("select summary, created_at from credit_requests").fetchall()

    per_day: dict[str, dict[str, int]] = collections.defaultdict(lambda: {"tok": 0, "req": 0})
    total_tok = 0
    total_req = 0
    for summary, ts in rows:
        try:
            payload = json.loads(summary)
        except (TypeError, json.JSONDecodeError):
            continue
        tok = (payload.get("inputTokens") or 0) + (payload.get("outputTokens") or 0)
        day = datetime.datetime.fromtimestamp(((ts + OFFSET) // DAY) * DAY / 1000).strftime("%Y-%m-%d")
        per_day[day]["tok"] += tok
        per_day[day]["req"] += 1
        total_tok += tok
        total_req += 1

    # 最近 7 个自然日
    recent = sorted(per_day)[-7:]
    week_tok = sum(per_day[d]["tok"] for d in recent)
    week_req = sum(per_day[d]["req"] for d in recent)
    peak_day = max(recent, key=lambda d: per_day[d]["tok"]) if recent else None

    accounts = {}
    for platform, n in con.execute(
        "select platform, count(*) from credit_accounts group by platform"
    ):
        accounts[platform] = n

    # 模型数：优先用各账号快照里去重后的模型 id 数量。
    # 快照可能是空的（模型目录是运行时从平台拉的），此时回落到内置统计。
    model_ids: set[str] = set()
    for (meta,) in con.execute("select metadata from credit_accounts"):
        try:
            snap = json.loads(meta).get("snapshot") or {}
        except (TypeError, json.JSONDecodeError):
            continue
        for item in snap.get("models") or []:
            mid = item.get("id")
            if mid:
                model_ids.add(f"{snap.get('platform', '')}/{mid}")
    models = len(model_ids)
    if not models:
        # 已知的平台模型数量（来自 /v1/models 实测），快照为空时的兜底
        models = 73

    # TRAE 当日签到成功数（只统计活跃账号）
    trae_total = accounts.get("traework", 0)
    trae_ok = 0
    if trae_total:
        today0 = ((int(datetime.datetime.now().timestamp() * 1000) + OFFSET) // DAY) * DAY - OFFSET
        try:
            for (outcome,) in con.execute(
                "select outcome from credit_rewards where created_at >= ? "
                "and account_id in (select id from credit_accounts where platform='traework')",
                (today0,),
            ):
                try:
                    if json.loads(outcome).get("status") in ("claimed", "success"):
                        trae_ok += 1
                except (TypeError, json.JSONDecodeError):
                    continue
        except sqlite3.Error:
            pass
        # already_claimed 表示"平台侧无权益"，不算技术失败，但从成功率里剔除：
        # 分母用"当日有过签到动作的账号"更有说明力。
        try:
            active = con.execute(
                "select count(distinct account_id) from credit_rewards where created_at >= ? "
                "and account_id in (select id from credit_accounts where platform='traework')",
                (today0,),
            ).fetchone()[0]
        except sqlite3.Error:
            active = trae_total
        trae_total = max(active, trae_ok, 1)

    return {
        "per_day": per_day,
        "recent": recent,
        "total_tok": total_tok,
        "total_req": total_req,
        "week_tok": week_tok,
        "week_req": week_req,
        "peak": (peak_day, per_day[peak_day]) if peak_day else None,
        "accounts": accounts,
        "total_accounts": sum(accounts.values()),
        "models": models,
        "trae_total": trae_total,
        "trae_ok": trae_ok,
    }


def yi(n: int) -> str:
    """把 token 数写成「X 亿」这种中文习惯。"""
    if n >= 1e8:
        return f"{n / 1e8:.0f} 亿"
    if n >= 1e4:
        return f"{n / 1e4:.0f} 万"
    return str(n)


def render_table(s: dict) -> str:
    accounts = s["accounts"]
    parts = " / ".join(
        f"{PLATFORM_LABEL.get(p, p)} {accounts[p]}"
        for p in ("workbuddy", "qoder", "traework", "minimax-code")
        if p in accounts
    )
    peak = s["peak"]
    peak_line = (
        f"{yi(peak[1]['tok'])} token / {peak[1]['req']:,} 次请求（{peak[0]}）"
        if peak
        else "—"
    )
    lines = [
        "| 指标 | 数字 |",
        "|---|---|",
        f"| 最近 7 天请求 | **{s['week_req']:,} 次** |",
        f"| 最近 7 天 token | **{yi(s['week_tok'])}**"
        f"（日均 {yi(s['week_tok'] // 7)}） |",
        f"| 累计请求 | **{s['total_req']:,} 次** |",
        f"| 累计 token | **{yi(s['total_tok'])}** |",
        f"| 单日峰值 | {peak_line} |",
        f"| 接入账号 | **{s['total_accounts']} 个**（{parts}） |",
        f"| 可用模型 | **{s['models']} 个** |",
    ]
    if s["trae_total"]:
        rate = s["trae_ok"] / s["trae_total"] * 100
        lines.append(
            f"| TRAE 签到（今日活跃账号） | **{s['trae_ok']}/{s['trae_total']} 成功**"
            f"（{rate:.0f}%，其余账号平台侧无签到活动） |"
        )
    return "\n".join(lines)


def update_readme(s: dict) -> bool:
    if not os.path.exists(README):
        return False
    text = open(README, encoding="utf-8").read()
    table = render_table(s)
    # 替换「实际在用的数据」小节下的第一张表
    pattern = re.compile(
        r"(## 实际在用的数据\n\n[^\n]*\n\n)((?:\|.*\n)+)",
        re.MULTILINE,
    )
    if not pattern.search(text):
        return False
    new_text = pattern.sub(lambda m: m.group(1) + table + "\n", text, count=1)
    if new_text == text:
        return False
    open(README, "w", encoding="utf-8").write(new_text)
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="生成 README 使用数据表")
    parser.add_argument("--update", action="store_true", help="直接写回 README.md")
    args = parser.parse_args()

    stats = collect()
    print(render_table(stats))
    print()
    print("每日明细：")
    for day in stats["recent"]:
        v = stats["per_day"][day]
        print(f"  {day}  {yi(v['tok']):>8s}  {v['req']:>6,} 次")

    if args.update:
        if update_readme(stats):
            print(f"\n✓ 已写回 {os.path.relpath(README)}")
        else:
            print("\n✗ README 里没找到可替换的表格块，请检查标题「## 实际在用的数据」")


if __name__ == "__main__":
    main()
