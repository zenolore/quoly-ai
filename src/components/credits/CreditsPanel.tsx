import { memo, useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight, CheckCircle2, ChevronDown, ChevronRight, Coins, Copy, ExternalLink, KeyRound, Loader2,
  FileUp, Info, Plus, Power, RefreshCw, Server, ShieldCheck, SlidersHorizontal, Sparkles, Trash2, X,
} from "lucide-react";
// 别名导入：组件内部大量使用 `open` 作为展开状态，直接导入会触发 no-shadow。
import { open as openExternal } from "@tauri-apps/plugin-shell";
import { toast } from "sonner";
import { Modal } from "@/components/ui/Modal";
import { Pressable } from "@/components/ui/Pressable";
import { AgentIcon } from "@/components/ui/AgentIcon";
import { ModelInfoPopover } from "./ModelInfoPopover";
import { ModelHealthView } from "./ModelHealthView";
import { creditsQueryKey, useCreditsAction, useCreditsDashboard } from "@/hooks/useCredits";
import {
  creditsApi, type CreditAccount, type CreditDashboard,
  type CreditModel, type CreditPlatform, type CreditRegion,
  type SubAccount,
  type GrowthAccountState, type GrowthDay, type ImportOutcome,
  type OAuthStart, type ParseReport, type RewardOutcome,
} from "@/lib/api/credits";

const platforms: Array<{ id: CreditPlatform; name: string }> = [
  { id: "workbuddy", name: "WorkBuddy" },
  { id: "traework", name: "TRAE" },
  { id: "qoder", name: "Qoder" },
  { id: "minimax-code", name: "MiniMax Code" },
  { id: "doubao", name: "豆包" },
];
const sections = [
  { id: "accounts", label: "账户与积分" },
  { id: "proxy", label: "本地代理" },
  { id: "health", label: "模型监控" },
] as const;
type PanelTab = typeof sections[number]["id"];
const platformName = (platform: CreditPlatform) => platforms.find((item) => item.id === platform)?.name ?? platform;
/** 停用或封禁的账号统一标红（列表行与详情卡片共用）。 */
const inactiveAccount = (account: { enabled: boolean; status: string }) => !account.enabled || account.status === "banned";
/**
 * 仅可签到的账号：签到/领积分正常，但调模型会被上游拒绝（WorkBuddy 11140）。
 * 单独判断，因为它既不是"好号"也不是"坏号"——标红会让人以为要删，
 * 标绿又会让人拿去跑任务然后频繁失败，所以用琥珀色提示。
 */
const signableOnly = (account: { status: string }) => account.status === "signable";
const regionName = (region?: CreditRegion) => region === "intl" ? "国际版" : "国内版";
const unitName = (unit?: string) => ({ credits: "积分", usd: "USD", requests: "次", mixed: "分项额度", unknown: "单位未知" })[unit ?? "credits"] ?? unit;

/**
 * 数字格式化器**预建并复用**，不要在函数里 new。
 *
 * 为什么（2026-11-18 实测）：`new Intl.NumberFormat` 是重量级对象，
 * 构造一次比直接 format 慢 34 倍（实测 1,896 次调用：每次新建 34ms，
 * 复用实例 1ms）。而这个函数在 158 个账号卡片里被调用上千次，
 * 每次重渲染全部重建 —— 这是主线程被拖住的主要原因之一。
 */
const FMT_INT = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 2 });
const FMT_USD = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 6 });
const amount = (value: number, unit?: string) => (unit?.toLowerCase() === "usd" ? FMT_USD : FMT_INT).format(value);
const number = (value: number) => FMT_INT.format(value);
const time = (value?: number | null) => value ? new Date(value).toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "尚未同步";
const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error);
const isCredits = (kind: string) => ["credit", "credits", "points"].includes(kind);
const isClaimable = (account: CreditAccount) => account.enabled && account.snapshot?.rewards.some((reward) =>
  isCredits(reward.kind) && ["claimable", "available", "eligible"].includes(reward.status));
const accountModels = (account: CreditAccount) => {
  const seen = new Set<string>();
  return (account.snapshot?.models ?? []).filter((model) => {
    if (seen.has(model.id)) return false;
    seen.add(model.id);
    return true;
  });
};
const availableModels = (data: CreditDashboard) => data.accounts.filter((account) => account.enabled && account.status === "ready" && (account.snapshot?.balance == null || account.snapshot.balance > 0))
  .flatMap((account) => accountModels(account).map((model) => ({ account, model })));

const labels: Record<string, string> = {
  active: "已连接", ready: "已连接", connected: "已连接", authenticated: "已连接",
  expired: "登录已过期", auth_expired: "登录已过期", error: "连接异常", unknown: "待刷新",
  // 仅可签到：签到/领积分正常，但调模型会被上游 11140 拒绝。
  // 必须跟「已连接」区分开，否则用户会以为它能调模型，用它跑任务就频繁失败。
  signable: "仅可签到",
  // 连续 3 天检测不过，已自动停用（用户 2026-11-18 要求）。
  retired: "已停用（连续3天不可用）",
  unsupported: "暂不支持", pending: "等待授权", claimed: "已领取", success: "成功",
  failed: "失败", skipped: "已跳过", claimable: "可领取", available: "可领取",
  unavailable: "暂不可领", already_claimed: "已领取", not_supported: "暂不支持",
  device_required: "需要已绑定的设备", eligible: "可领取",
  cancelled: "已取消", completed: "成功",
};

function Badge({ children, success = false }: { children: ReactNode; success?: boolean }) {
  return <span className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[11px] font-medium ${success ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300" : "bg-slate-500/10 text-slate-600 dark:text-slate-300"}`}>{children}</span>;
}

async function copyValue(value: string, label: string) {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
    } else {
      const field = document.createElement("textarea");
      field.value = value;
      field.style.position = "fixed";
      field.style.opacity = "0";
      document.body.appendChild(field);
      field.select();
      const copied = document.execCommand("copy");
      field.remove();
      if (!copied) throw new Error("复制失败");
    }
    toast.success(`已复制${label}`);
  } catch {
    toast.error("复制失败，请检查剪贴板权限");
  }
}

function CopyField({ label, value }: { label: string; value: string }) {
  return <div className="space-y-2">
    <span className="text-xs font-medium text-slate-500 dark:text-slate-400">{label}</span>
    <div className="glass-code flex items-center gap-3 rounded-xl px-3 py-2.5">
      <code className="min-w-0 flex-1 break-all text-xs">{value}</code>
      <Pressable type="button" onClick={() => void copyValue(value, label)} className="glass-icon-button shrink-0" aria-label={`复制${label}`}><Copy size={14} /></Pressable>
    </div>
  </div>;
}

type ModelEntry = { account: CreditAccount; model: CreditModel };
type ModelGroup = { title: string; entries: ModelEntry[] };

interface ModelRowProps extends ModelEntry {
  infoId?: string;
  onPreview: (entry: ModelEntry, anchor: HTMLElement, immediate?: boolean) => void;
  onToggle: (entry: ModelEntry, anchor: HTMLElement) => void;
  onLeave: () => void;
  onBlur: () => void;
}

function ModelRow({ account, model, infoId, onPreview, onToggle, onLeave, onBlur }: ModelRowProps) {
  const row = useRef<HTMLDivElement>(null);
  const id = `${account.platform}${account.region === "intl" ? "-intl" : ""}/${model.id}`;
  return <div ref={row} className={`rounded-xl px-3 py-2.5 transition-colors ${infoId ? "bg-blue-500/10" : "bg-slate-500/5 hover:bg-slate-500/10"}`}
    onPointerEnter={(event) => onPreview({ account, model }, event.currentTarget)} onPointerLeave={onLeave} onBlur={onBlur}>
    <div className="flex flex-wrap items-center justify-between gap-2">
      <button type="button" className="min-w-0 flex-1 rounded-lg text-left outline-none focus-visible:ring-2 focus-visible:ring-blue-500/60"
        aria-label={`查看 ${model.name || model.id} 模型信息`} aria-describedby={infoId} aria-expanded={!!infoId}
        onFocus={() => { if (row.current) onPreview({ account, model }, row.current, true); }}
        onClick={() => { if (row.current) onToggle({ account, model }, row.current); }}>
        <span className="flex items-center gap-2 text-xs font-medium"><span className="min-w-0 break-all">{model.name || model.id}</span><Info size={13} className="shrink-0 text-slate-400" aria-hidden="true" /></span>
        <span className="mt-1 block break-all text-[11px] text-slate-500 dark:text-slate-400">{id}</span>
      </button>
      <div className="flex shrink-0 items-center gap-1.5"><Pressable type="button" className="glass-icon-button" aria-label={`复制模型 ID ${id}`} onClick={() => void copyValue(id, "模型 ID")}><Copy size={13} /></Pressable></div>
    </div>
  </div>;
}

function ModelsModal({ title, groups, onClose }: { title: string; groups: ModelGroup[]; onClose: () => void }) {
  const infoId = useId();
  const [details, setDetails] = useState<{ entry: ModelEntry; anchor: HTMLElement } | null>(null);
  const delay = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pinned = useRef(false);
  const pointerInside = useRef(false);
  const clearDelay = useCallback(() => { if (delay.current) clearTimeout(delay.current); delay.current = null; }, []);
  const dismiss = useCallback(() => { clearDelay(); pinned.current = false; pointerInside.current = false; setDetails(null); }, [clearDelay]);
  useEffect(() => clearDelay, [clearDelay]);

  const preview = (entry: ModelEntry, anchor: HTMLElement, immediate = false) => {
    clearDelay();
    if (!immediate) pointerInside.current = true;
    const show = () => { pinned.current = false; setDetails({ entry, anchor }); };
    if (immediate) show();
    else if (details?.anchor !== anchor) delay.current = setTimeout(show, 180);
  };
  const scheduleClose = () => {
    clearDelay();
    if (pinned.current) return;
    delay.current = setTimeout(() => {
      if (!pointerInside.current && !details?.anchor.contains(document.activeElement)) setDetails(null);
    }, 160);
  };
  const leave = () => { pointerInside.current = false; scheduleClose(); };
  const blur = () => { pinned.current = false; scheduleClose(); };
  const toggle = (entry: ModelEntry, anchor: HTMLElement) => {
    if (details?.anchor === anchor && pinned.current) dismiss();
    else { clearDelay(); pinned.current = true; setDetails({ entry, anchor }); }
  };

  return <Modal open onClose={onClose} size="xl">
    <div className="flex items-center justify-between gap-3 border-b border-slate-500/10 px-6 py-4"><div><h2 className="text-base font-semibold">{title}</h2><p className="mt-1 text-[11px] text-slate-500 dark:text-slate-400">悬浮或点击模型，查看通道参数与能力。</p></div><Pressable type="button" className="glass-icon-button" onClick={onClose} aria-label="关闭模型列表"><X size={18} /></Pressable></div>
    <div className="max-h-[75vh] space-y-6 overflow-y-auto p-6">
      {groups.length ? groups.map((group) => <section key={group.title}><h3 className="text-xs font-semibold text-slate-500 dark:text-slate-400">{group.title}</h3><div className="mt-3 space-y-2.5">{group.entries.map(({ account, model }) => <ModelRow key={`${account.id}-${model.id}`} account={account} model={model}
        infoId={details?.entry.model === model && details.entry.account.id === account.id ? infoId : undefined}
        onPreview={preview} onToggle={toggle} onLeave={leave} onBlur={blur} />)}</div></section>) : <p className="text-xs text-slate-500 dark:text-slate-400">模型目录尚未就绪，刷新账户后查看。</p>}
    </div>
    {details ? <ModelInfoPopover id={infoId} anchor={details.anchor} account={details.entry.account} model={details.entry.model} platformName={platformName(details.entry.account.platform)} onDismiss={dismiss}
      onPointerEnter={() => { pointerInside.current = true; clearDelay(); }} onPointerLeave={leave} /> : null}
  </Modal>;
}

