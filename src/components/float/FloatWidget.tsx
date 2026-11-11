import { useEffect, useMemo, useRef, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useCreditsDashboard } from '@/hooks/useCredits';
import { shellApi } from '@/lib/api/shell';

/**
 * 桌面悬浮窗 —— 竖向圆角条。
 *
 * 形态：一块竖长圆角矩形，四组数字竖排居中，没有标题栏、没有常驻按钮。
 * 之所以收成竖条：横版卡片即使再紧凑，也会在桌面角落占掉一块矩形面积；
 * 竖条窄，可以贴屏幕边缘当「数字侧栏」，扫一眼就有数。
 *
 * 交互上的取舍——待机时不摆任何按钮：
 *   - 按住任意位置拖动（Tauri 原生 startDragging，跨屏不抖）
 *   - 双击打开主界面
 *   - 悬停时右上角浮出一个小 ×
 *   - 右键也隐藏（窄条上比找 × 快）
 */

function formatInt(n: number | null | undefined) {
  return (n ?? 0).toLocaleString('en-US');
}

function formatTokens(n: number | null | undefined) {
  const v = n ?? 0;
  if (v >= 1e8) return `${(v / 1e8).toFixed(2)}亿`;
  if (v >= 1e4) return `${(v / 1e4).toFixed(1)}万`;
  return String(Math.round(v));
}

function formatCredit(n: number | null | undefined) {
  const v = n ?? 0;
  if (v === 0) return '0';
  if (Math.abs(v) < 1) return v.toFixed(2);
  return v.toLocaleString('en-US', { maximumFractionDigits: 1 });
}

