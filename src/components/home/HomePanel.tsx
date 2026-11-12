import { useMemo, useState } from 'react';
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Coins,
  Gauge,
  LayoutDashboard,
  RefreshCw,
  Sparkles,
  Zap,
} from 'lucide-react';
import { ClaimDetail } from './ClaimDetail';
import { UsageTrend } from './UsageTrend';
import { useCreditsDashboard } from '@/hooks/useCredits';
import { useInstalledTools } from '@/contexts/InstalledToolsContext';
import { useAppVersion } from '@/hooks/useAppVersion';
import { Pressable } from '@/components/ui/Pressable';

const PLATFORM_LABELS: Record<string, string> = {
  workbuddy: 'WorkBuddy',
  traework: 'TRAE',
  qoder: 'Qoder',
  'minimax-code': 'MiniMax',
};

function platformName(id: string) {
  return PLATFORM_LABELS[id] ?? id;
}

function formatInt(n: number | null | undefined) {
  return (n ?? 0).toLocaleString('en-US');
}

/** Credits carry cents: 0.29, 34.5, 1,203.8. Keep two decimals, drop trailing zeros. */
function formatCredit(n: number | null | undefined) {
  const v = n ?? 0;
  if (v === 0) return '0';
  if (Math.abs(v) < 1) return v.toFixed(2);
  return v.toLocaleString('en-US', { maximumFractionDigits: 2 });
}

function formatTokens(n: number | null | undefined) {
  const v = n ?? 0;
  // Chinese units, not B/M/k. "2.49B" reads as "2.49 billion" to an English
  // reader but as "24.9亿" to a Chinese one — the same number, yet the label
  // invites exactly the wrong estimate. 亿/万 are unambiguous.
  if (v >= 1e8) return `${(v / 1e8).toFixed(2)}亿`;
  if (v >= 1e4) return `${(v / 1e4).toFixed(1)}万`;
  return String(Math.round(v));
}

function isSuccess(status: string) {
  return status === 'success' || status === 'ready' || status === 'ok' || status === 'granted';
}

function isClaimed(status: string) {
  return status === 'claimed' || status === 'already_claimed' || status === 'success';
}

function beijingDayKey(ms: number) {
  return new Date(ms + 8 * 3600 * 1000).toISOString().slice(0, 10);
}