/**
 * 单个账号卡片。
 *
 * 用 `memo` 包住（2026-11-18 性能优化）：账号已经有 158 个，每次 dashboard
 * 刷新都会重渲染全部卡片，而绝大多数账号的数据根本没变。memo 后只有数据
 * 变了的卡片才重建，主线程压力大幅下降 —— 这是"拖动卡顿"的直接缓解。
 *
 * 注意：props 里**不能传内联箭头函数**，否则引用每次都变，memo 形同虚设。
 * 所以 `onDelete` 改成传 `account` + 稳定的 `onDeleteAccount` 回调。
 */
const AccountCard = memo(function AccountCard({ account, onDeleteAccount, refreshing = false }: { account: CreditAccount; onDeleteAccount: (account: CreditAccount) => void; refreshing?: boolean }) {
  const refresh = useCreditsAction(() => creditsApi.refresh(account.id));
  const claim = useCreditsAction(() => creditsApi.claim(account.id));
  const toggle = useCreditsAction((enabled: boolean) => creditsApi.setAccountEnabled(account.id, enabled));
  const snapshot = account.snapshot;
  const automaticReward = account.platform === "workbuddy" && account.region === "intl";
  const noDailyReward = account.platform === "traework" && account.region === "intl";
  const models = accountModels(account);
  const credits = snapshot?.rewards.filter((reward) => isCredits(reward.kind)) ?? [];
  const activities = snapshot?.rewards.filter((reward) => reward.kind === "activity") ?? [];
  const otherRewards = snapshot?.rewards.filter((reward) => !isCredits(reward.kind) && reward.kind !== "activity") ?? [];
  const busy = refresh.isPending || claim.isPending || toggle.isPending;
  // 停用/封禁的账号整体标红：红色描边 + 淡红底 + 降低饱和度，一眼能从列表里区分出来。
  const inactive = !account.enabled || account.status === "banned";
  const [showingModels, setShowingModels] = useState(false);
  return <article className={`glass-card min-w-0 p-5 ${inactive ? "opacity-80 ring-2 ring-red-500/60 border-red-500/40 bg-red-500/5" : ""}`}>
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0"><h3 className={`truncate text-sm font-semibold ${inactive ? "text-red-600 dark:text-red-400" : ""}`}>{account.label || platformName(account.platform)}</h3>
        {/* 副标题里平台名已经由列标题给过了，再拼一遍会变成「豆包 豆包」。
            豆包的 userId 是网关地址，比"账户"两个字有信息量，单独展示。 */}
        <p className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400">{platformName(account.platform)} · {regionName(account.region)} · {account.userId || "账户"}</p></div>
      <Pressable type="button" className="glass-icon-button shrink-0" aria-label={`移除账户 ${account.label}`} onClick={() => onDeleteAccount(account)} disabled={busy}><Trash2 size={14} /></Pressable>
    </div>
    <div className="mt-5 flex items-end justify-between gap-2">
      <div><p className="text-[11px] font-medium text-slate-500 dark:text-slate-400">剩余额度</p><p className="mt-2 text-3xl font-semibold tracking-tight tabular-nums">{snapshot?.balance != null ? amount(snapshot.balance, snapshot.unit) : "—"}<span className="ml-2 text-xs font-normal text-slate-500 dark:text-slate-400">{unitName(snapshot?.unit)}</span></p></div>
      <Badge success={account.enabled && ["active", "ready", "connected", "authenticated"].includes(account.status)}>{!account.enabled ? "已停用" : account.status === "banned" ? "已封禁" : labels[account.status] ?? account.status}</Badge>
    </div>
    {signableOnly(account) ? <p className="mt-2 rounded-lg bg-amber-500/10 px-3 py-2 text-[11px] leading-relaxed text-amber-800 dark:text-amber-200">此账号签到与领积分正常，但调用模型会被上游拒绝（11140）。它不会参与 API 路由，不会拉低调用成功率；积分照常领取。</p> : null}
    <p role="status" className="mt-2 text-[11px] text-slate-500 dark:text-slate-400">{refreshing || refresh.isPending ? "同步中 · " : ""}{snapshot?.fetchedAt ? `上次成功同步 ${time(snapshot.fetchedAt)}` : "余额待同步"}{snapshot?.stale ? account.lastError ? " · 同步失败，显示历史余额" : " · 余额待更新" : ""}{snapshot?.quota ? ` · ${snapshot.quota}` : ""}</p>
    {account.lastError || snapshot?.diagnostic ? <p className={`mt-3 rounded-lg px-3 py-2 text-xs leading-relaxed ${inactive ? "bg-red-500/10 text-red-700 dark:text-red-300" : "bg-amber-500/10 text-amber-800 dark:text-amber-200"}`}>{account.lastError || snapshot?.diagnostic}</p> : null}
    {snapshot?.subAccounts?.length ? <DoubaoSubAccounts subs={snapshot.subAccounts} /> : null}
    <div className="mt-4 flex flex-wrap gap-2">
      {!automaticReward && !noDailyReward ? <Pressable type="button" className="glass-primary-button min-h-8 px-3 py-2 text-xs" disabled={busy || !isClaimable(account)} onClick={() => claim.mutate(undefined, { onSuccess: reportClaims })}>{claim.isPending ? <Loader2 size={13} className="animate-spin" /> : <Coins size={13} />}领取积分</Pressable> : null}
      <Pressable type="button" className="glass-secondary-button min-h-8 px-3 py-2 text-xs" disabled={busy} onClick={() => refresh.mutate(undefined)}><RefreshCw size={13} className={refresh.isPending || refreshing ? "animate-spin" : ""} />刷新</Pressable>
      <Pressable type="button" className="glass-secondary-button min-h-8 px-3 py-2 text-xs" disabled={busy} onClick={() => toggle.mutate(!account.enabled)}>{account.enabled ? "停用" : "启用"}</Pressable>
    </div>
    <div className="mt-4 border-t border-slate-500/10 pt-4">
      <p className="mb-2 text-xs font-medium">{automaticReward ? "奖励发放" : "积分领取"}</p>
      {credits.length ? <div className="space-y-2">{credits.map((reward) => <div key={reward.id} className="flex items-start justify-between gap-3 text-xs"><span className="min-w-0 flex-1 break-words">{reward.name}{reward.amount != null ? ` · ${number(reward.amount)}` : ""}</span><span className="shrink-0 whitespace-nowrap text-slate-500 dark:text-slate-400">{labels[reward.status] ?? reward.status}</span></div>)}</div> : <p className="text-xs text-slate-500 dark:text-slate-400">{automaticReward ? "符合条件的活跃奖励由平台自动发放，无需手动领取。" : noDailyReward ? "国际版未提供每日签到任务。" : snapshot ? "当前没有可领取的积分任务。" : "刷新后查看领取资格。"}</p>}
      {account.platform !== "qoder" && activities.length ? <div className="mt-3 rounded-lg bg-sky-500/5 p-2.5"><p className="mb-1 text-[11px] font-medium text-slate-500 dark:text-slate-400">平台活动</p>{activities.map((reward) => <div key={reward.id} className="flex flex-wrap items-center justify-between gap-1 py-0.5"><span className="min-w-0 flex-1 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">{reward.name} · {labels[reward.status] ?? reward.status}</span>{reward.actionUrl ? <Pressable type="button" className="glass-secondary-button min-h-6 shrink-0 px-2 py-0.5 text-[11px]" onClick={() => void openExternal(reward.actionUrl!).catch(() => toast.error("无法打开活动页"))}>活动详情<ExternalLink size={11} /></Pressable> : null}</div>)}</div> : null}
      {account.platform !== "qoder" && otherRewards.length ? <div className="mt-3 rounded-lg bg-slate-500/5 p-2.5"><p className="mb-1 text-[11px] font-medium text-slate-500 dark:text-slate-400">优惠券及其他奖励</p>{otherRewards.map((reward) => <div key={reward.id} className="flex flex-wrap items-center justify-between gap-1 py-0.5"><span className="min-w-0 flex-1 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">{reward.name} · {labels[reward.status] ?? reward.status}</span>{reward.actionUrl ? <Pressable type="button" className="glass-secondary-button min-h-6 shrink-0 px-2 py-0.5 text-[11px]" onClick={() => void openExternal(reward.actionUrl!).catch(() => toast.error("无法打开活动页"))}>打开活动页<ExternalLink size={11} /></Pressable> : null}</div>)}</div> : null}
    </div>
    {account.region === "intl" && snapshot?.packs.length ? <div className="mt-4 space-y-2 border-t border-slate-500/10 pt-3" aria-label="额度明细">{snapshot.packs.map((pack) => <div key={pack.id} className="flex justify-between gap-3 text-xs"><span className="min-w-0 break-words text-slate-500 dark:text-slate-400">{pack.name}</span><span className="shrink-0 tabular-nums">{pack.remaining == null ? "未知" : amount(pack.remaining, pack.unit ?? snapshot.unit)} {unitName(pack.unit ?? snapshot.unit)}</span></div>)}</div> : null}
    <div className="mt-4 border-t border-slate-500/10 pt-3"><Pressable type="button" className="glass-secondary-button min-h-8 px-3 py-2 text-xs" onClick={() => setShowingModels(true)}>可用模型 · {models.length}</Pressable></div>
    {showingModels ? <ModelsModal title="可用模型" groups={[{ title: account.label || platformName(account.platform), entries: models.map((model) => ({ account, model })) }]} onClose={() => setShowingModels(false)} /> : null}
  </article>;
});

/**
 * 豆包网关内部的账号清单。
 *
 * 豆包一个"账户"是一个网关，网关里养着多个豆包号。其他平台是一张卡片一个号，
 * 豆包这里用一份紧凑列表把内部的号都列出来，状态、剩余有效期、被用了几次
 * 都摆出来 —— 用户能一眼看出"我登了 7 个号，哪个在干活、哪个快过期了"。
 *
 * 不把它们渲染成独立卡片，是因为这些号没法单独启用/停用/删除（真删要回网关
 * 操作），做成可点卡片会误导。
 */