export function FloatWidget() {
  const { data, isLoading } = useCreditsDashboard();
  const [dragging, setDragging] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [pulse, setPulse] = useState(false);
  const pulseTimer = useRef<number | null>(null);

  const stats = useMemo(() => {
    const usage = data?.usage;
    const requests = data?.requests ?? [];
    return {
      running: data?.gateway.running ?? false,
      requests: usage ? Number(usage.todayRequests) : requests.length,
      failed: usage ? Number(usage.todayFailed) : 0,
      spent: usage ? Number(usage.todaySpentCredits ?? 0) : 0,
      claimed: usage ? Number(usage.todayClaimedCredits) : 0,
      claiming: usage ? Number(usage.todayClaimingAccounts) : 0,
      tokens: usage
        ? Number(usage.todayBillablePromptTokens) + Number(usage.todayCompletionTokens)
        : 0,
      sampledAt: usage ? Number(usage.sampledAt ?? 0) : 0,
    };
  }, [data]);

  // 数据刷新时给个极轻的边框高亮，用来分辨「这是新数还是旧数」。
  useEffect(() => {
    if (!stats.sampledAt) return;
    setPulse(true);
    if (pulseTimer.current) window.clearTimeout(pulseTimer.current);
    pulseTimer.current = window.setTimeout(() => setPulse(false), 800);
    return () => {
      if (pulseTimer.current) window.clearTimeout(pulseTimer.current);
    };
  }, [stats.sampledAt]);

  const startDrag = async (event: React.PointerEvent) => {
    if (event.button !== 0) return;
    // 按在 × 上时不启动拖动，否则一按就跟着走、根本点不中
    if ((event.target as HTMLElement).closest('[data-no-drag]')) return;
    setDragging(true);
    try {
      await getCurrentWindow().startDragging();
      // Promise 在拖动结束后 resolve，这时坐标才是最终的；
      // 跟着 move 事件写会把磁盘刷爆。
      await shellApi.rememberFloatPosition();
    } catch {
      // 拖动失败不影响读数
    } finally {
      setDragging(false);
    }
  };

  const hide = () => void shellApi.hideFloat();

  return (
    <div
      className="h-screen w-screen bg-transparent p-[6px]"
      onDoubleClick={() => void shellApi.openMainWindow()}
      onContextMenu={(event) => {
        event.preventDefault();
        hide();
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div
        onPointerDown={startDrag}
        className={`relative flex h-full w-full cursor-grab flex-col items-center overflow-hidden rounded-[26px] border px-[10px] pb-[14px] pt-[14px] transition-[box-shadow,border-color,transform] duration-300 ${
          dragging ? 'scale-[0.985] cursor-grabbing' : ''
        } ${
          pulse
            ? 'border-sky-300/90 shadow-[0_14px_34px_-14px_rgba(14,165,233,0.45),inset_0_1px_0_rgba(255,255,255,0.95)]'
            : 'border-white/90 shadow-[0_14px_34px_-16px_rgba(15,23,42,0.30),inset_0_1px_0_rgba(255,255,255,0.95)]'
        }`}
        style={{
          background:
            'linear-gradient(178deg, rgba(255,255,255,0.90) 0%, rgba(250,253,255,0.86) 52%, rgba(242,248,255,0.84) 100%)',
          backdropFilter: 'blur(26px) saturate(180%)',
          WebkitBackdropFilter: 'blur(26px) saturate(180%)',
        }}
      >
        {/* 顶部一层极淡高光，让圆角有厚度，不至于像一块贴纸 */}
        <span
          aria-hidden
          className="pointer-events-none absolute inset-x-0 top-0 h-[42%]"
          style={{
            background:
              'linear-gradient(180deg, rgba(255,255,255,0.55) 0%, rgba(255,255,255,0) 100%)',
          }}
        />

        {/* 状态点：绿=代理在跑，灰=未启动 */}
        <span
          title={stats.running ? '代理运行中' : '代理未启动'}
          className={`relative mb-3 h-[5px] w-[5px] flex-shrink-0 rounded-full ${
            stats.running
              ? 'bg-emerald-500 shadow-[0_0_0_3px_rgba(16,185,129,0.14)]'
              : 'bg-slate-300'
          }`}
        />

        <Cell
          value={isLoading ? '—' : formatInt(stats.requests)}
          label="今日请求"
          tone={stats.failed > 0 ? 'amber' : 'blue'}
          title={stats.failed > 0 ? `失败 ${formatInt(stats.failed)} 次` : '全部成功'}
        />
        <Cell
          value={isLoading ? '—' : formatCredit(stats.spent)}
          label="消耗积分"
          tone="slate"
          title="今日实际扣费（上游上报值）"
        />
        <Cell
          value={isLoading ? '—' : formatInt(stats.claimed)}
          label="今日已领"
          tone="green"
          title={stats.claiming > 0 ? `${stats.claiming} 个账号已领` : '今日尚未领取'}
        />
        <Cell
          value={isLoading ? '—' : formatTokens(stats.tokens)}
          label="Token"
          tone="slate"
          title="今日计费 Token（不含缓存命中）"
        />

        {/* 悬停才出现的小 ×：待机时不占视觉 */}
        <button
          type="button"
          data-no-drag
          title="隐藏悬浮窗（设置里可重新打开，托盘也能唤出）"
          onClick={hide}
          className={`absolute right-[7px] top-[7px] flex h-[15px] w-[15px] items-center justify-center rounded-full bg-slate-900/10 text-[11px] leading-none text-slate-500 transition-opacity duration-200 hover:bg-rose-500/15 hover:text-rose-600 ${
            hovered ? 'opacity-100' : 'pointer-events-none opacity-0'
          }`}
        >
          ×
        </button>
      </div>
    </div>
  );
}

/** 一组数字。四组等高竖排，所以用 flex-1 而不是写死高度。 */
function Cell({
  value,
  label,
  tone,
  title,
}: {
  value: string;
  label: string;
  tone: 'blue' | 'green' | 'amber' | 'slate';
  title: string;
}) {
  const toneClass =
    tone === 'green'
      ? 'text-emerald-600'
      : tone === 'amber'
        ? 'text-amber-600'
        : tone === 'slate'
          ? 'text-slate-800'
          : 'text-sky-600';
  return (
    <div
      title={title}
      className="relative flex min-h-0 w-full flex-1 flex-col items-center justify-center gap-[3px]"
    >
      <div
        className={`text-[17px] font-bold leading-none tracking-[-0.02em] tabular-nums ${toneClass}`}
      >
        {value}
      </div>
      <div className="text-[9px] font-medium leading-none text-slate-400">{label}</div>
    </div>
  );
}
