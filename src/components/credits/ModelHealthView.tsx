import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity, AlertTriangle, CheckCircle2, Clock, Loader2, PauseCircle, RefreshCw, ShieldAlert,
} from "lucide-react";
import { Pressable } from "@/components/ui/Pressable";
import { creditsApi, type ModelHealthReport } from "@/lib/api/credits";

/**
 * Live model health board.
 *
 * Shows, per platform and per model, how many calls actually succeeded, how
 * fast the answers come back, and which routes are currently quarantined. The
 * proxy records this passively from real traffic, so the numbers reflect what
 * the user's own workload experienced rather than an artificial probe.
 */

const REFRESH_MS = 5000;

const platformLabel: Record<string, string> = {
  workbuddy: "WorkBuddy",
  traework: "TRAE",
  qoder: "Qoder",
  "minimax-code": "MiniMax Code",
};

type Health = "good" | "warn" | "bad" | "idle";

function classify(row: ModelHealthReport): Health {
  if (row.quarantinedMs) return "bad";
  // Judge by the same figure shown on screen, so the badge can never contradict
  // the numbers beside it.
  const rate = row.successRate ?? row.recentRate;
  if (rate == null) return "idle";
  if (rate >= 0.9) return "good";
  if (rate >= 0.6) return "warn";
  return "bad";
}

const tone: Record<Health, { text: string; bg: string; border: string; label: string }> = {
  good: {
    text: "text-emerald-600 dark:text-emerald-400",
    bg: "bg-emerald-500",
    border: "border-emerald-500/30",
    label: "正常",
  },
  warn: {
    text: "text-amber-600 dark:text-amber-400",
    bg: "bg-amber-500",
    border: "border-amber-500/30",
    label: "不稳",
  },
  bad: {
    text: "text-rose-600 dark:text-rose-400",
    bg: "bg-rose-500",
    border: "border-rose-500/30",
    label: "异常",
  },
  idle: {
    text: "text-slate-500 dark:text-slate-400",
    bg: "bg-slate-400",
    border: "border-slate-400/30",
    label: "未使用",
  },
};

const pct = (value: number | null) =>
  value == null ? "—" : `${Math.round(value * 100)}%`;

const latency = (ms: number | null) => {
  if (ms == null) return "—";
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
};

const quarantine = (ms: number | null) => {
  if (!ms) return null;
  const sec = Math.ceil(ms / 1000);
  return sec >= 60 ? `${Math.ceil(sec / 60)} 分钟` : `${sec} 秒`;
};

/**
 * One platform's model table.
 *
 * Collapsed to the busiest `VISIBLE` rows by default: a platform can expose
 * dozens of routes (many of them one-off tries), and expanding all of them makes
 * the page unreadable. The rest stay one click away.
 */
const VISIBLE = 8;

