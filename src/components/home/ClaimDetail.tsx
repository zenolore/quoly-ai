import { useState } from "react";
import { AlertTriangle, ChevronDown, ChevronRight, CircleSlash, Clock, CheckCircle2, MinusCircle, Server } from "lucide-react";
import type { ClaimAccountRow, PlatformClaims } from "@/lib/api/credits";

/**
 * Today's credit-claim breakdown, per platform and per account.
 *
 * The overview card only shows a single total, which cannot answer the questions
 * that actually matter: which platform is failing, and which specific account
 * needs a manual retry. This expands to that level.
 */

const platformLabel: Record<string, string> = {
  workbuddy: "WorkBuddy",
  traework: "TRAE",
  qoder: "Qoder",
  "minimax-code": "MiniMax Code",
  doubao: "豆包",
};

const statusMeta: Record<string, { label: string; text: string; bg: string }> = {
  claimed: {
    label: "已领取",
    text: "text-emerald-600 dark:text-emerald-400",
    bg: "bg-emerald-500",
  },
  already_claimed: {
    label: "今日已领过",
    text: "text-sky-600 dark:text-sky-400",
    bg: "bg-sky-500",
  },
  pending: {
    label: "待确认",
    text: "text-amber-600 dark:text-amber-400",
    bg: "bg-amber-500",
  },
  unavailable: {
    label: "无可领权益",
    text: "text-slate-500 dark:text-slate-400",
    bg: "bg-slate-400",
  },
  failed: {
    label: "领取失败",
    text: "text-rose-600 dark:text-rose-400",
    bg: "bg-rose-500",
  },
  none: {
    label: "未尝试",
    text: "text-slate-400 dark:text-slate-500",
    bg: "bg-slate-300",
  },
  // 转发型平台（豆包）网关内部的号：签到由网关自己的调度器跑，
  // 这里只做展示，不参与"领到/没领到"的统计。
  managed: {
    label: "网关维护",
    text: "text-violet-600 dark:text-violet-400",
    bg: "bg-violet-400",
  },
};

const statusIcon = (status: string) => {
  switch (status) {
    case "claimed":
      return <CheckCircle2 size={12} className="text-emerald-600 dark:text-emerald-400" />;
    case "already_claimed":
      return <CheckCircle2 size={12} className="text-sky-600 dark:text-sky-400" />;
    case "pending":
      return <Clock size={12} className="text-amber-600 dark:text-amber-400" />;
    case "unavailable":
      return <CircleSlash size={12} className="text-slate-400" />;
    case "failed":
      return <AlertTriangle size={12} className="text-rose-600 dark:text-rose-400" />;
    case "managed":
      return <Server size={12} className="text-violet-600 dark:text-violet-400" />;
    default:
      return <MinusCircle size={12} className="text-slate-300 dark:text-slate-600" />;
  }
};

const pct = (v: number | null) => (v == null ? "—" : `${Math.round(v * 100)}%`);

function rateTone(rate: number | null): string {
  if (rate == null) return "text-slate-500 dark:text-slate-400";
  if (rate >= 0.99) return "text-emerald-600 dark:text-emerald-400";
  if (rate >= 0.8) return "text-amber-600 dark:text-amber-400";
  return "text-rose-600 dark:text-rose-400";
}

