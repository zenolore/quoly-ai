import { useMemo, useState } from 'react';
import { useUsageDaily } from '@/hooks/useCredits';
import type { UsageDay } from '@/lib/api/credits';

/**
 * Token 与调用趋势 — stacked daily bars with a request-count line.
 *
 * Written as hand-rolled SVG rather than pulling in a chart library: the app
 * ships as a 50MB Tauri bundle and a charting dependency would add ~200KB of
 * JS for one screen with fixed, known requirements (stacked columns, one
 * overlay line, a hover card).
 *
 * Two axes on purpose. Bar height is tokens, line height is request count —
 * the whole reading of this chart is "did the bill grow because of traffic or
 * because of context bloat", which one axis cannot show. Tokens are plotted as
 * SENT (cache reads included) because that is what the upstream meters; the
 * card above uses the billable figure, hence the explicit legend wording.
 */

const RANGES = [
  { key: 'today', label: '今日', days: 1 },
  { key: '7d', label: '近 7 天', days: 7 },
  { key: '30d', label: '近 30 天', days: 30 },
  { key: 'month', label: '本月', days: 0 },
] as const;

type RangeKey = (typeof RANGES)[number]['key'];

const SEGMENTS = [
  { key: 'cachedTokens', label: '缓存读取', color: '#10b981' },
  { key: 'newPromptTokens', label: '新增输入', color: '#34d399' },
  { key: 'completionTokens', label: '输出', color: '#8b5cf6' },
  { key: 'unmeasuredPromptTokens', label: '未测缓存', color: '#f59e0b' },
] as const;

function formatTokens(n: number) {
  if (n >= 1e8) return `${(n / 1e8).toFixed(2)}亿`;
  if (n >= 1e4) return `${(n / 1e4).toFixed(0)}万`;
  return String(Math.round(n));
}

function formatInt(n: number) {
  return n.toLocaleString('en-US');
}

/** Pretty axis ticks: 0 … max in 1/2/5 × 10^n steps. */
function niceTicks(max: number, count = 4): number[] {
  if (max <= 0) return [0];
  const raw = max / count;
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const step = (norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10) * mag;
  const ticks: number[] = [];
  for (let v = 0; v <= max + step * 0.001; v += step) ticks.push(v);
  return ticks;
}

/** Days elapsed this month, so "本月" starts on the 1st rather than 30 days back. */
function monthDayCount() {
  const d = new Date();
  return d.getDate();
}

function dayLabel(date: string) {
  const parts = date.split('-');
  return `${Number(parts[1])}/${Number(parts[2])}`;
}

interface Props {
  /** Server-reported total for today, used to keep the 今日 bar honest. */
  className?: string;
}