function DoubaoSubAccounts({ subs }: { subs: SubAccount[] }) {
  const stateMeta: Record<string, { label: string; className: string }> = {
    ok: { label: "可用", className: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300" },
    expiring_soon: { label: "即将过期", className: "bg-amber-500/10 text-amber-700 dark:text-amber-300" },
    cooldown: { label: "限流中", className: "bg-amber-500/10 text-amber-700 dark:text-amber-300" },
    expired: { label: "需重登", className: "bg-rose-500/10 text-rose-700 dark:text-rose-300" },
    disabled: { label: "已停用", className: "bg-slate-500/10 text-slate-500 dark:text-slate-400" },
  };
  return <div className="mt-4 border-t border-slate-500/10 pt-3" aria-label="网关内账号">
    <p className="mb-2 text-xs font-medium">网关内账号 · {subs.length}</p>
    <div className="space-y-1.5">
      {subs.map((sub) => {
        const meta = stateMeta[sub.state] ?? { label: sub.state, className: "bg-slate-500/10 text-slate-500" };
        const left = sub.secondsLeft;
        const leftText = left == null
          ? ""
          : left <= 0 ? "已过期"
          : left < 86400 ? `${Math.max(1, Math.round(left / 3600000))} 小时后过期`
          : `${Math.round(left / 86400000)} 天后过期`;
        return <div key={sub.name} className="flex items-center gap-2 rounded-lg bg-slate-500/5 px-2.5 py-1.5 text-[11px]">
          <span className="min-w-0 flex-1 truncate" title={sub.name}>{sub.name}</span>
          <span className="shrink-0 tabular-nums text-slate-500 dark:text-slate-400">{leftText}</span>
          {sub.calls > 0 ? <span className="shrink-0 tabular-nums text-slate-500 dark:text-slate-400">{sub.calls} 次</span> : null}
          <span className={`shrink-0 rounded-full px-1.5 py-0.5 ${meta.className}`}>{meta.label}</span>
        </div>;
      })}
    </div>
    <p className="mt-2 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">
      这些号由豆包网关轮换使用，增删和重新登录在网关面板操作。
    </p>
  </div>;
}

/**
 * 「仅可签到」账号折叠区。
 *
 * 这类账号的处境很特殊（2026-11-18 实测）：
 *   - 签到接口返回 active:true、每天正常拿积分 ✅
 *   - 推理接口直接 403/11140 ❌
 *
 * 所以它们既不能当"好号"铺在正常列表里（用户拿去跑任务会频繁失败），
 * 也不能当"坏号"折叠到异常区（它们每天都在真实产出积分）。
 * 单独一组，用琥珀色说明清楚，既保住积分收益，又不误导用户。
 *
 * 它们不参与网关路由 —— 网关的 account_eligible 只认 ready，
 * 而这里的状态是 signable，天然被排除。
 */
function SignableAccountsGroup({ accounts, onDelete, platformLabel }: { accounts: CreditAccount[]; onDelete: (account: CreditAccount) => void; platformLabel: string }) {
  const [open, setOpen] = useState(false);
  return <div className="rounded-[var(--radius-card,12px)] border border-dashed border-amber-400/50 dark:border-amber-600/40">
    <button
      type="button"
      onClick={() => setOpen((v) => !v)}
      className="flex w-full items-center gap-2 px-3 py-2.5 text-left"
      aria-expanded={open}
    >
      <ChevronDown size={14} className={`shrink-0 text-amber-500 transition-transform ${open ? "" : "-rotate-90"}`} />
      <span className="text-xs font-medium text-amber-700 dark:text-amber-300">{accounts.length} 个仅可签到</span>
      <span className="text-[11px] text-slate-400 dark:text-slate-500">积分照领 · 不参与 API 路由</span>
      <span className="ml-auto text-[11px] text-slate-400 dark:text-slate-500">{open ? "收起" : "点开查看"}</span>
    </button>
    {open ? <div className="space-y-1.5 border-t border-amber-200/60 px-3 py-2.5 dark:border-amber-900/40">
      <p className="pb-1 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">
        {platformLabel} 的这批账号签到、领积分都正常，但调用模型会被上游以 11140（request illegal）拒绝。
        它们不会参与 API 路由，所以不会拉低调用成功率；每天该拿的积分照常领取。
        上游风控有时会自行解除，过几天可以重新体检一次。
      </p>
      {accounts.map((account) => {
        const name = account.label || account.userId || account.id;
        return <div key={account.id} className="flex items-center gap-2 rounded-md bg-amber-500/[0.06] px-2 py-1.5">
          <span className="min-w-0 flex-1 truncate text-[11px] text-slate-600 dark:text-slate-300" title={name}>{name}</span>
          <span className="shrink-0 text-[11px] text-amber-700 dark:text-amber-300">仅可签到</span>
          <button
            type="button"
            onClick={() => onDelete(account)}
            className="shrink-0 text-[11px] text-slate-400 hover:text-rose-600 dark:hover:text-rose-400"
            title="删除该账号"
          >
            删除
          </button>
        </div>;
      })}
    </div> : null}
  </div>;
}

/**
 * 异常账号折叠区：封禁 / 过期 / 停用的账号收在这里，默认收起，只显示数量。
 *
 * 为什么要折叠：一个平台可能攒了几十个签不了号的账号（历史封禁标记、
 * 凭证过期等），如果和正常号混排，正常号会被挤到看不见的地方，用户也
 * 判断不出"我到底还有多少能用的号"。
 *
 * 折叠后：标题行写「N 个异常账号（点开查看）」，展开才逐条列出，
 * 每条带状态和原因。这样隔几天回来重测也方便——展开一眼就能看到还有哪些。
 */
function DeadAccountsGroup({ accounts, onDelete, platformLabel }: { accounts: CreditAccount[]; onDelete: (account: CreditAccount) => void; platformLabel: string }) {
  const [open, setOpen] = useState(false);
  const banned = accounts.filter((a) => a.status === "banned").length;
  const expired = accounts.filter((a) => a.status === "expired").length;
  const disabled = accounts.filter((a) => !a.enabled).length;
  const parts = [banned ? `封禁 ${banned}` : "", expired ? `过期 ${expired}` : "", disabled ? `停用 ${disabled}` : ""].filter(Boolean);
  return <div className="rounded-[var(--radius-card,12px)] border border-dashed border-slate-300/70 dark:border-slate-700">
    <button
      type="button"
      onClick={() => setOpen((v) => !v)}
      className="flex w-full items-center gap-2 px-3 py-2.5 text-left"
      aria-expanded={open}
    >
      <ChevronDown size={14} className={`shrink-0 text-slate-400 transition-transform ${open ? "" : "-rotate-90"}`} />
      <span className="text-xs font-medium text-slate-600 dark:text-slate-300">{accounts.length} 个异常账号</span>
      <span className="text-[11px] text-slate-400 dark:text-slate-500">{parts.join(" · ")}</span>
      <span className="ml-auto text-[11px] text-slate-400 dark:text-slate-500">{open ? "收起" : "点开查看"}</span>
    </button>
    {open ? <div className="space-y-1.5 border-t border-slate-200/70 px-3 py-2.5 dark:border-slate-800">
      <p className="pb-1 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">
        这些 {platformLabel} 账号当前无法签到。建议隔几天再刷新重测一次：上游封禁有时会自行解除，
        仅凭证过期的号重新登录即可恢复。确认长期不行再删除。
      </p>
      {accounts.map((account) => {
        const name = account.label || account.userId || account.id;
        const reason = !account.enabled ? "已停用" : account.status === "banned" ? "已封禁" : "登录已过期";
        return <div key={account.id} className="flex items-center gap-2 rounded-md bg-slate-500/[0.04] px-2 py-1.5 dark:bg-slate-400/[0.06]">
          <span className="min-w-0 flex-1 truncate text-[11px] text-slate-600 dark:text-slate-300" title={name}>{name}</span>
          <span className="shrink-0 text-[11px] text-rose-600 dark:text-rose-400">{reason}</span>
          <button
            type="button"
            onClick={() => onDelete(account)}
            className="shrink-0 text-[11px] text-slate-400 hover:text-rose-600 dark:hover:text-rose-400"
            title="删除该账号"
          >
            删除
          </button>
        </div>;
      })}
    </div> : null}
  </div>;
}

function AccountStack({ accounts, onDelete, refreshingAccountIds = [] }: { accounts: CreditAccount[]; onDelete: (account: CreditAccount) => void; refreshingAccountIds?: string[] }) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const frontCard = useRef<HTMLDivElement>(null);
  const selectedIndex = Math.max(0, accounts.findIndex((account) => account.id === selectedId));
  return <div className="isolate grid" style={{ paddingTop: Math.max(0, accounts.length - 1) * 44 }}>
    {accounts.map((account, index) => {
      const active = index === selectedIndex;
      const depth = active ? 0 : index < selectedIndex ? index + 1 : index;
      const name = account.label || platformName(account.platform);
      const balance = account.snapshot?.balance;
      const unit = unitName(account.snapshot?.unit);
      return <div
        key={account.id}
        className="relative grid min-w-0 origin-top transition-transform duration-300 ease-[var(--motion-spring)] motion-reduce:transition-none [grid-area:1/1]"
        style={{ transform: `translateY(${-depth * 44}px) scale(${Math.max(0.9, 1 - depth * 0.025)})`, zIndex: accounts.length - depth }}
      >
        <div
          ref={active ? frontCard : undefined}
          role="group"
          aria-label={`账户 ${name}`}
          aria-hidden={!active}
          tabIndex={active ? -1 : undefined}
          className={`grid min-w-0 rounded-[var(--radius-card,12px)] ${active ? "bg-[hsl(var(--card))]" : "invisible pointer-events-none"}`}
        >
          <AccountCard account={account} onDeleteAccount={onDelete} refreshing={refreshingAccountIds.includes(account.id)} />
        </div>
        {!active ? <Pressable
          type="button"
          noScale
          aria-label={`切换到账户 ${name} · ${account.userId || "账户"}`}
          title={`切换到账户 ${name}`}
          onClick={() => {
            setSelectedId(account.id);
            requestAnimationFrame(() => frontCard.current?.focus({ preventScroll: true }));
          }}
          className={`absolute inset-x-0 top-0 flex h-14 items-start gap-2 rounded-[var(--radius-card,12px)] border px-4 pt-3 text-left text-xs transition-colors ${
            inactiveAccount(account)
              ? "border-red-400/70 bg-red-500/10 text-red-700 dark:border-red-500/60 dark:bg-red-500/15 dark:text-red-300"
              : "border-slate-200/80 bg-slate-100 text-slate-600 hover:bg-slate-200/80 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700"
          }`}
        >
          <span className={`min-w-0 flex-1 truncate font-medium ${inactiveAccount(account) ? "text-red-600 dark:text-red-400" : ""}`}>{name} · {regionName(account.region)}</span>
          <span className={`shrink-0 text-[11px] tabular-nums ${inactiveAccount(account) ? "font-medium text-red-600 dark:text-red-400" : "text-slate-500 dark:text-slate-400"}`}>{!account.enabled ? "已停用" : account.status === "banned" ? "已封禁" : balance != null ? `${amount(balance, account.snapshot?.unit)} ${unit}` : "切换账户"}</span>
          <ArrowUpRight size={13} className="mt-0.5 shrink-0 text-slate-400" aria-hidden="true" />
        </Pressable> : null}
      </div>;
    })}
  </div>;
}