function PlatformBlock({ data }: { data: PlatformClaims }) {
  // 默认全部折叠。之前按「有没有未处理的账号」决定初始展开状态，结果 TRAE
  // 那种大面积「无权益」的账号会让它自动展开成一张长列表，把下面三个平台
  // 全顶到屏幕外。折叠着看四个平台一行一个，点开才是明细，这样更好扫。
  const [open, setOpen] = useState(false);

  const segments = [
    { key: "claimed", count: data.claimed, label: "已领" },
    { key: "already_claimed", count: data.alreadyClaimed, label: "已领过" },
    { key: "pending", count: data.pending, label: "待确认" },
    { key: "unavailable", count: data.unavailable, label: "无权益" },
    { key: "missing", count: data.missing, label: "未尝试" },
  ].filter((s) => s.count > 0);

  return (
    <div className="rounded-xl border border-slate-200/70 dark:border-slate-700/60">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full flex-wrap items-center gap-2 px-3 py-2 text-left transition hover:bg-slate-50/60 dark:hover:bg-slate-800/40"
      >
        {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        <span className="text-xs font-semibold">{platformLabel[data.platform] ?? data.platform}</span>
        <span className={`text-[11px] font-semibold ${rateTone(data.successRate)}`}>
          {pct(data.successRate)}
        </span>
        <span className="text-[11px] text-slate-500 dark:text-slate-400">
          {/* 转发型平台（豆包）没有"领取"这个动作：一个 Quoly AI 账号其实
              是一个本地网关，网关里养着好几个号，签到由网关自己跑。这里
              只把那几个号列出来，别拿它去比成功率。 */}
          {data.gateway
            ? <>{data.rows.length} 个号 · 签到由网关自己跑，不在这里领</>
            : data.credited > data.credits + 0.5
              ? <>{data.claimed}/{data.accounts} 个账号 · 本程序新领 {Math.round(data.credits)} 分 · 平台确认到账 {Math.round(data.credited)} 分（含此前其他途径）</>
              : <>{data.claimed}/{data.accounts} 个账号领到 · 共 {Math.round(data.credits)} 分</>}
        </span>
        <span className="ml-auto flex items-center gap-1.5">
          {segments.map((s) => (
            <span
              key={s.key}
              className="flex items-center gap-1 text-[11px] text-slate-500 dark:text-slate-400"
              title={`${s.label} ${s.count}`}
            >
              <span className={`h-1.5 w-1.5 rounded-full ${statusMeta[s.key]?.bg ?? "bg-slate-300"}`} />
              {s.label} {s.count}
            </span>
          ))}
        </span>
      </button>

      {open ? (
        <div className="divide-y divide-slate-200/60 border-t border-slate-200/70 dark:divide-slate-700/50 dark:border-slate-700/60">
          {data.rows.map((row) => (
            <AccountRow key={row.accountId} row={row} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function AccountRow({ row }: { row: ClaimAccountRow }) {
  const meta = statusMeta[row.status] ?? statusMeta.none;
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-1.5">
      {statusIcon(row.status)}
      <span className="min-w-0 flex-1 truncate text-[11px]">{row.name}</span>
      <span className={`text-[11px] font-medium ${meta.text}`}>{meta.label}</span>
      {row.amount > 0 ? (
        <span className="text-[11px] font-semibold text-emerald-600 dark:text-emerald-400">
          +{Math.round(row.amount)}
        </span>
      ) : null}
      {row.message ? (
        <span
          className="basis-full truncate pl-4 text-[10px] text-slate-400 dark:text-slate-500"
          title={row.message}
        >
          {row.message}
        </span>
      ) : null}
    </div>
  );
}

export function ClaimDetail({ claims }: { claims?: PlatformClaims[] }) {
  if (!claims || claims.length === 0) {
    return (
      <div className="mt-2 rounded-xl border border-slate-200/70 px-3 py-2 text-[11px] text-slate-500 dark:border-slate-700/60 dark:text-slate-400">
        今日暂无领取记录
      </div>
    );
  }

  // 网关型平台不进合计：它的"账号"是一个网关，不是可领取账号，算进
  // 「共 N 个账号」和「合计 X 分」只会让两个总数对不上明细。
  const counted = claims.filter((p) => !p.gateway);
  const totalAccounts = counted.reduce((sum, p) => sum + p.accounts, 0);
  const totalClaimed = counted.reduce((sum, p) => sum + p.claimed, 0);
  const totalCredits = counted.reduce((sum, p) => sum + p.credits, 0);

  return (
    <div className="mt-2 space-y-1.5">
      <div className="flex flex-wrap items-center gap-2 text-[11px] text-slate-500 dark:text-slate-400">
        <span>
          共 {totalAccounts} 个账号 · {totalClaimed} 个今日新领 · 合计 {Math.round(totalCredits)} 分
        </span>
        <span className="text-slate-300 dark:text-slate-600">|</span>
        <span>「今日已领过」表示此前已经领到，无需处理</span>
      </div>
      {claims.map((p) => (
        <PlatformBlock key={p.platform} data={p} />
      ))}
    </div>
  );
}