function formatClock(ms: number) {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(d.getSeconds()).padStart(2, '0')}`;
}

interface StatCardProps {
  icon: React.ReactNode;
  label: string;
  value: string;
  /** Second figure shown beside `value`, for cards that must report two numbers. */
  secondary?: string;
  secondaryLabel?: string;
  note?: string;
  tone?: 'blue' | 'green' | 'amber' | 'slate';
}

function StatCard({ icon, label, value, secondary, secondaryLabel, note, tone = 'blue' }: StatCardProps) {
  const toneClass =
    tone === 'green'
      ? 'text-emerald-600 dark:text-emerald-400'
      : tone === 'amber'
        ? 'text-amber-600 dark:text-amber-400'
        : tone === 'slate'
          ? 'text-slate-700 dark:text-slate-200'
          : 'text-blue-600 dark:text-blue-400';
  return (
    <div className="glass-card flex min-w-0 flex-1 basis-40 flex-col gap-1 !rounded-xl px-3.5 py-3">
      <div className="flex items-center gap-1.5 text-[10.5px] font-medium text-slate-500 dark:text-slate-400">
        <span className="text-blue-500/70 dark:text-blue-400/70">{icon}</span>
        <span className="truncate">{label}</span>
      </div>
      <div className="flex items-baseline gap-2">
        <div className={`text-lg font-bold leading-tight ${toneClass}`}>{value}</div>
        {secondary ? (
          <div className="flex min-w-0 flex-col leading-tight">
            <span className={`truncate text-sm font-semibold ${toneClass}`}>{secondary}</span>
            {secondaryLabel && (
              <span className="truncate text-[9px] text-slate-400 dark:text-slate-500">
                {secondaryLabel}
              </span>
            )}
          </div>
        ) : null}
      </div>
      {note && <div className="truncate text-[9.5px] text-slate-400 dark:text-slate-500">{note}</div>}
    </div>
  );
}

export default function HomePanel() {
  const { data, isLoading, isError, refetch, isFetching } = useCreditsDashboard();
  const { installedAgents } = useInstalledTools();
  const appVersion = useAppVersion();
  // Collapsed by default. With 70+ accounts the expanded view is a wall of
  // rows that pushes the trend chart and platform table below the fold; the
  // one-line summary (「29/33 领到 · 2900 分」) already answers "how many
  // accounts, how much" at a glance. Anyone chasing a specific failure opens it.
  //
  // 2026-11-11 改成默认展开：用户反馈折叠态看不到每个平台到底领了几个号、
  // 失败原因是什么，每次都要多点一下。平台行的信息密度不高（一个平台一行），
  // 展开后不会把下面的平台汇总挤出屏幕，所以直接把摘要当默认视图。
  const [claimOpen, setClaimOpen] = useState(true);

  const stats = useMemo(() => {
    const requests = data?.requests ?? [];
    const accounts = data?.accounts ?? [];
    // Server-side rollup over the whole history. The `requests` / `rewards`
    // arrays are capped at 100 rows for the recent-activity lists, so deriving
    // "today" figures from them under-counts badly once a day has more events
    // than that (60+ accounts checking in already exceeds the cap).
    const usage = data?.usage;
    const todayKey = beijingDayKey(Date.now());

    const succeeded = usage
      ? Number(usage.todaySucceeded)
      : requests.filter((r) => isSuccess(r.status)).length;
    const todayRequests = usage ? Number(usage.todayRequests) : requests.filter((r) => beijingDayKey(r.createdAt) === todayKey).length;
    const inputTokens = usage ? Number(usage.todayPromptTokens) : requests.reduce((sum, r) => sum + (r.inputTokens ?? 0), 0);
    const outputTokens = usage ? Number(usage.todayCompletionTokens) : requests.reduce((sum, r) => sum + (r.outputTokens ?? 0), 0);
    const billableTokens = usage
      ? Number(usage.todayBillablePromptTokens) + Number(usage.todayCompletionTokens)
      : null;
    // Use the server-computed rate, which divides hits by prompt tokens on
    // records that actually reported a cache figure. Recomputing it here from
    // total billable/total prompt is what produced the wrong 19%: unmeasured
    // (pre-upgrade) rows count as full misses and swamp the real ~78%.
    const cacheHitRate = usage?.todayCacheHitRate != null
      ? Number(usage.todayCacheHitRate)
      : null;
    // How much of today's traffic that rate covers. Below 1.0 the rate is a
    // sample, not the whole day, and the UI says so.
    const cacheCoverage = usage?.todayCacheCoverage != null
      ? Number(usage.todayCacheCoverage)
      : null;

    const avgLatency = usage?.todayAvgLatencyMs != null
      ? Number(usage.todayAvgLatencyMs)
      : (() => {
          const latencies = requests.map((r) => r.elapsedMs).filter((v) => v > 0);
          return latencies.length ? Math.round(latencies.reduce((a, b) => a + b, 0) / latencies.length) : null;
        })();

    // Claimed credits come from the server rollup: it sums every claim in the
    // window, while the local `rewards` list only holds the newest 100 rows.
    const todayCredits = usage ? Number(usage.todayClaimedCredits) : 0;
    // 今天平台实际发出的资源包总额，不管是谁领的。用户装了第二个签到工具时
    // 这个数会明显大于 todayCredits，差额就是那个工具领的——见数值下面的注脚。
    const todayNewPacks = usage ? Number(usage.todayNewPackCredits) : 0;
    const otherClients = Math.max(0, todayNewPacks - todayCredits);
    const todayClaimed = usage ? Number(usage.todayClaimingAccounts) : 0;
    const todayPending = usage ? Number(usage.todayPendingClaims) : 0;

    const totalBalance = accounts.reduce((sum, a) => sum + (a.snapshot?.balance ?? 0), 0);
    const withBalance = accounts.filter((a) => a.snapshot?.balance != null).length;
    const claimableAccounts = accounts.filter((a) =>
      (a.snapshot?.rewards ?? []).some((r) => r.status === 'claimable'),
    ).length;

    return {
      accounts,
      requests,
      // Kept for the "recent claims" list, which is display-only. Totals must
      // come from `usage`, never from this capped array.
      rewards: data?.rewards ?? [],
      todayRequests,
      inputTokens,
      outputTokens,
      billableTokens,
      cacheHitRate,
      cacheCoverage,
      // What today actually consumed, cache accounted for. This is the number
      // that maps to the credit bill; `inputTokens + outputTokens` is what was
      // sent and reads far larger.
      todaySpentTokens: billableTokens ?? inputTokens + outputTokens,
      // Real spend reported by the upstream, not a token-derived estimate.
      todaySpentCredits: usage ? Number(usage.todaySpentCredits ?? 0) : 0,
      todayCreditCoverage: usage ? Number(usage.todayCreditCoverage ?? 0) : 0,
      monthSpentCredits: usage ? Number(usage.monthSpentCredits ?? 0) : 0,
      succeeded,
      failed: usage ? Number(usage.todayFailed) : requests.length - succeeded,
      avgLatency,
      todayCredits,
      todayNewPacks,
      otherClients,
      todayClaimed,
      todayPending,
      totalBalance,
      withBalance,
      claimableAccounts,
      // 这批数字的生成时间，用来在页面标「用量截至 HH:MM:SS」。
      sampledAt: usage ? Number(usage.sampledAt ?? 0) : 0,
      month: usage
        ? {
            prompt: Number(usage.monthPromptTokens),
            billable: Number(usage.monthBillablePromptTokens),
            completion: Number(usage.monthCompletionTokens),
            requests: Number(usage.monthRequests),
            claimed: Number(usage.monthClaimedCredits),
          }
        : null,
    };
  }, [data]);

  // One-line summary for the collapsed claim card.
  //
  // Deliberately counts accounts, not rows: 「55 个账号今日新领 · 合计 6400 分」
  // is the number a person wants at a glance. Per-platform bars live in the
  // expanded view; repeating them here would recreate the wall of text that
  // made collapsing worthwhile.
  const claimSummary = useMemo(() => {
    const claims = data?.usage?.claims ?? [];
    if (claims.length === 0) return '今日暂无领取记录';
    const accounts = claims.reduce((sum, p) => sum + p.accounts, 0);
    const claimed = claims.reduce((sum, p) => sum + p.claimed, 0);
    const already = claims.reduce((sum, p) => sum + p.alreadyClaimed, 0);
    const pending = claims.reduce((sum, p) => sum + p.pending, 0);
    const credits = claims.reduce((sum, p) => sum + p.credits, 0);
    const parts = [
      `${accounts} 个账号`,
      `${claimed} 个今日新领`,
      `${Math.round(credits)} 分`,
    ];
    if (already > 0) parts.push(`${already} 个此前已领`);
    if (pending > 0) parts.push(`${pending} 个待补领`);
    return parts.join(' · ');
  }, [data]);

  const platformRows = useMemo(() => {
    const map = new Map<
      string,
      {
        platform: string;
        accounts: number;
        balance: number;
        requests: number;
        tokens: number;
        rawTokens: number;
        cacheHitRate: number | null;
      }
    >();
    for (const account of stats.accounts) {
      const key = String(account.platform);
      const row =
        map.get(key) ??
        { platform: key, accounts: 0, balance: 0, requests: 0, tokens: 0, rawTokens: 0, cacheHitRate: null };
      row.accounts += 1;
      row.balance += account.snapshot?.balance ?? 0;
      map.set(key, row);
    }
    // Request counts and tokens come from the server-side rollup. Walking
    // `stats.requests` here would only count the newest 100 rows, which is why
    // this table used to show "100" for one platform and 0 for the rest.
    for (const p of data?.usage?.platforms ?? []) {
      const key = p.platform;
      const row =
        map.get(key) ??
        { platform: key, accounts: 0, balance: 0, requests: 0, tokens: 0, rawTokens: 0, cacheHitRate: null };
      row.requests = Number(p.requests ?? 0);
      // 与顶部卡片同口径：用 billable（去掉缓存命中）而不是 total（含缓存）。
      // 之前这里取 totalTokens，导致表格三个平台加起来 18.2 亿，而卡片只有
      // 8.5 亿，同一屏相差一倍多。缓存命中率高的平台（WorkBuddy 95%）
      // 尤其虚高。
      row.tokens = Number(p.billableTokens ?? p.totalTokens ?? 0);
      row.rawTokens = Number(p.totalTokens ?? 0);
      row.cacheHitRate =
        typeof p.cacheHitRate === 'number' ? Math.round(p.cacheHitRate * 1000) / 10 : null;
      map.set(key, row);
    }
    return [...map.values()].sort((a, b) => b.requests - a.requests);
  }, [stats, data]);

  const recentRequests = stats.requests.slice(0, 8);
  const recentRewards = stats.rewards.slice(0, 6);

  const gatewayRunning = data?.gateway.running ?? false;
  const gatewayUrl = data?.gateway.baseUrl ?? 'http://127.0.0.1:17860/v1';

  return (
    <div className="glass-app flex h-full flex-col overflow-hidden">
      {/* 头部 */}
      <div className="glass-header flex-shrink-0">
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="glass-kicker">
              <LayoutDashboard size={13} />
              OVERVIEW
            </div>
            <h2 className="mt-3 truncate text-display">总览</h2>
            <p className="mt-1.5 text-sm text-slate-500 dark:text-slate-400">
              账号积分、代理状态与调用统计一览
              {/* 标出数据时间。之前对账时对方拿十分钟前的旧值当结论，
                  有这个时间戳就不会再发生。 */}
              {stats.sampledAt ? (
                <span className="ml-2 text-[11px] text-slate-400 dark:text-slate-500">
                  用量截至{' '}
                  {new Date(stats.sampledAt).toLocaleTimeString('zh-CN', {
                    hour: '2-digit',
                    minute: '2-digit',
                    second: '2-digit',
                    hour12: false,
                  })}
                </span>
              ) : null}
            </p>
          </div>
          <div className="flex items-center gap-2">
            <span
              className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium ${
                gatewayRunning
                  ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                  : 'bg-slate-500/10 text-slate-500 dark:text-slate-400'
              }`}
            >
              <span
                className={`h-1.5 w-1.5 rounded-full ${gatewayRunning ? 'bg-emerald-500' : 'bg-slate-400'}`}
              />
              {gatewayRunning ? '代理运行中' : '代理未启动'}
            </span>
            <Pressable
              onClick={() => void refetch()}
              className="inline-flex items-center gap-1.5 rounded-xl border border-white/50 bg-white/60 px-3 py-1.5 text-xs font-medium text-slate-600 transition hover:bg-white/80 disabled:opacity-50 dark:border-white/10 dark:bg-slate-900/50 dark:text-slate-300"
              disabled={isFetching}
            >
              <RefreshCw size={13} className={isFetching ? 'animate-spin' : ''} />
              刷新
            </Pressable>
          </div>
        </div>
      </div>

      {/* 内容 */}
      <div className="min-h-0 flex-1 overflow-y-auto px-1 pb-6">
        {isError && (
          <div className="mb-4 rounded-xl border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-600 dark:border-red-900/40 dark:bg-red-950/30 dark:text-red-400">
            读取积分数据失败，请稍后重试。
          </div>
        )}

        {/* 概览卡片 */}
        <div className="mb-5 flex flex-wrap gap-2.5">
          <StatCard
            icon={<Coins size={12} />}
            label="积分总余额"
            value={formatInt(stats.totalBalance)}
            note={`${stats.withBalance} 个账号有余额`}
            tone="blue"
          />
          <StatCard
            icon={<Sparkles size={12} />}
            label="今日已领积分"
            value={formatInt(stats.todayCredits)}
            note={
              stats.otherClients > 0
                ? `另有 ${formatInt(stats.otherClients)} 分由其他客户端领到（平台今日共发出 ${formatInt(stats.todayNewPacks)} 分）`
                : stats.todayPending > 0
                  ? `${stats.todayClaimed} 个账号已领 · ${stats.todayPending} 待补领`
                  : `${stats.todayClaimed} 个账号已领 · 明细见下方`
            }
            tone={stats.todayPending > 0 ? 'amber' : 'green'}
          />
          <StatCard
            icon={<Gauge size={12} />}
            label="今日请求 / 消耗"
            value={formatInt(stats.todayRequests)}
            secondary={
              stats.todayCreditCoverage > 0
                ? `${formatCredit(stats.todaySpentCredits)} 分`
                : '—'
            }
            secondaryLabel="消耗积分"
            note={`${formatInt(stats.succeeded)} 成功 · ${formatInt(stats.failed)} 失败`}
            tone="slate"
          />
          {/*
            The headline is what was *sent*, and most of that is the prompt
            cache being re-read: every turn resends the whole conversation, so
            a long agent loop racks up hundreds of millions of input tokens for
            very little new work. Quoting that number under a label like
            "Token 用量" invites reading it as consumption, which is off by an
            order of magnitude — measured today: 2.56亿 sent, 1.58亿 billable,
            i.e. 38% of the headline was cache hits.

            So show the billable figure as the headline, and keep the sent
            figure beside it as context. The cache note explains the gap in the
            same breath instead of leaving the reader to wonder.
          */}
          <StatCard
            icon={<Activity size={12} />}
            label="Token 计费量（今日）"
            value={formatTokens(stats.billableTokens)}
            note={
              stats.cacheHitRate != null
                ? `发送 ${formatTokens(stats.inputTokens + stats.outputTokens)} · 缓存命中 ${Math.round(stats.cacheHitRate * 100)}%${
                    stats.cacheCoverage != null && stats.cacheCoverage < 0.995
                      ? `（覆盖 ${Math.round(stats.cacheCoverage * 100)}%）`
                      : ''
                  }`
                : `入 ${formatTokens(stats.inputTokens)} · 出 ${formatTokens(stats.outputTokens)}`
            }
            tone="slate"
          />
          <StatCard
            icon={<Zap size={12} />}
            label="平均延迟"
            value={stats.avgLatency != null ? `${formatInt(stats.avgLatency)}ms` : '—'}
            note={`成功 ${formatInt(stats.succeeded)} · 失败 ${formatInt(stats.failed)}`}
            tone={stats.failed > stats.succeeded ? 'amber' : 'green'}
          />
        </div>

        {/* Token 与调用趋势：紧跟概览卡片。
            顺序是刻意的——卡片只给「今天多少」，趋势回答「这一周/这一个月是
            怎么变化的」，两个问题连着看才有意义；放到页面底部就得来回滚动。 */}
        <UsageTrend className="mb-5" />

        {/* 今日领取详情：按平台 → 账号展开，方便定位需要手动补领的号。
            默认折叠，一行摘要足够看清「几个号领了多少」。 */}
        <div className="glass-card mb-5 !rounded-xl px-4 py-3">
          <button
            type="button"
            className="flex w-full flex-wrap items-center gap-2 text-left"
            onClick={() => setClaimOpen((v) => !v)}
          >
            {claimOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            <span className="text-xs font-semibold">今日领取详情</span>
            <span className="text-[11px] text-slate-500 dark:text-slate-400">
              {claimSummary}
            </span>
            <span className="ml-auto text-[11px] text-slate-400 dark:text-slate-500">
              {claimOpen ? '收起' : '展开'}
            </span>
          </button>
          {claimOpen ? <ClaimDetail claims={data?.usage?.claims} /> : null}
        </div>

        {/* 代理地址 */}
        <div className="glass-card mb-5 flex flex-wrap items-center justify-between gap-3 !rounded-xl px-4 py-3">
          <div className="min-w-0">
            <div className="text-[10.5px] font-medium text-slate-500 dark:text-slate-400">
              本地代理地址（本机 / 局域网）
            </div>
            <div className="mt-0.5 truncate font-mono text-sm text-slate-700 dark:text-slate-200">
              {gatewayUrl}
            </div>
          </div>
          <div className="flex items-center gap-4 text-right">
            <div>
              <div className="text-[10.5px] text-slate-500 dark:text-slate-400">已装 Agent</div>
              <div className="text-sm font-semibold text-slate-700 dark:text-slate-200">
                {installedAgents.length}
              </div>
            </div>
            <div>
              <div className="text-[10.5px] text-slate-500 dark:text-slate-400">版本</div>
              <div className="text-sm font-semibold text-emerald-600 dark:text-emerald-400">
                v{appVersion}
              </div>
            </div>
          </div>
        </div>

        {/* 平台汇总 */}
        <section className="glass-card mb-5 !rounded-xl p-4">
          <div className="mb-1 flex flex-wrap items-baseline justify-between gap-2">
            <h3 className="text-sm font-semibold text-slate-700 dark:text-slate-200">
              平台汇总
            </h3>
            {data?.usage ? (
              <span className="text-[10px] text-slate-400 dark:text-slate-500">
                请求与 Token 为今日累计 · 计费口径（不含缓存命中）
              </span>
            ) : null}
          </div>
          <p className="mb-3 text-[11px] text-slate-400 dark:text-slate-500">
            账号数与余额为当前值；「计费 Token」与顶部卡片同口径，已扣除缓存命中；
            「缓存」列是各平台上一次上报时的命中率
          </p>
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="text-slate-500 dark:text-slate-400">
                  <th className="border-b border-slate-200 py-2 pr-3 text-left font-medium dark:border-slate-700">
                    平台
                  </th>
                  <th className="border-b border-slate-200 py-2 pr-3 text-right font-medium dark:border-slate-700">
                    账号
                  </th>
                  <th className="border-b border-slate-200 py-2 pr-3 text-right font-medium dark:border-slate-700">
                    余额
                  </th>
                  <th className="border-b border-slate-200 py-2 pr-3 text-right font-medium dark:border-slate-700">
                    今日请求
                  </th>
                  <th className="border-b border-slate-200 py-2 pr-3 text-right font-medium dark:border-slate-700">
                    计费 Token
                  </th>
                  <th className="border-b border-slate-200 py-2 text-right font-medium dark:border-slate-700">
                    缓存
                  </th>
                </tr>
              </thead>
              <tbody>
                {platformRows.length === 0 && (
                  <tr>
                    <td colSpan={6} className="py-6 text-center text-slate-400 dark:text-slate-500">
                      {isLoading ? '正在加载…' : '暂无数据'}
                    </td>
                  </tr>
                )}
                {platformRows.map((row) => (
                  <tr
                    key={row.platform}
                    className="text-slate-700 transition hover:bg-slate-50 dark:text-slate-200 dark:hover:bg-slate-800/50"
                  >
                    <td className="border-b border-slate-100 py-2 pr-3 text-left font-medium dark:border-slate-800">
                      {platformName(row.platform)}
                    </td>
                    <td className="border-b border-slate-100 py-2 pr-3 text-right dark:border-slate-800">
                      {row.accounts}
                    </td>
                    <td className="border-b border-slate-100 py-2 pr-3 text-right dark:border-slate-800">
                      {formatInt(row.balance)}
                    </td>
                    <td className="border-b border-slate-100 py-2 pr-3 text-right dark:border-slate-800">
                      {formatInt(row.requests)}
                    </td>
                    <td
                      className="border-b border-slate-100 py-2 pr-3 text-right dark:border-slate-800"
                      title={
                        row.rawTokens
                          ? `含缓存的原始输入：${formatTokens(row.rawTokens)}`
                          : undefined
                      }
                    >
                      {formatTokens(row.tokens)}
                    </td>
                    <td className="border-b border-slate-100 py-2 text-right dark:border-slate-800">
                      {row.cacheHitRate === null ? (
                        <span className="text-slate-300 dark:text-slate-600">—</span>
                      ) : row.cacheHitRate === 0 ? (
                        <span className="text-amber-600 dark:text-amber-400">0%</span>
                      ) : (
                        <span className="text-emerald-600 dark:text-emerald-400">
                          {row.cacheHitRate}%
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        <div className="grid gap-5 lg:grid-cols-2">
          {/* 最近请求 */}
          <section className="glass-card !rounded-xl p-4">
            <h3 className="mb-1 text-sm font-semibold text-slate-700 dark:text-slate-200">
              最近请求
            </h3>
            <p className="mb-3 text-[11px] text-slate-400 dark:text-slate-500">
              最新 8 条调用记录
            </p>
            <div className="space-y-1.5">
              {recentRequests.length === 0 && (
                <div className="py-6 text-center text-xs text-slate-400 dark:text-slate-500">
                  {isLoading ? '正在加载…' : '暂无调用记录'}
                </div>
              )}
              {recentRequests.map((request) => (
                <div
                  key={request.id}
                  className="flex items-center justify-between gap-2 rounded-lg bg-slate-50/70 px-2.5 py-2 text-xs dark:bg-slate-800/40"
                >
                  <div className="min-w-0">
                    <div className="truncate font-medium text-slate-700 dark:text-slate-200">
                      {request.model}
                    </div>
                    <div className="truncate text-[10px] text-slate-400 dark:text-slate-500">
                      {platformName(String(request.platform))} · {formatClock(request.createdAt)}
                    </div>
                  </div>
                  <div className="flex flex-shrink-0 items-center gap-2">
                    <span className="text-[10px] text-slate-400 dark:text-slate-500">
                      {formatTokens((request.inputTokens ?? 0) + (request.outputTokens ?? 0))}
                    </span>
                    <span
                      className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
                        isSuccess(request.status)
                          ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                          : 'bg-red-500/10 text-red-600 dark:text-red-400'
                      }`}
                    >
                      {isSuccess(request.status) ? '成功' : '失败'}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </section>

          {/* 领取记录 */}
          <section className="glass-card !rounded-xl p-4">
            <h3 className="mb-1 text-sm font-semibold text-slate-700 dark:text-slate-200">
              最近领取
            </h3>
            <p className="mb-3 text-[11px] text-slate-400 dark:text-slate-500">
              最新 6 条签到记录（待补领会自动重试至 23:00）
            </p>
            <div className="space-y-1.5">
              {recentRewards.length === 0 && (
                <div className="py-6 text-center text-xs text-slate-400 dark:text-slate-500">
                  {isLoading ? '正在加载…' : '暂无领取记录'}
                </div>
              )}
              {recentRewards.map((reward) => {
                const claimed = isClaimed(reward.status);
                const pending = reward.status === 'pending';
                return (
                  <div
                    key={`${reward.accountId}-${reward.rewardId}-${reward.createdAt}`}
                    className="flex items-center justify-between gap-2 rounded-lg bg-slate-50/70 px-2.5 py-2 text-xs dark:bg-slate-800/40"
                  >
                    <div className="min-w-0">
                      <div className="truncate font-medium text-slate-700 dark:text-slate-200">
                        {reward.name}
                      </div>
                      <div className="truncate text-[10px] text-slate-400 dark:text-slate-500">
                        {formatClock(reward.createdAt)}
                      </div>
                    </div>
                    <span
                      className={`flex-shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium ${
                        pending
                          ? 'bg-amber-500/10 text-amber-600 dark:text-amber-400'
                          : claimed
                            ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                            : 'bg-slate-500/10 text-slate-500 dark:text-slate-400'
                      }`}
                    >
                      {pending ? '待补领' : claimed ? '已领' : reward.status}
                    </span>
                  </div>
                );
              })}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
