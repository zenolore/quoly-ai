import { useState } from "react";
import { AlertTriangle, ChevronDown, ChevronRight, CircleSlash, Clock, CheckCircle2, MinusCircle } from "lucide-react";
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
  // Open by default when something needs attention.
  const needsAction = data.claimed < data.accounts - data.alreadyClaimed;
  const [open, setOpen] = useState(needsAction);

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
          {data.claimed}/{data.accounts} 个账号领到 · 共 {Math.round(data.credits)} 分
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

  const totalAccounts = claims.reduce((sum, p) => sum + p.accounts, 0);
  const totalClaimed = claims.reduce((sum, p) => sum + p.claimed, 0);
  const totalCredits = claims.reduce((sum, p) => sum + p.credits, 0);

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