function reportClaims(results: RewardOutcome[]) {
  if (!results.length) { toast("当前没有可领取的积分"); return; }
  const successful = results.filter((result) => ["success", "claimed"].includes(result.status)).length;
  if (successful) toast.success(`已领取 ${successful} 项积分奖励`);
  const failed = results.filter((result) => ["failed", "error"].includes(result.status)).length;
  if (failed) toast.error(`${failed} 项领取失败，请查看领取记录`);
  if (!successful && !failed) toast("领取检查完成，请查看领取记录");
}

function AddAccountModal({ platform: initialPlatform, onClose }: { platform: CreditPlatform; onClose: () => void }) {
  const [platform, setPlatform] = useState<CreditPlatform>(initialPlatform);
  const [region, setRegion] = useState<CreditRegion>("cn");
  const [flow, setFlow] = useState<OAuthStart | null>(null);
  const [flowError, setFlowError] = useState("");
  const [importError, setImportError] = useState("");
  const [authorizing, setAuthorizing] = useState(false);
  const [importing, setImporting] = useState(false);
  const queryClient = useQueryClient();

  const importLocal = async () => {
    setImporting(true);
    setImportError("");
    try {
      const account = await creditsApi.importLocal(platform, region);
      await queryClient.invalidateQueries({ queryKey: creditsQueryKey });
      if (account.status !== "ready") {
        setImportError(`账户已保存，但积分同步失败：${account.lastError || account.snapshot?.diagnostic || "平台暂未返回可用积分信息"}`);
        return;
      }
      toast.success("已从本机应用导入账户");
      onClose();
    } catch (error) {
      setImportError(errorMessage(error));
    } finally {
      setImporting(false);
    }
  };

  useEffect(() => {
    if (!flow) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (Date.now() >= flow.expiresAt) { setFlow(null); setFlowError("授权已过期，请重新登录"); return; }
      try {
        const result = await creditsApi.oauthPoll(flow.flowId);
        if (cancelled) return;
        if (result.status === "completed") {
          await queryClient.invalidateQueries({ queryKey: creditsQueryKey });
          if (!cancelled) { toast.success("账户已连接"); onClose(); }
          return;
        }
        if (result.status === "failed") { setFlow(null); setFlowError(result.message); return; }
        timer = setTimeout(() => void poll(), 2000);
      } catch (error) {
        if (!cancelled) { setFlow(null); setFlowError(errorMessage(error)); }
      }
    };
    void poll();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [flow, queryClient, onClose]);

  const login = async () => {
    setAuthorizing(true); setFlowError("");
    try {
      const result = await creditsApi.oauthStart(platform, region);
      await openExternal(result.authorizationUrl);
      setFlow(result);
    } catch (error) { setFlowError(errorMessage(error)); }
    finally { setAuthorizing(false); }
  };
  return <Modal open onClose={onClose} size="xl" dismissOnBackdrop={!authorizing}>
    <div className="flex items-center justify-between border-b border-slate-500/10 px-6 py-4"><h2 className="text-base font-semibold">连接平台账户</h2><Pressable type="button" className="glass-icon-button" onClick={onClose} disabled={authorizing} aria-label="关闭账户表单"><X size={18} /></Pressable></div>
    <div className="max-h-[75vh] space-y-5 overflow-y-auto p-6">
      <fieldset>
        <legend className="mb-2 text-xs font-medium">平台</legend>
        <div className="flex flex-wrap gap-3">
          {platforms.map((item) => {
            const selected = platform === item.id;
            return <Pressable
              key={item.id}
              type="button"
              aria-label={item.name}
              aria-pressed={selected}
              title={item.name}
              disabled={Boolean(flow) || authorizing || importing}
              onClick={() => { setPlatform(item.id); setFlowError(""); setImportError(""); }}
              className={`relative inline-flex h-14 w-14 items-center justify-center rounded-xl border transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${selected ? "border-blue-400 bg-blue-500/10 shadow-sm dark:border-sky-300/60 dark:bg-sky-300/10" : "border-white/55 bg-white/50 hover:bg-white/75 dark:border-white/10 dark:bg-white/5 dark:hover:bg-white/10"}`}
            >
              <AgentIcon id={item.id} name={item.name} className="h-8 w-8" />
              {selected ? <CheckCircle2 size={14} aria-hidden="true" className="absolute -right-1 -top-1 rounded-full bg-white text-blue-600 dark:bg-slate-900 dark:text-sky-300" /> : null}
            </Pressable>;
          })}
        </div>
      </fieldset>
      <fieldset><legend className="mb-2 text-xs font-medium">账号区域</legend><div className="flex gap-2">{(["cn", "intl"] as const).map((value) => <Pressable key={value} type="button" aria-pressed={region === value} disabled={Boolean(flow) || authorizing || importing} className={region === value ? "glass-primary-button text-xs" : "glass-secondary-button text-xs"} onClick={() => { setRegion(value); setFlowError(""); setImportError(""); }}>{regionName(value)}</Pressable>)}</div></fieldset>
      {platform === "traework" && region === "intl" ? <p className="text-xs leading-relaxed text-slate-500 dark:text-slate-400">国际版支持身份与额度查询；模型目录和代理待新版协议验证后开放。</p> : null}
      {platform === "minimax-code" ? <p className="text-xs leading-relaxed text-slate-500 dark:text-slate-400">支持积分、每日签到与模型代理，模型和奖励资格以当前账号返回为准。</p> : null}
      {platform === "doubao" ? <DoubaoGatewayBlock onDone={onClose} /> : null}
      {platform === "doubao" ? null : <div className="rounded-xl bg-blue-500/5 p-4"><p className="text-sm font-medium">在浏览器中完成授权</p><p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">授权完成后自动连接账户。</p>{flow ? <div className="mt-4 flex flex-wrap items-center gap-3"><Loader2 size={16} className="animate-spin text-blue-500" /><span className="text-xs">等待授权 · {time(flow.expiresAt)} 到期</span><Pressable type="button" className="glass-secondary-button min-h-7 px-2 py-1 text-xs" onClick={() => { setFlow(null); setFlowError(""); }}>取消</Pressable></div> : <Pressable type="button" className="glass-primary-button mt-4 text-xs" onClick={() => void login()} disabled={authorizing}>{authorizing ? <Loader2 size={14} className="animate-spin" /> : <ExternalLink size={14} />}前往 {platformName(platform)} {regionName(region)} 登录</Pressable>}{flowError ? <p role="alert" className="mt-3 break-words text-xs text-amber-700 dark:text-amber-200">{flowError}</p> : null}</div>}
      {platform === "traework" || platform === "minimax-code" ? <div className="rounded-xl bg-blue-500/5 p-4"><p className="text-sm font-medium">从本机 {platformName(platform)} {regionName(region)} 应用一键导入</p><p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">读取本机 {platformName(platform)} {regionName(region)}桌面登录信息{platform === "minimax-code" ? "，兼容 mcode CLI 凭据" : region === "cn" ? "，兼容国内 CLI 凭据" : ""}；无需复制 Token。</p><Pressable type="button" className="glass-secondary-button mt-4 text-xs" disabled={importing || authorizing || Boolean(flow)} onClick={() => void importLocal()}>{importing ? <Loader2 size={13} className="animate-spin" /> : <ShieldCheck size={13} />}一键导入本机凭据</Pressable>{importError ? <p role="alert" className="mt-3 break-words text-xs text-amber-700 dark:text-amber-200">{importError}</p> : null}</div> : null}
      {platform === "workbuddy" ? <FileImportBlock onDone={onClose} /> : null}
    </div>
  </Modal>;
}

/**
 * 从 JSON 文件批量导入账户。
 *
 * 分两步：先选文件解析，把识别结果摆出来让人确认；点「确认导入」才真正写库。
 * 这样导错了能当场看见，不会像直接导入那样无声地写进去一堆无效账号。
 */
/**
 * 豆包网关接入表单。
 *
 * 豆包是这个面板里唯一不需要 OAuth 的平台 —— 它后面是本机的豆包网关
 * （Python 进程），凭证就是"网关地址 + API Key"。所以这里给两个输入框，
 * 而不是授权按钮。
 *
 * 地址默认填 127.0.0.1:9091，粘上 Key 就能用。之所以让地址可改，
 * 是为了支持把网关跑在局域网另一台机器上。
 */
function DoubaoGatewayBlock({ onDone }: { onDone: () => void }) {
  const queryClient = useQueryClient();
  const [key, setKey] = useState("");
  const [url, setUrl] = useState("http://127.0.0.1:9091");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const submit = async () => {
    setError("");
    if (!key.trim()) {
      setError("请填写 API Key。它在网关目录的 api_key.txt 里。");
      return;
    }
    setBusy(true);
    try {
      await creditsApi.addDoubao(key, url);
      await queryClient.invalidateQueries({ queryKey: creditsQueryKey });
      toast.success("豆包网关已接入");
      onDone();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded-xl bg-blue-500/5 p-4">
      <p className="text-sm font-medium">接入本机豆包网关</p>
      <p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">
        豆包通过独立的本地网关运行（负责多账号轮换和每日签到），这里只需要填网关地址和密钥。
        即使网关出问题，也不会影响其他平台的调用。
      </p>
      <label className="mt-4 block text-xs font-medium" htmlFor="doubao-gateway-url">网关地址</label>
      <input
        id="doubao-gateway-url"
        className="glass-input mt-1 w-full text-xs"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        placeholder="http://127.0.0.1:9091"
        spellCheck={false}
      />
      <label className="mt-3 block text-xs font-medium" htmlFor="doubao-gateway-key">API Key</label>
      <input
        id="doubao-gateway-key"
        className="glass-input mt-1 w-full text-xs"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="dbt_..."
        spellCheck={false}
        type="password"
      />
      <p className="mt-2 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">
        Key 在网关目录的 <code>api_key.txt</code> 里。网关没启动时这里会提示连不上，先把它运行起来即可。
      </p>
      {error ? <p role="alert" className="mt-3 break-words text-xs text-amber-700 dark:text-amber-200">{error}</p> : null}
      <Pressable type="button" className="glass-primary-button mt-4 text-xs" disabled={busy} onClick={() => void submit()}>
        {busy ? <Loader2 size={13} className="animate-spin" /> : <Server size={13} />}连接网关
      </Pressable>
    </div>
  );
}

function FileImportBlock({ onDone }: { onDone: () => void }) {
  const queryClient = useQueryClient();
  const [report, setReport] = useState<ParseReport | null>(null);
  const [suffix, setSuffix] = useState("（购买）");
  const [content, setContent] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [outcomes, setOutcomes] = useState<ImportOutcome[] | null>(null);

  const pick = async () => {
    setError("");
    setOutcomes(null);
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: "账号文件", extensions: ["json"] }],
      });
      if (!picked || typeof picked !== "string") return;
      const { readTextFile } = await import("@tauri-apps/plugin-fs");
      const text = await readTextFile(picked);
      setContent(text);
      const parsed = await creditsApi.parseAccountFile(text);
      setReport(parsed);
    } catch (e) {
      setError(errorMessage(e));
      setReport(null);
    }
  };

  const apply = async () => {
    if (!content) return;
    setBusy(true);
    setError("");
    try {
      const result = await creditsApi.importAccounts(content, suffix);
      setOutcomes(result);
      const ok = result.filter((r) => r.ok).length;
      if (ok > 0) {
        await queryClient.invalidateQueries({ queryKey: creditsQueryKey });
        toast.success(`已导入 ${ok} 个账户${result.length > ok ? `，${result.length - ok} 个失败` : ""}`);
        // 全部成功就直接收起弹窗；有失败时留在原地让人看清原因。
        if (ok === result.length) onDone();
      } else {
        toast.error("没有账户导入成功");
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = report?.accounts.filter((a) => a.problems.length === 0).length ?? 0;
  const bad = (report?.accounts.length ?? 0) - ready;

  return (
    <div className="rounded-xl bg-emerald-500/5 p-4">
      <p className="text-sm font-medium">从 JSON 文件批量导入</p>
      <p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">
        支持整包导出和单号导出两种结构，会自动识别字段差异、跳过重复账号。导入前逐个校验凭据，无效的不会写入。
      </p>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Pressable type="button" className="glass-secondary-button text-xs" disabled={busy} onClick={() => void pick()}>
          <FileUp size={13} />选择文件
        </Pressable>
        {report ? (
          <label className="flex items-center gap-1.5 text-xs">
            标签后缀
            <input
              className="glass-input h-7 w-28 px-2 text-xs"
              value={suffix}
              onChange={(e) => setSuffix(e.target.value)}
              placeholder="可留空"
            />
          </label>
        ) : null}
      </div>

      {report ? (
        <div className="mt-3">
          <p className="text-xs text-slate-600 dark:text-slate-300">
            识别为 <span className="font-medium">{report.format}</span> · 共 {report.accounts.length} 个
            {report.duplicates ? ` · 文件内重复 ${report.duplicates} 个已跳过` : ""}
            {bad > 0 ? ` · ${bad} 个有问题` : ""}
          </p>
          <div className="glass-code mt-2 max-h-44 overflow-auto rounded-lg px-2.5 py-2 text-[11px] leading-relaxed">
            {report.accounts.map((a, i) => (
              <p key={i} className="break-all">
                {a.problems.length === 0 ? "✅" : "⚠️"} {a.nickname}
                {a.existingId ? " · 已存在将更新" : ""}
                {a.problems.length ? ` · ${a.problems.join("；")}` : ""}
              </p>
            ))}
          </div>
          <Pressable
            type="button"
            className="glass-primary-button mt-3 text-xs"
            disabled={busy || ready === 0}
            onClick={() => void apply()}
          >
            {busy ? <Loader2 size={13} className="animate-spin" /> : <CheckCircle2 size={13} />}
            确认导入 {ready} 个
          </Pressable>
        </div>
      ) : null}

      {outcomes ? (
        <div className="glass-code mt-3 max-h-40 overflow-auto rounded-lg px-2.5 py-2 text-[11px] leading-relaxed">
          {outcomes.map((o, i) => (
            <p key={i} className="break-all">
              {o.ok ? "✅" : "❌"} {o.label} · {o.message}
            </p>
          ))}
        </div>
      ) : null}

      {error ? (
        <p role="alert" className="mt-3 break-words text-xs text-amber-700 dark:text-amber-200">
          {error}
        </p>
      ) : null}
    </div>
  );
}

function AccountsView({ data, onAdd }: { data: CreditDashboard; onAdd: (platform: CreditPlatform) => void }) {
  const refresh = useCreditsAction(() => creditsApi.refresh());
  const claim = useCreditsAction(() => creditsApi.claim());
  const remove = useCreditsAction(creditsApi.deleteAccount);
  const [deleting, setDeleting] = useState<CreditAccount | null>(null);
  const [growthRunning, setGrowthRunning] = useState(false);
  const [autoClaim, setAutoClaim] = useState(data.settings.autoClaim);
  const [autoRefresh, setAutoRefresh] = useState(data.settings.autoRefresh ?? true);
  const [autoGrowth, setAutoGrowth] = useState(data.settings.autoGrowth ?? false);
  const save = useCreditsAction(creditsApi.saveSettings);
  // Which accounts to include in a manual growth run. Null means "all eligible",
  // which is the same default the automatic pass uses; an explicit set lets the
  // user exclude an account without disabling it for the proxy.
  const [growthPick, setGrowthPick] = useState<Set<string> | null>(null);
  const [growthLog, setGrowthLog] = useState<string[]>([]);
  // Daily history and per-account readiness, loaded on demand so the tab does not
  // fire two dozen snapshot requests every time it renders.
  const [growthDays, setGrowthDays] = useState<GrowthDay[] | null>(null);
  const [growthStates, setGrowthStates] = useState<GrowthAccountState[] | null>(null);
  const [loadingIntel, setLoadingIntel] = useState(false);
  const [activationRunning, setActivationRunning] = useState(false);
  const [activationLog, setActivationLog] = useState<string[]>([]);
  const [activationPick, setActivationPick] = useState<Set<string>>(new Set());
  const loadGrowthIntel = async () => {
    setLoadingIntel(true);
    try {
      const [days, states] = await Promise.all([
        creditsApi.growthDaily(14),
        creditsApi.growthAccounts(),
      ]);
      setGrowthDays(days);
      setGrowthStates(states);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setLoadingIntel(false);
    }
  };
  const changed = autoClaim !== data.settings.autoClaim
    || autoRefresh !== (data.settings.autoRefresh ?? true)
    || autoGrowth !== (data.settings.autoGrowth ?? false);
  useEffect(() => { setAutoClaim(data.settings.autoClaim); }, [data.settings.autoClaim]);
  useEffect(() => { setAutoRefresh(data.settings.autoRefresh ?? true); }, [data.settings.autoRefresh]);
  useEffect(() => { setAutoGrowth(data.settings.autoGrowth ?? false); }, [data.settings.autoGrowth]);
  const growthAccounts = data.accounts.filter((a) => a.platform === "workbuddy" && a.enabled !== false);
  const picked = growthPick ?? new Set(growthAccounts.map((a) => a.id));
  const togglePick = (id: string) => {
    const next = new Set(picked);
    if (next.has(id)) next.delete(id); else next.add(id);
    setGrowthPick(next);
  };
  const runActivation = () => {
    const targets = [...activationPick];
    if (!targets.length) {
      toast.error("请先勾选要激活的账户（每次最多 5 个）");
      return;
    }
    setActivationLog([]);
    void (async () => {
      setActivationRunning(true);
      const lines: string[] = [];
      try {
        const results = await creditsApi.activateAccounts(targets);
        for (const item of results) {
          lines.push(`${item.label}  ${item.outcome}  连登 ${item.streakBefore}→${item.streakAfter}  ${item.note}`);
        }
        setActivationLog(lines.length ? lines : ["没有返回结果"]);
        toast.success("激活任务已执行");
        setActivationPick(new Set());
        await loadGrowthIntel();
      } catch (error) {
        toast.error(errorMessage(error));
        if (lines.length) setActivationLog(lines);
      } finally {
        setActivationRunning(false);
      }
    })();
  };

  const runGrowth = () => {    setGrowthLog([]);
    // Run per selected account so an unchecked account is skipped. Passing no id
    // would run every enabled account, defeating the checkbox list.
    const targets = [...picked];
    if (!targets.length) {
      toast.error("请先勾选要执行的账户");
      return;
    }
    void (async () => {
      const collected: string[] = [];
      setGrowthRunning(true);
      try {
        for (const id of targets) {
          const outcomes = await creditsApi.runGrowth(id);
          for (const outcome of outcomes) collected.push(...outcome.steps);
        }
        setGrowthLog(collected.length ? collected : ["本次没有可领取的成长奖励"]);
        toast.success(`成长中心执行完成（${targets.length} 个账户）`);
        // Refresh the history so the run just performed shows up immediately.
        void loadGrowthIntel();
      } catch (error) {
        toast.error(errorMessage(error));
        if (collected.length) setGrowthLog(collected);
      } finally {
        setGrowthRunning(false);
      }
    })();
  };
  return <div className="space-y-6">
    <div className="flex flex-wrap items-center justify-between gap-3"><p className="text-xs text-slate-500 dark:text-slate-400">各平台分别展示原生额度，领取结果逐项记录。<span className="ml-1 text-slate-400 dark:text-slate-500">排序：最新添加的账号始终排在最前，停用/封禁标红。</span></p><div className="flex flex-wrap gap-2"><Pressable type="button" className="glass-secondary-button text-xs" disabled={refresh.isPending || !data.accounts.length} onClick={() => refresh.mutate(undefined)}><RefreshCw size={14} className={refresh.isPending ? "animate-spin" : ""} />刷新余额</Pressable><Pressable type="button" className="glass-primary-button text-xs" disabled={claim.isPending || !data.accounts.some(isClaimable)} onClick={() => claim.mutate(undefined, { onSuccess: reportClaims })}>{claim.isPending ? <Loader2 size={14} className="animate-spin" /> : <Coins size={14} />}一键领取积分</Pressable></div></div>
    <div className="grid grid-cols-1 items-start gap-4 lg:grid-cols-3">
      {platforms.map((platform) => {
        const all = data.accounts.filter((account) => account.platform === platform.id);
        // 异常账号（封禁/过期/停用）折叠收起，只显示数量。
        // 理由：这些号签不了，却和正常号混在一起占满列表，用户既看不清
        // 又有多少是好号。折叠后正常号一目了然，异常号随时可展开查看。
        // 过几天再展开重测，确认真的不行才清理。
        // 三类分开：
        //   正常  —— 能签到也能调模型，直接铺开
        //   仅可签到 —— 签到正常但推理被封，单独折叠（不占正常列表，也不误报为坏号）
        //   异常  —— 封禁/过期/停用，折叠，隔几天重测再决定清理
        const accounts = all.filter((a) => a.enabled && a.status !== "banned" && a.status !== "expired" && a.status !== "signable");
        const signable = all.filter((a) => a.status === "signable");
        const dead = all.filter((a) => a.status !== "signable" && (!a.enabled || a.status === "banned" || a.status === "expired"));
        // 豆包这类转发型平台，一个账户背后是网关里的一堆号。
        // 数量标签要显示真实号数（7 个），而不是 1 个网关 —— 否则用户会觉得"我的号呢"。
        const subCount = accounts.reduce((sum, a) => sum + (a.snapshot?.subAccounts?.length ?? 0), 0);
        const countLabel = subCount > 0 ? `${subCount} 个账号` : `${accounts.length} 个账户`;
        return <section key={platform.id} className="min-w-0 space-y-3">
          <div className="flex items-center gap-2 px-1"><AgentIcon id={platform.id} name={platform.name} className="h-7 w-7" /><h2 className="text-sm font-semibold">{platform.name}</h2>{signable.length > 0 ? <span className="ml-auto text-xs text-amber-600 dark:text-amber-400">{signable.length} 个仅可签到</span> : null}{dead.length > 0 ? <span className={`text-xs text-rose-600 dark:text-rose-400 ${signable.length > 0 ? "" : "ml-auto"}`}>{dead.length} 个异常</span> : null}<span className={signable.length > 0 || dead.length > 0 ? "text-xs text-slate-500 dark:text-slate-400" : "ml-auto text-xs text-slate-500 dark:text-slate-400"}>{countLabel}</span></div>
          {accounts.length ? <AccountStack accounts={accounts} onDelete={setDeleting} refreshingAccountIds={data.refreshingAccountIds} /> : (dead.length === 0 && signable.length === 0) ? <div className="glass-card flex min-h-[260px] flex-col items-center justify-center p-6 text-center"><KeyRound size={25} className="text-slate-400" /><p className="mt-4 text-sm font-medium">连接 {platform.name}</p><p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">登录后查看余额、领取资格与模型目录。</p><Pressable type="button" className="glass-secondary-button mt-5 text-xs" onClick={() => onAdd(platform.id)}><Plus size={14} />添加账户</Pressable></div> : null}
          {signable.length > 0 ? <SignableAccountsGroup accounts={signable} onDelete={setDeleting} platformLabel={platform.name} /> : null}
          {dead.length > 0 ? <DeadAccountsGroup accounts={dead} onDelete={setDeleting} platformLabel={platform.name} /> : null}
        </section>;
      })}
    </div>
    <section className="glass-card p-5"><h2 className="text-sm font-semibold">同步与领取设置</h2>
      <label aria-label="自动刷新余额" htmlFor="credits-auto-refresh" className="mt-4 flex items-start gap-3"><input id="credits-auto-refresh" type="checkbox" className="mt-0.5 h-4 w-4 accent-blue-500" checked={autoRefresh} onChange={(event) => setAutoRefresh(event.target.checked)} disabled={save.isPending} /><span><span className="block text-sm font-medium">自动刷新余额</span><span className="mt-1 block text-xs leading-relaxed text-slate-500 dark:text-slate-400">默认开启。查看本页时每 5 分钟同步，其他页面或托盘运行时每 15 分钟同步；代理调用后自动补刷。退出应用后停止。</span></span></label>
      <label aria-label="自动领取可用积分" htmlFor="credits-auto-claim" className="mt-4 flex items-start gap-3"><input id="credits-auto-claim" type="checkbox" className="mt-0.5 h-4 w-4 accent-blue-500" checked={autoClaim} onChange={(event) => setAutoClaim(event.target.checked)} disabled={save.isPending} /><span><span className="block text-sm font-medium">自动领取可用积分</span><span className="mt-1 block text-xs leading-relaxed text-slate-500 dark:text-slate-400">开启后仅在 Quoly AI 运行期间，按北京时间每天 08:00 起检查并领取；当天 08:00 之后启动会立刻补做一次，已领取的任务自动跳过。</span></span></label>
      <label aria-label="自动执行成长任务" htmlFor="credits-auto-growth" className="mt-4 flex items-start gap-3"><input id="credits-auto-growth" type="checkbox" className="mt-0.5 h-4 w-4 accent-blue-500" checked={autoGrowth} onChange={(event) => setAutoGrowth(event.target.checked)} disabled={save.isPending} /><span><span className="block text-sm font-medium">自动执行成长任务</span><span className="mt-1 block text-xs leading-relaxed text-slate-500 dark:text-slate-400">默认关闭。跟随「自动领取」同一次检查执行 WorkBuddy 成长中心：领回旅行礼物、接取新任务、领取已完成任务的奖励、连登兑换、补登、抽奖、开盲盒，最后派出旅行。任务进度由平台按真实使用记账，这里只接取和领取，不代为完成。</span></span></label>
      <Pressable type="button" className="glass-secondary-button mt-4 text-xs" disabled={!changed || save.isPending} onClick={() => save.mutate({ ...data.settings, autoClaim, autoRefresh, autoGrowth }, { onSuccess: () => toast.success("设置已保存") })}>{save.isPending ? <Loader2 size={13} className="animate-spin" /> : null}保存设置</Pressable>
    </section>
    <section className="glass-card p-5"><h2 className="text-sm font-semibold">成长中心（WorkBuddy）</h2>
      <p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">手动执行一次完整流程：领回旅行礼物 → 领已完成任务奖励 → 接取新任务 → 连登兑换 / 补登 → 抽奖 → 开盲盒 → 派出旅行。任务进度由平台按真实使用记账，这里不会代替你完成任务。</p>
      {growthAccounts.length ? <div className="mt-4 space-y-1.5">{growthAccounts.map((account) => {
        const state = growthStates?.find((item) => item.id === account.id);
        return <label key={account.id} className="flex flex-wrap items-center gap-2 text-xs">
          <input type="checkbox" className="h-3.5 w-3.5 accent-blue-500" checked={picked.has(account.id)} onChange={() => togglePick(account.id)} />
          <span className="min-w-24 truncate font-medium">{account.label || account.userId}</span>
          {state ? <>
            <Badge success={state.ready}>{state.ready ? "可领奖" : "未领养 Buddy"}</Badge>
            {state.ready ? <span className="text-[11px] text-slate-500 dark:text-slate-400">能量 {state.energy} · 已领 {state.claimedTasks} 个任务{state.travelState === "traveling" ? " · 旅行中" : state.travelState === "arrived" ? " · 已返回待领" : ""}</span> : <span className="text-[11px] text-amber-700 dark:text-amber-300">{state.hint}</span>}
          </> : null}
        </label>;
      })}</div> : <p className="mt-4 text-xs text-slate-500 dark:text-slate-400">还没有 WorkBuddy 账户。</p>}
      <div className="mt-4 flex flex-wrap gap-2">
        <Pressable type="button" className="glass-primary-button text-xs" disabled={growthRunning || !picked.size} onClick={runGrowth}>{growthRunning ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}立即执行成长任务</Pressable>
        <Pressable type="button" className="glass-secondary-button text-xs" disabled={growthRunning || picked.size === growthAccounts.length} onClick={() => setGrowthPick(null)}>全选</Pressable>
        <Pressable type="button" className="glass-secondary-button text-xs" disabled={loadingIntel} onClick={() => void loadGrowthIntel()}>{loadingIntel ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}刷新账号状态</Pressable>
      </div>
      {growthLog.length ? <div className="glass-code mt-4 max-h-64 space-y-1 overflow-auto rounded-xl px-3 py-2.5 text-[11px] leading-relaxed">{growthLog.map((line, index) => <p key={index} className="break-words">{line}</p>)}</div> : null}
    </section>
    <section className="glass-card p-5"><h2 className="text-sm font-semibold">激活未领养 Buddy 的账户</h2>
      <p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">没有 Buddy 的账户领不到任何成长奖励（平台会拒绝任务接取）。这里对选中的账户各上报一条客户端活跃事件并完成领养，单账号每天只发 1 次，账号间隔 3~6 分钟随机。建议先用 3 个账户运行 2 天，确认账户正常后再逐个开放。</p>
      {(() => {
        const pending = (growthStates ?? []).filter((item) => !item.ready);
        if (!growthStates) return <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">点上方「刷新账号状态」查看哪些账户需要激活。</p>;
        if (!pending.length) return <p className="mt-3 text-xs text-emerald-700 dark:text-emerald-300">全部 WorkBuddy 账户都已领养 Buddy，无需激活。</p>;
        return <div className="mt-3 space-y-1.5">{pending.map((item) => <label key={item.id} className="flex items-center gap-2 text-xs">
          <input type="checkbox" className="h-3.5 w-3.5 accent-amber-500" checked={activationPick.has(item.id)} onChange={() => {
            const next = new Set(activationPick);
            if (next.has(item.id)) next.delete(item.id);
            else if (next.size < 5) next.add(item.id);
            else toast.error("单次最多选 5 个");
            setActivationPick(next);
          }} />
          <span className="min-w-24 truncate font-medium">{item.label}</span>
          <span className="text-[11px] text-amber-700 dark:text-amber-300">{item.hint}</span>
        </label>)}</div>;
      })()}
      <div className="mt-4 flex flex-wrap gap-2">
        <Pressable type="button" className="glass-primary-button text-xs" disabled={activationRunning || !activationPick.size} onClick={runActivation}>{activationRunning ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}激活选中账户{activationPick.size ? `（${activationPick.size}）` : ""}</Pressable>
        {activationRunning ? <span className="self-center text-[11px] text-slate-500 dark:text-slate-400">执行中，账号间隔 3~6 分钟，请勿关闭窗口</span> : null}
      </div>
      {activationLog.length ? <div className="glass-code mt-4 space-y-1 rounded-xl px-3 py-2.5 text-[11px] leading-relaxed">{activationLog.map((line, index) => <p key={index} className="break-words">{line}</p>)}</div> : null}
    </section>
    <section className="glass-card p-5"><div className="flex flex-wrap items-center justify-between gap-3"><h2 className="text-sm font-semibold">每日领取记录</h2><span className="text-[11px] text-slate-500 dark:text-slate-400">按北京时间自然日统计，最近 14 天</span></div>
      {growthDays === null ? <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">点上方「刷新账号状态」加载每日记录。</p> : growthDays.some((day) => day.runs > 0) ? <div className="mt-3 overflow-x-auto"><table className="w-full text-left text-xs"><thead className="text-[11px] text-slate-500 dark:text-slate-400"><tr><th className="py-2 pr-4 font-medium">日期</th><th className="py-2 pr-4 font-medium">积分</th><th className="py-2 pr-4 font-medium">账号数</th><th className="py-2 pr-4 font-medium">任务领取</th><th className="py-2 pr-4 font-medium">旅行奖励</th><th className="py-2 font-medium">盲盒</th></tr></thead><tbody>{growthDays.filter((day) => day.runs > 0).map((day) => <tr key={day.date} className="border-t border-slate-500/10"><td className="py-2 pr-4 tabular-nums">{day.date}</td><td className="py-2 pr-4 tabular-nums font-medium">{number(day.credits)}</td><td className="py-2 pr-4 tabular-nums">{day.accounts}</td><td className="py-2 pr-4 tabular-nums">{day.tasksClaimed}</td><td className="py-2 pr-4 tabular-nums">{day.travelClaimed}</td><td className="py-2 tabular-nums">{day.buddyBoxes}</td></tr>)}</tbody></table></div> : <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">最近 14 天还没有成长中心执行记录。</p>}
    </section>
    <section className="glass-card p-5"><h2 className="text-sm font-semibold">最近领取</h2>{data.rewards.length ? <div className="mt-4 divide-y divide-slate-500/10">{data.rewards.slice(0, 12).map((outcome) => <div key={`${outcome.accountId}-${outcome.rewardId}-${outcome.createdAt}-${outcome.status}`} className="flex flex-wrap items-center justify-between gap-3 py-3 text-xs"><div className="min-w-0"><p className="font-medium">{outcome.name}{outcome.amount != null && isCredits(outcome.kind) ? ` · +${number(outcome.amount)}` : ""}</p><p className="mt-1 break-words text-slate-500 dark:text-slate-400">{data.accounts.find((account) => account.id === outcome.accountId)?.label || "账户"} · {outcome.message}</p></div><div className="flex shrink-0 items-center gap-3"><span className="text-[11px] text-slate-500 dark:text-slate-400">{time(outcome.createdAt)}</span><Badge success={["success", "claimed"].includes(outcome.status)}>{outcome.status === "pending" ? "待确认" : labels[outcome.status] ?? outcome.status}</Badge></div></div>)}</div> : <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">还没有领取记录。</p>}</section>
    {deleting ? <Modal open onClose={() => !remove.isPending && setDeleting(null)} size="sm" dismissOnBackdrop={!remove.isPending}><div className="p-6"><h2 className="text-base font-semibold">移除账户</h2><p className="mt-3 break-words text-sm leading-relaxed text-slate-500 dark:text-slate-400">移除“{deleting.label || platformName(deleting.platform)}”及本地保存的凭据，后续代理请求将不再使用此账户。</p><div className="mt-6 flex justify-end gap-2"><Pressable type="button" className="glass-secondary-button text-xs" disabled={remove.isPending} onClick={() => setDeleting(null)}>取消</Pressable><Pressable type="button" className="glass-danger-button text-xs" disabled={remove.isPending} onClick={() => remove.mutate(deleting.id, { onSuccess: () => setDeleting(null) })}>{remove.isPending ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}移除</Pressable></div></div></Modal> : null}
  </div>;
}

function ProxyView({ data }: { data: CreditDashboard }) {
  const start = useCreditsAction(creditsApi.startProxy);
  const stop = useCreditsAction(creditsApi.stopProxy);
  const probe = useCreditsAction(creditsApi.probeProxy);
  // 保存勾选的模型清单。用 spread 合并现有设置，只改 sharedModels ——
  // 早先这里漏传字段会把 lanShare 一起清掉，等于把局域网共享悄悄关掉。
  const saveSettings = useCreditsAction(creditsApi.saveSettings);
  // 缓存住：它会展开 3,459 个（账号，模型）组合（158 个账号 × 各自模型目录），
  // 每次 render 重算一遍纯属浪费。data 不变就不该重算。
  const models = useMemo(() => availableModels(data), [data]);
  const [showingCatalog, setShowingCatalog] = useState(false);
  const catalogGroups: ModelGroup[] = platforms
    .flatMap((platform) => (["cn", "intl"] as const).map((region) => ({ title: `${platform.name} · ${regionName(region)}`, entries: models.filter(({ account }) => account.platform === platform.id && (account.region ?? "cn") === region) })))
    .filter((group) => group.entries.length);
  return <div className="space-y-5">
    <section className="glass-card p-6"><div className="flex flex-wrap items-center justify-between gap-4"><div className="flex items-center gap-3"><span className={`flex h-11 w-11 items-center justify-center rounded-xl ${data.gateway.running ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-300" : "bg-slate-500/10 text-slate-500"}`}><Server size={22} /></span><div><h2 className="text-base font-semibold">本地模型代理</h2><p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{data.gateway.running ? "正在运行 · 关闭窗口后继续运行" : "已停止 · 手动开启后供其他工具使用"}</p></div></div><Pressable type="button" className={data.gateway.running ? "glass-secondary-button text-sm" : "glass-primary-button text-sm"} disabled={start.isPending || stop.isPending} onClick={() => data.gateway.running ? stop.mutate(undefined) : start.mutate(undefined)}>{start.isPending || stop.isPending ? <Loader2 size={15} className="animate-spin" /> : <Power size={15} />}{data.gateway.running ? "停止代理" : "一键开启代理"}</Pressable></div>
      <p className="mt-3 text-xs leading-5 text-slate-500 dark:text-slate-400">关闭窗口后应用留在系统托盘，可从托盘恢复主窗口；选择“退出应用（停止代理）”会停止代理并完全退出。</p>
      {data.gateway.lastError ? <p role="alert" className="mt-4 rounded-lg bg-amber-500/10 p-3 text-xs text-amber-800 dark:text-amber-200">{data.gateway.lastError}</p> : null}
      <p className="mt-5 text-xs leading-relaxed text-slate-500 dark:text-slate-400">在客户端的自定义提供商设置中填写下方连接参数，再添加一个目录模型 ID。保持 Quoly AI 与代理运行即可使用。</p>
      <div className="mt-5 grid gap-5 md:grid-cols-2"><CopyField label="提供商名称" value="Quoly AI" /><CopyField label="Base URL" value={data.gateway.baseUrl} /></div>
      <div className="mt-5 grid gap-5 md:grid-cols-2"><div className="space-y-2"><p className="text-xs font-medium text-slate-500 dark:text-slate-400">协议</p><div className="glass-code space-y-2 rounded-xl px-3 py-2.5 text-xs"><p>OpenAI Chat Completions <code className="block break-all text-[11px] text-slate-500 dark:text-slate-400">/v1/chat/completions</code></p><p>Anthropic Messages <code className="block break-all text-[11px] text-slate-500 dark:text-slate-400">/v1/messages</code></p></div><p className="text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">两种协议使用同一 Base URL，在客户端选择对应协议。</p></div><div className="space-y-2"><p className="text-xs font-medium text-slate-500 dark:text-slate-400">API Key</p><div className="glass-code flex min-h-[44px] items-center justify-between gap-3 rounded-xl px-3 py-2.5"><span className="text-xs text-slate-500 dark:text-slate-400">使用本地代理密钥</span><CopyProxyKey /></div></div></div>
      <div className="mt-4 flex flex-wrap gap-2"><Pressable type="button" className="glass-secondary-button text-xs" disabled={!data.gateway.running || probe.isPending} onClick={() => probe.mutate(undefined, { onSuccess: (message) => toast.success(message) })}>{probe.isPending ? <Loader2 size={13} className="animate-spin" /> : <CheckCircle2 size={13} />}检测连接</Pressable><CopyAllProxyConfig baseUrl={data.gateway.baseUrl} lanUrl={data.gateway.lanUrl} models={models} saved={data.settings.sharedModels ?? []} running={data.gateway.running} onSave={async (ids) => { await saveSettings.mutateAsync({ ...data.settings, sharedModels: ids }); toast.success(`已记住 ${ids.length} 个模型的选择`); }} /></div>
    </section>
    <section className="glass-card p-6"><div className="flex flex-wrap items-start justify-between gap-4"><div className="min-w-0"><h2 className="text-sm font-semibold">目录模型 ID</h2><p className="mt-2 text-xs leading-relaxed text-slate-500 dark:text-slate-400">平台模型 ID 来自已连接账户的目录，可以指定对应平台调用；上游推理的可用性以实际请求结果为准。</p></div><Pressable type="button" className="glass-secondary-button shrink-0 text-xs" onClick={() => setShowingCatalog(true)}>查看模型 · {models.length}</Pressable></div><p className="mt-4 text-[11px] leading-relaxed text-slate-500 dark:text-slate-400">模型 ID 固定平台与区域，国际版使用 -intl 前缀；失败不会切换到国内账号。</p>{showingCatalog ? <ModelsModal title="目录模型 ID" groups={catalogGroups} onClose={() => setShowingCatalog(false)} /> : null}</section>
    <section className="glass-card p-6"><h2 className="text-sm font-semibold">最近请求</h2>{data.requests.length ? <div className="mt-3 overflow-x-auto"><table className="w-full text-left text-xs"><thead className="text-[11px] text-slate-500 dark:text-slate-400"><tr><th className="py-2 pr-4 font-medium">模型 / 平台</th><th className="py-2 pr-4 font-medium">耗时</th><th className="py-2 pr-4 font-medium">Tokens 输入 / 输出</th><th className="py-2 pr-4 font-medium">状态</th><th className="py-2 font-medium">时间</th></tr></thead><tbody>{data.requests.slice(0, 15).map((request) => <tr key={request.id} className="border-t border-slate-500/10"><td className="max-w-[260px] py-3 pr-4"><p className="break-all font-medium">{request.model}</p><p className="mt-1 text-[11px] text-slate-500 dark:text-slate-400">{platformName(request.platform)} · {regionName(request.region)} · {data.accounts.find((account) => account.id === request.accountId)?.label || request.accountId}</p></td><td className="whitespace-nowrap py-3 pr-4 tabular-nums">{number(request.elapsedMs)} ms</td><td className="whitespace-nowrap py-3 pr-4 tabular-nums">{request.inputTokens == null ? "—" : number(request.inputTokens)} / {request.outputTokens == null ? "—" : number(request.outputTokens)}</td><td className="py-3 pr-4"><Badge success={["success", "completed"].includes(request.status)}>{labels[request.status] ?? request.status}</Badge>{request.diagnostic ? <details className="mt-2 min-w-[180px] max-w-[340px] text-[11px]"><summary className="cursor-pointer text-blue-600 dark:text-blue-300">错误详情</summary><p className="mt-2 break-words leading-relaxed text-slate-600 dark:text-slate-300">{request.diagnostic.message}</p><p className="mt-2 break-all text-slate-500 dark:text-slate-400">代理请求 ID：{request.id}</p></details> : null}</td><td className="whitespace-nowrap py-3 text-[11px] text-slate-500 dark:text-slate-400">{time(request.createdAt)}</td></tr>)}</tbody></table></div> : <p className="mt-3 text-xs text-slate-500 dark:text-slate-400">开启代理并从客户端调用后，显示平台、耗时和用量。</p>}</section>
  </div>;
}

function CopyProxyKey() {
  const [copying, setCopying] = useState(false);
  const copy = async () => {
    setCopying(true);
    try { await copyValue(await creditsApi.proxyKey(), "代理 API Key"); }
    catch (error) { toast.error(errorMessage(error)); }
    finally { setCopying(false); }
  };
  return <Pressable type="button" className="glass-secondary-button min-h-7 shrink-0 px-2 py-1 text-xs" disabled={copying} onClick={() => void copy()}>{copying ? <Loader2 size={12} className="animate-spin" /> : <Copy size={12} />}复制 API Key</Pressable>;
}

/**
 * 一键复制本地代理的整套连接参数。
 *
 * 为什么要它：手动配客户端时要抄三样东西（Base URL、协议、API Key），
 * 还容易漏掉"局域网地址"这个变量。一次复制成一段可直接粘贴的文本更省事。
 *
 * 复制内容刻意做成"能直接读懂"的格式而不是 JSON：用户多半是往 Claude Code、
 * Cline 这类工具的自定义提供商表单里填，边看边填比解析 JSON 顺手。
 */
/**
 * 一键复制连接参数 —— 带模型勾选。
 *
 * 为什么需要勾选：用户拿这段配置是发给别人的，让对方照着填。如果只给地址，
 * 对方还得自己问"配哪个模型"。把选好的模型清单一起给他，一句话就能说清。
 *
 * 选择永久记住（存在 ProxySettings.sharedModels）：这是用户的配置资产，
 * 每次重新勾一遍太烦。第二次点按钮时若已有保存的清单，直接复制不再弹窗 ——
 * 这正是用户要的"以后点一下就复制"。
 *
 * 想改选择时点旁边的「选择模型」小链接，或按住 Alt 点主按钮。
 */
function CopyAllProxyConfig({ baseUrl, lanUrl, models, saved, onSave, running }: {
  baseUrl: string;
  lanUrl: string | null;
  models: ModelEntry[];
  saved: string[];
  onSave: (ids: string[]) => Promise<void>;
  running: boolean;
}) {
  const [picking, setPicking] = useState(false);
  const [copying, setCopying] = useState(false);

  // 按平台分组，供弹窗折叠展示
  const groups = platforms
    .map((platform) => ({
      id: platform.id,
      name: platform.name,
      entries: models.filter(({ account }) => account.platform === platform.id),
    }))
    .filter((group) => group.entries.length);

  const build = async (selected: string[]) => {
    const key = await creditsApi.proxyKey();
    const chosen = models.filter(({ model }) => selected.includes(modelKey(model)));
    // 按平台归组，方便对方逐个平台配
    const byPlatform = new Map<string, string[]>();
    for (const { account, model } of chosen) {
      const name = platformName(account.platform);
      byPlatform.set(name, [...(byPlatform.get(name) ?? []), model.id]);
    }
    const modelLines = byPlatform.size
      ? [...byPlatform.entries()].flatMap(([name, ids]) => [`  ${name}：`, ...ids.map((id) => `    - ${id}`)])
      : ["  （未选择模型）"];
    const lines = [
      `提供商名称：Quoly AI`,
      `Base URL：${baseUrl}`,
      lanUrl ? `局域网地址：${lanUrl}` : `局域网地址：未开启（代理仅监听本机）`,
      `协议：OpenAI Chat Completions（/v1/chat/completions）`,
      `　　　Anthropic Messages（/v1/messages）`,
      `API Key：${key}`,
      ``,
      `需要配置的模型（共 ${chosen.length} 个）：`,
      ...modelLines,
      ``,
      `（两种协议共用同一 Base URL，在客户端选择对应协议即可）`,
    ];
    await copyValue(lines.join("\n"), `代理连接参数（含 ${chosen.length} 个模型）`);
  };

  const copyNow = async (selected: string[]) => {
    setCopying(true);
    try {
      await build(selected);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setCopying(false);
    }
  };

  // 已经选过 → 直接复制；没选过 → 弹窗选
  const primary = async () => {
    if (!running) {
      toast.error("代理还没开启，先点「一键开启代理」");
      return;
    }
    if (!saved.length) {
      setPicking(true);
      return;
    }
    await copyNow(saved);
  };

  return <>
    <Pressable type="button" className="glass-primary-button text-xs" disabled={copying} onClick={() => void primary()}>
      {copying ? <Loader2 size={13} className="animate-spin" /> : <Copy size={13} />}
      一键复制连接参数
    </Pressable>
    <Pressable type="button" className="glass-secondary-button text-xs" disabled={copying} onClick={() => setPicking(true)}>
      <SlidersHorizontal size={13} />
      {saved.length ? `已选 ${saved.length} 个模型` : "选择模型"}
    </Pressable>
    {picking ? <ModelPickerModal
      groups={groups}
      initial={saved}
      onCancel={() => setPicking(false)}
      onConfirm={async (ids) => {
        setPicking(false);
        await onSave(ids);
        await copyNow(ids);
      }}
    /> : null}
  </>;
}

/** 复制内容里的模型唯一键。跨平台同名模型必须区分，否则勾选会串。 */
function modelKey(model: CreditModel) {
  return model.id;
}

/**
 * 模型勾选弹窗。四个平台默认折叠，展开后逐个勾选。
 *
 * 折叠而不是全展开：四个平台加起来 86 个模型，全摊开要滚半屏，
 * 用户找某个平台的模型很累。
 */
function ModelPickerModal({ groups, initial, onCancel, onConfirm }: {
  groups: Array<{ id: CreditPlatform; name: string; entries: ModelEntry[] }>;
  initial: string[];
  onCancel: () => void;
  onConfirm: (ids: string[]) => Promise<void>;
}) {
  const [selected, setSelected] = useState<Set<string>>(new Set(initial));
  const [open, setOpen] = useState<Set<string>>(new Set());
  const toggleModel = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };
  const toggleGroup = (id: string) => {
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };
  // 整组勾选/取消：展开后一次点掉一个平台的全部模型
  const toggleAll = (entries: ModelEntry[], on: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev);
      for (const { model } of entries) {
        if (on) next.add(modelKey(model)); else next.delete(modelKey(model));
      }
      return next;
    });
  };
  return <Modal open onClose={onCancel} size="xl">
    <div className="flex items-center justify-between gap-3 border-b border-slate-500/10 px-6 py-4">
      <div><h2 className="text-base font-semibold">选择要分享的模型</h2><p className="mt-1 text-[11px] text-slate-500 dark:text-slate-400">勾选的清单会随连接参数一起复制，并永久记住。</p></div>
      <Pressable type="button" className="glass-icon-button" onClick={onCancel} aria-label="关闭模型选择"><X size={18} /></Pressable>
    </div>
    <div className="max-h-[62vh] space-y-3 overflow-y-auto p-6">
      {groups.length ? groups.map((group) => {
        const expanded = open.has(group.id);
        const all = group.entries.map(({ model }) => modelKey(model));
        const picked = all.filter((id) => selected.has(id)).length;
        return <section key={group.id} className="rounded-xl border border-slate-500/10">
          <div className="flex items-center gap-2 px-3 py-2.5">
            <Pressable type="button" noScale className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => toggleGroup(group.id)} aria-expanded={expanded}>
              <ChevronRight size={14} className={`shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`} />
              <AgentIcon id={group.id} name={group.name} className="h-5 w-5" />
              <span className="text-sm font-medium">{group.name}</span>
              <span className="text-[11px] text-slate-500 dark:text-slate-400">{picked}/{all.length} 已选</span>
            </Pressable>
            <Pressable type="button" className="glass-secondary-button min-h-6 shrink-0 px-2 py-0.5 text-[11px]" onClick={() => toggleAll(group.entries, picked !== all.length)}>
              {picked === all.length ? "全不选" : "全选"}
            </Pressable>
          </div>
          {expanded ? <div className="border-t border-slate-500/10 p-2">
            {group.entries.map(({ account, model }) => {
              const id = modelKey(model);
              const checked = selected.has(id);
              return <label key={`${account.id}-${model.id}`} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-xs hover:bg-slate-500/5">
                <input type="checkbox" className="h-3.5 w-3.5 accent-blue-500" checked={checked} onChange={() => toggleModel(id)} />
                <code className="min-w-0 flex-1 break-all">{model.id}</code>
              </label>;
            })}
          </div> : null}
        </section>;
      }) : <p className="text-xs text-slate-500 dark:text-slate-400">还没有可用模型，先连接平台账户并刷新。</p>}
    </div>
    <div className="flex items-center justify-between gap-3 border-t border-slate-500/10 px-6 py-4">
      <span className="text-xs text-slate-500 dark:text-slate-400">已选 {selected.size} 个模型</span>
      <div className="flex gap-2">
        <Pressable type="button" className="glass-secondary-button text-xs" onClick={onCancel}>取消</Pressable>
        <Pressable type="button" className="glass-primary-button text-xs" disabled={!selected.size} onClick={() => void onConfirm([...selected])}>
          <Copy size={13} />确定并复制
        </Pressable>
      </div>
    </div>
  </Modal>;
}