function PlatformSection({ platform, list }: { platform: string; list: ModelHealthReport[] }) {
  const [expanded, setExpanded] = useState(false);
  const hidden = list.length - VISIBLE;
  const shown = expanded ? list : list.slice(0, VISIBLE);
  // Worth surfacing the count of genuinely broken routes on the header.
  const failing = list.filter((r) => (r.successRate ?? 1) < 0.9).length;

  return (
    <div className="glass-card overflow-hidden">
      <div className="flex items-center justify-between border-b border-slate-200/70 px-4 py-3 dark:border-slate-700/60">
        <div className="flex items-center gap-2">
          <div className="text-sm font-semibold">{platformLabel[platform] ?? platform}</div>
          {failing > 0 ? (
            <span className="rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
              {failing} 个成功率偏低
            </span>
          ) : null}
        </div>
        <div className="text-[11px] text-slate-500 dark:text-slate-400">{list.length} 个模型</div>
      </div>
      <div className="divide-y divide-slate-200/60 dark:divide-slate-700/50">
        {shown.map((row) => {
          const state = classify(row);
          const colors = tone[state];
          const wait = quarantine(row.quarantinedMs);
          return (
            <div className="flex flex-wrap items-center gap-3 px-4 py-3" key={row.route}>
              <span className={`h-2 w-2 shrink-0 rounded-full ${colors.bg}`} aria-hidden />
              <div className="min-w-0 flex-1">
                <div className="truncate text-xs font-medium">{row.model}</div>
                <div className="mt-0.5 truncate text-[11px] text-slate-500 dark:text-slate-400">
                  {row.route}
                </div>
              </div>
              <div className="flex items-center gap-4 text-[11px]">
                <div className="text-right">
                  <div className="text-slate-500 dark:text-slate-400">成功率</div>
                  <div className={`font-semibold ${colors.text}`}>
                    {pct(row.successRate ?? row.recentRate)}
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-slate-500 dark:text-slate-400">平均延迟</div>
                  <div className="font-semibold">{latency(row.avgLatencyMs)}</div>
                </div>
                <div className="text-right">
                  <div className="text-slate-500 dark:text-slate-400">
                    成功/失败{row.totalCancelled > 0 ? "/中断" : ""}
                  </div>
                  <div className="font-semibold" title="中断为客户端主动取消或超时，不计入成功率">
                    <span className="text-emerald-600 dark:text-emerald-400">{row.totalOk}</span>
                    <span className="text-slate-400"> / </span>
                    <span
                      className={
                        row.totalFail > 0
                          ? "text-rose-600 dark:text-rose-400"
                          : "text-slate-400"
                      }
                    >
                      {row.totalFail}
                    </span>
                    {row.totalCancelled > 0 ? (
                      <>
                        <span className="text-slate-400"> / </span>
                        <span className="text-slate-400">{row.totalCancelled}</span>
                      </>
                    ) : null}
                  </div>
                </div>
                <div
                  className={`flex items-center gap-1 rounded-full border px-2 py-0.5 ${colors.border} ${colors.text}`}
                >
                  {state === "good" ? (
                    <CheckCircle2 size={11} />
                  ) : state === "idle" ? (
                    <Clock size={11} />
                  ) : state === "warn" ? (
                    <AlertTriangle size={11} />
                  ) : (
                    <ShieldAlert size={11} />
                  )}
                  {wait ? `隔离 ${wait}` : colors.label}
                </div>
              </div>
              {row.lastError ? (
                <div
                  className="w-full truncate text-[11px] text-rose-600/80 dark:text-rose-400/80"
                  title={row.lastError}
                >
                  最近错误：{row.lastError}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
      {hidden > 0 ? (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="w-full border-t border-slate-200/70 px-4 py-2 text-[11px] text-slate-500 transition hover:bg-slate-50/60 dark:border-slate-700/60 dark:text-slate-400 dark:hover:bg-slate-800/40"
        >
          {expanded ? "收起" : `展开其余 ${hidden} 个模型`}
        </button>
      ) : null}
    </div>
  );
}

export function ModelHealthView({ proxyRunning }: { proxyRunning: boolean }) {
  const [rows, setRows] = useState<ModelHealthReport[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      const next = await creditsApi.modelHealth();
      setRows(next);
      setError(null);
      setUpdatedAt(Date.now());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [load]);

  // Group by platform so the user can answer "which platform is best for this
  // model" at a glance. Inside a group, order by usage volume: with 60+ routes
  // in the table, sorting purely by success rate buries the two models that
  // handle everything under a long tail of rarely-used names.
  const grouped = useMemo(() => {
    const map = new Map<string, ModelHealthReport[]>();
    for (const row of rows) {
      const list = map.get(row.platform) ?? [];
      list.push(row);
      map.set(row.platform, list);
    }
    for (const list of map.values()) {
      list.sort(
        (a, b) =>
          b.totalOk + b.totalFail - (a.totalOk + a.totalFail) ||
          a.route.localeCompare(b.route),
      );
    }
    // Platforms with the most traffic first.
    return [...map.entries()].sort((a, b) => {
      const sum = (l: ModelHealthReport[]) =>
        l.reduce((s, r) => s + r.totalOk + r.totalFail, 0);
      return sum(b[1]) - sum(a[1]);
    });
  }, [rows]);

  const summary = useMemo(() => {
    // Denominator is ok + fail only. Cancelled requests are client-side aborts:
    // counting them either way would misreport how reliable the model is.
    const total = rows.reduce((sum, r) => sum + r.totalOk + r.totalFail, 0);
    const ok = rows.reduce((sum, r) => sum + r.totalOk, 0);
    const cancelled = rows.reduce((sum, r) => sum + (r.totalCancelled ?? 0), 0);
    const quarantined = rows.filter((r) => r.quarantinedMs).length;
    return { total, ok, cancelled, quarantined, rate: total ? ok / total : null };
  }, [rows]);

  if (!proxyRunning) {
    return (
      <div className="glass-card p-6">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <PauseCircle size={16} />代理未运行
        </div>
        <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
          模型健康数据来自真实调用记录。请先在「本地代理」页开启代理，之后这里会显示每个平台、每个模型的实际成功率。
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* 概览 */}
      <div className="glass-card p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-sm font-semibold">
            <Activity size={16} />模型健康总览
          </div>
          <div className="flex items-center gap-3 text-xs text-slate-500 dark:text-slate-400">
            {updatedAt ? <span>更新于 {new Date(updatedAt).toLocaleTimeString("zh-CN")}</span> : null}
            <Pressable
              type="button"
              className="glass-secondary-button min-h-7 px-3 py-1 text-xs"
              onClick={() => { setLoading(true); void load(); }}
              disabled={loading}
            >
              <RefreshCw size={12} className={loading ? "animate-spin" : ""} />刷新
            </Pressable>
          </div>
        </div>
        <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div className="rounded-xl border border-slate-200/70 p-3 dark:border-slate-700/60">
            <div className="text-[11px] text-slate-500 dark:text-slate-400">监控模型数</div>
            <div className="mt-1 text-xl font-semibold">{rows.length}</div>
          </div>
          <div className="rounded-xl border border-slate-200/70 p-3 dark:border-slate-700/60">
            <div className="text-[11px] text-slate-500 dark:text-slate-400">累计调用</div>
            <div className="mt-1 text-xl font-semibold">{summary.total + summary.cancelled}</div>
            {summary.cancelled > 0 ? (
              <div className="mt-0.5 text-[10px] text-slate-400 dark:text-slate-500">
                含 {summary.cancelled} 次中断
              </div>
            ) : null}
          </div>
          <div className="rounded-xl border border-slate-200/70 p-3 dark:border-slate-700/60">
            <div className="text-[11px] text-slate-500 dark:text-slate-400">总体成功率</div>
            <div className={`mt-1 text-xl font-semibold ${summary.rate != null && summary.rate >= 0.9 ? "text-emerald-600 dark:text-emerald-400" : summary.rate != null && summary.rate >= 0.6 ? "text-amber-600 dark:text-amber-400" : "text-rose-600 dark:text-rose-400"}`}>
              {pct(summary.rate)}
            </div>
          </div>
          <div className="rounded-xl border border-slate-200/70 p-3 dark:border-slate-700/60">
            <div className="text-[11px] text-slate-500 dark:text-slate-400">隔离中</div>
            <div className={`mt-1 text-xl font-semibold ${summary.quarantined > 0 ? "text-rose-600 dark:text-rose-400" : ""}`}>
              {summary.quarantined}
            </div>
          </div>
        </div>
      </div>

      {error ? (
        <div className="glass-card p-4">
          <p role="alert" className="break-words text-xs text-rose-600 dark:text-rose-400">{error}</p>
        </div>
      ) : null}

      {loading && rows.length === 0 ? (
        <div className="flex min-h-[160px] items-center justify-center gap-3 text-sm text-slate-500">
          <Loader2 size={18} className="animate-spin" />正在读取模型健康数据…
        </div>
      ) : rows.length === 0 ? (
        <div className="glass-card p-6 text-center">
          <div className="text-sm font-semibold">暂无调用记录</div>
          <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
            代理已就绪，但还没有模型被调用过。发起一次对话后，这里会立即出现对应的健康数据。
          </p>
        </div>
      ) : (
        grouped.map(([platform, list]) => (
          <PlatformSection key={platform} platform={platform} list={list} />
        ))
      )}
    </div>
  );
}