export function UsageTrend({ className }: Props) {
  const [range, setRange] = useState<RangeKey>('30d');
  const days = range === 'month' ? monthDayCount() : RANGES.find((r) => r.key === range)!.days;
  const { data, isLoading, isFetching } = useUsageDaily(days);
  const [hover, setHover] = useState<number | null>(null);

  const rows: UsageDay[] = useMemo(() => data ?? [], [data]);

  // While a new range loads, react-query keeps the previous window on screen
  // (`placeholderData`), so a 30-day payload can briefly sit under the 今日 tab.
  // Trimming to the last `days` entries is what makes that safe: the series
  // always has the length the tab claims, and since the backend returns
  // chronological order, "last N" is always the right N. Without this the
  // 今日 tab would plot 30 bars under a one-bar label.
  const series = useMemo(() => {
    if (rows.length === 0) return [];
    if (rows.length <= days) return rows;
    return rows.slice(rows.length - days);
  }, [rows, days]);

  const totals = useMemo(() => {
    const sum = { cached: 0, newPrompt: 0, completion: 0, unmeasured: 0, requests: 0 };
    for (const d of series) {
      sum.cached += d.cachedTokens;
      sum.newPrompt += d.newPromptTokens;
      sum.completion += d.completionTokens;
      sum.unmeasured += d.unmeasuredPromptTokens;
      sum.requests += d.requests;
    }
    return sum;
  }, [series]);

  const stacked = series.map((d) => d.cachedTokens + d.newPromptTokens + d.completionTokens + d.unmeasuredPromptTokens);
  // 给峰值留 8% 头顶空间：niceTicks 取到的最大刻度可能正好等于峰值，柱子
  // 就会贴死绘图区上沿，峰值本身也读不出"还有多少余量"。
  const maxTokens = Math.max(1, ...stacked) * 1.08;
  const maxRequests = Math.max(1, ...series.map((d) => d.requests)) * 1.08;
  const tokenTicks = niceTicks(maxTokens);
  const requestTicks = niceTicks(maxRequests);
  const tokenMax = tokenTicks[tokenTicks.length - 1] || 1;
  const requestMax = requestTicks[requestTicks.length - 1] || 1;

  const W = 720;
  const H = 248;
  // top 从 12 提到 26：柱子按 tokenMax 归一化，峰值那天正好顶到绘图区上沿，
  // 而顶部刻度文字也画在同一条线上，两者叠在一起看着像"被裁掉了"。
  // 留出 26px 后最高柱子上方还有一圈空白，读数线也不会压在柱顶上。
  const PAD = { top: 26, right: 46, bottom: 24, left: 52 };
  const plotW = W - PAD.left - PAD.right;
  const plotH = H - PAD.top - PAD.bottom;

  // Cap the bar width rather than letting one bar fill the plot: under the
  // 今日 tab there is a single bar, and a 600px-wide block of colour reads as a
  // broken chart, not as "one day". 56px is wide enough to show the stack and
  // narrow enough to still look like a bar.
  const barGap = series.length > 20 ? 2 : series.length > 7 ? 4 : 12;
  const slot = plotW / Math.max(series.length, 1);
  const barW = Math.max(2, Math.min(56, slot - barGap));

  const xOf = (i: number) => PAD.left + slot * i + slot / 2;
  const yTokens = (v: number) => PAD.top + plotH - (v / tokenMax) * plotH;
  const yRequests = (v: number) => PAD.top + plotH - (v / requestMax) * plotH;

  const linePath = series
    .map((d, i) => `${i === 0 ? 'M' : 'L'}${xOf(i).toFixed(1)},${yRequests(d.requests).toFixed(1)}`)
    .join(' ');

  // Label every Nth date so 30 bars do not overlap.
  const labelStep = series.length <= 7 ? 1 : series.length <= 14 ? 2 : Math.ceil(series.length / 8);

  if (isLoading && series.length === 0) {
    return (
      <section className={`glass-card !rounded-xl p-4 ${className ?? ''}`}>
        <h3 className="text-sm font-semibold text-slate-700 dark:text-slate-200">Token 与调用趋势</h3>
        <div className="flex h-40 items-center justify-center text-xs text-slate-400 dark:text-slate-500">
          正在加载…
        </div>
      </section>
    );
  }

  if (series.every((d) => d.requests === 0)) {
    return (
      <section className={`glass-card !rounded-xl p-4 ${className ?? ''}`}>
        <h3 className="text-sm font-semibold text-slate-700 dark:text-slate-200">Token 与调用趋势</h3>
        <div className="flex h-40 items-center justify-center text-xs text-slate-400 dark:text-slate-500">
          所选区间没有调用记录
        </div>
      </section>
    );
  }

  const hoverDay = hover != null ? series[hover] : null;

  return (
    <section className={`glass-card !rounded-xl p-4 ${className ?? ''}`}>
      <div className="mb-1 flex flex-wrap items-center justify-between gap-2">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-slate-700 dark:text-slate-200">Token 与调用趋势</h3>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            彩色堆叠柱为每日 Token 构成，虚线为调用次数
            {isFetching ? <span className="ml-1.5 text-slate-300 dark:text-slate-600">更新中…</span> : null}
          </p>
        </div>
        <div className="flex items-center gap-1 rounded-lg bg-slate-500/5 p-0.5">
          {RANGES.map((r) => (
            <button
              key={r.key}
              type="button"
              onClick={() => setRange(r.key)}
              className={`rounded-md px-2.5 py-1 text-[11px] font-medium transition ${
                range === r.key
                  ? 'bg-white text-slate-700 shadow-sm dark:bg-slate-700 dark:text-slate-100'
                  : 'text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200'
              }`}
            >
              {r.label}
            </button>
          ))}
        </div>
      </div>

      {/* 图例 */}
      <div className="mb-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-slate-500 dark:text-slate-400">
        {SEGMENTS.map((s) => (
          <span key={s.key} className="flex items-center gap-1.5">
            <span className="h-2 w-2 rounded-sm" style={{ background: s.color }} />
            {s.label}
          </span>
        ))}
        <span className="flex items-center gap-1.5">
          <span className="inline-block h-0 w-3 border-t border-dashed border-slate-400" />
          调用次数
        </span>
      </div>

      <div className="relative">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          className="w-full"
          role="img"
          aria-label="每日 Token 构成与调用次数趋势"
          onMouseLeave={() => setHover(null)}
        >
          {/* 横向网格 + 左右两轴刻度。
              底部那条线（t=0）用实线当基线，其余虚线；刻度文字一律加 3px
              下移补偿，否则 9px 字的下缘会压在基线上，视觉上像被划掉了。 */}
          {tokenTicks.map((t, i) => {
            const y = yTokens(t);
            const rIdx = Math.round((t / tokenMax) * (requestTicks.length - 1));
            const rVal = requestTicks[Math.min(rIdx, requestTicks.length - 1)];
            return (
              <g key={`grid-${t}`}>
                <line
                  x1={PAD.left}
                  x2={W - PAD.right}
                  y1={y}
                  y2={y}
                  stroke="currentColor"
                  className={i === 0 ? 'text-slate-300 dark:text-slate-600' : 'text-slate-200 dark:text-slate-700'}
                  strokeWidth={1}
                  strokeDasharray={i === 0 ? undefined : '3 4'}
                />
                <text
                  x={PAD.left - 6}
                  y={i === 0 ? y - 3 : y + 3}
                  textAnchor="end"
                  className="fill-slate-400 text-[9px] dark:fill-slate-500"
                >
                  {formatTokens(t)}
                </text>
                <text
                  x={W - PAD.right + 6}
                  y={i === 0 ? y - 3 : y + 3}
                  textAnchor="start"
                  className="fill-slate-400 text-[9px] dark:fill-slate-500"
                >
                  {formatInt(rVal)}
                </text>
              </g>
            );
          })}

          {/* 堆叠柱 */}
          {series.map((d, i) => {
            const x = xOf(i) - barW / 2;
            let acc = 0;
            const parts = SEGMENTS.map((s) => {
              const v = d[s.key];
              if (v <= 0) return null;
              const h = (v / tokenMax) * plotH;
              const y = PAD.top + plotH - acc - h;
              acc += h;
              return (
                <rect
                  key={s.key}
                  x={x}
                  y={y}
                  width={barW}
                  height={Math.max(h, 0.6)}
                  fill={s.color}
                  rx={series.length <= 7 ? 2 : 1}
                />
              );
            });
            return (
              <g
                key={d.date}
                onMouseEnter={() => setHover(i)}
                className={hover === i ? 'opacity-100' : 'opacity-95'}
              >
                {parts}
                {/* 透明热区：柱子低于 1px 时也能触发 hover */}
                <rect
                  x={xOf(i) - slot / 2}
                  y={PAD.top}
                  width={slot}
                  height={plotH}
                  fill="transparent"
                />
              </g>
            );
          })}

          {/* 调用次数虚线 */}
          <path
            d={linePath}
            fill="none"
            stroke="currentColor"
            className="text-slate-400/80 dark:text-slate-500"
            strokeWidth={1.5}
            strokeDasharray="4 3"
          />

          {/* 日期刻度 */}
          {series.map((d, i) =>
            i % labelStep === 0 || i === series.length - 1 ? (
              <text
                key={`x-${d.date}`}
                x={xOf(i)}
                y={H - 6}
                textAnchor="middle"
                className="fill-slate-400 text-[9px] dark:fill-slate-500"
              >
                {dayLabel(d.date)}
              </text>
            ) : null,
          )}
        </svg>

        {/* 悬浮卡 */}
        {hoverDay ? (
          <div
            className="pointer-events-none absolute top-2 z-10 min-w-[168px] rounded-lg border border-slate-200 bg-white/95 px-2.5 py-2 text-[11px] shadow-lg dark:border-slate-700 dark:bg-slate-900/95"
            style={{
              // Anchor to the hovered bar's real x position (as a fraction of
              // the SVG viewBox), then keep the card on screen. Using
              // `index / count` instead drifts left whenever bars are not
              // evenly spread across the full width — the single-bar 今日 view
              // is the worst case.
              left: `${Math.min(72, Math.max(2, (xOf(hover!) / W) * 100))}%`,
            }}
          >
            <div className="mb-1 font-semibold text-slate-700 dark:text-slate-200">
              {hoverDay.date}
            </div>
            <div className="mb-1 text-slate-500 dark:text-slate-400">
              Token 总量{' '}
              <span className="font-semibold text-slate-700 dark:text-slate-200">
                {formatInt(
                  hoverDay.cachedTokens +
                    hoverDay.newPromptTokens +
                    hoverDay.completionTokens +
                    hoverDay.unmeasuredPromptTokens,
                )}
              </span>
            </div>
            {SEGMENTS.map((s) => {
              const v = hoverDay[s.key];
              const total =
                hoverDay.cachedTokens +
                hoverDay.newPromptTokens +
                hoverDay.completionTokens +
                hoverDay.unmeasuredPromptTokens;
              return (
                <div key={s.key} className="flex items-center justify-between gap-3">
                  <span className="flex items-center gap-1.5 text-slate-500 dark:text-slate-400">
                    <span className="h-1.5 w-1.5 rounded-sm" style={{ background: s.color }} />
                    {s.label}
                  </span>
                  <span className="text-slate-600 dark:text-slate-300">
                    {formatInt(v)}
                    <span className="ml-1 text-slate-400 dark:text-slate-500">
                      ({total ? ((v / total) * 100).toFixed(1) : '0.0'}%)
                    </span>
                  </span>
                </div>
              );
            })}
            <div className="mt-1 flex items-center justify-between gap-3 border-t border-slate-100 pt-1 dark:border-slate-800">
              <span className="text-slate-500 dark:text-slate-400">调用次数</span>
              <span className="font-semibold text-slate-700 dark:text-slate-200">
                {formatInt(hoverDay.requests)}
              </span>
            </div>
            {hoverDay.failed > 0 ? (
              <div className="flex items-center justify-between gap-3">
                <span className="text-slate-500 dark:text-slate-400">其中失败</span>
                <span className="text-rose-600 dark:text-rose-400">{formatInt(hoverDay.failed)}</span>
              </div>
            ) : null}
            {hoverDay.avgLatencyMs != null ? (
              <div className="flex items-center justify-between gap-3">
                <span className="text-slate-500 dark:text-slate-400">平均延迟</span>
                <span className="text-slate-600 dark:text-slate-300">
                  {formatInt(hoverDay.avgLatencyMs)}ms
                </span>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-[11px] text-slate-400 dark:text-slate-500">
        <span>
          {range === 'today' ? '今日' : `近 ${series.length} 天`}合计{' '}
          <span className="font-medium text-slate-600 dark:text-slate-300">
            {formatTokens(totals.cached + totals.newPrompt + totals.completion + totals.unmeasured)}
          </span>{' '}
          Token ·{' '}
          <span className="font-medium text-slate-600 dark:text-slate-300">
            {formatInt(totals.requests)}
          </span>{' '}
          次调用
        </span>
        <span>左轴：Token · 右轴：调用次数（含缓存，与顶部卡片口径不同）</span>
      </div>
    </section>
  );
}