export default function CreditsPanel() {
  const [tab, setTab] = useState<PanelTab>("accounts");
  const [adding, setAdding] = useState<CreditPlatform | null>(null);
  const dashboard = useCreditsDashboard();
  const closeAdd = useCallback(() => setAdding(null), []);
  const moveTab = (key: string) => {
    const current = sections.findIndex((section) => section.id === tab);
    const index = key === "ArrowRight" ? (current + 1) % sections.length
      : key === "ArrowLeft" ? (current + sections.length - 1) % sections.length
      : key === "Home" ? 0 : key === "End" ? sections.length - 1 : -1;
    if (index < 0) return false;
    const next = sections[index].id;
    setTab(next);
    document.getElementById(`credits-tab-${next}`)?.focus();
    return true;
  };
  return <div className="glass-app flex h-full flex-col overflow-hidden">
    <div className="glass-header"><div className="flex flex-wrap items-start justify-between gap-4"><div><div className="glass-kicker"><Coins size={13} />Credits & Models</div><h1 className="mt-3 text-display">积分与代理</h1><p className="mt-2 text-caption text-slate-500 dark:text-slate-400">管理平台积分，将平台目录模型接入你的编程工具。</p></div><Pressable type="button" className="glass-primary-button mt-1 text-xs" onClick={() => setAdding("workbuddy")}><Plus size={15} />添加账户</Pressable></div><div className="mt-5 flex flex-wrap gap-2" role="tablist" aria-label="积分与代理功能">{sections.map((item) => <Pressable key={item.id} id={`credits-tab-${item.id}`} type="button" role="tab" aria-selected={tab === item.id} aria-controls="credits-tab-panel" tabIndex={tab === item.id ? 0 : -1} onKeyDown={(event) => { if (moveTab(event.key)) event.preventDefault(); }} className={tab === item.id ? "glass-primary-button min-h-8 px-4 py-2 text-xs" : "glass-secondary-button min-h-8 px-4 py-2 text-xs"} onClick={() => setTab(item.id)}>{item.label}</Pressable>)}</div></div>
    <div className="glass-content" role="tabpanel" id="credits-tab-panel" aria-labelledby={`credits-tab-${tab}` }>
      {dashboard.isLoading ? <div className="flex min-h-[280px] items-center justify-center gap-3 text-sm text-slate-500"><Loader2 size={20} className="animate-spin" />正在读取账户与代理状态…</div> : dashboard.isError ? <div className="glass-card p-6"><h2 className="text-sm font-semibold">读取失败</h2><p role="alert" className="mt-3 break-words text-xs text-slate-500 dark:text-slate-400">{errorMessage(dashboard.error)}</p><Pressable type="button" className="glass-secondary-button mt-4 text-xs" onClick={() => void dashboard.refetch()} disabled={dashboard.isFetching}><RefreshCw size={13} className={dashboard.isFetching ? "animate-spin" : ""} />重试</Pressable></div> : dashboard.data ? tab === "accounts" ? <AccountsView data={dashboard.data} onAdd={setAdding} /> : tab === "proxy" ? <ProxyView data={dashboard.data} /> : <ModelHealthView proxyRunning={dashboard.data.gateway.running} /> : null}
    </div>
    {adding ? <AddAccountModal platform={adding} onClose={closeAdd} /> : null}
  </div>;
}
