import { useState, useEffect, lazy, Suspense } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Toaster, toast } from "sonner";
import UnifiedMcpPanel from "@/components/mcp/UnifiedMcpPanel";
import UpdateModal from "@/components/mcp/UpdateModal";
import SkillsPanel from "@/components/skills/SkillsPanel";
import HomePanel from "@/components/home/HomePanel";
import { SettingsPanel } from "@/components/settings/SettingsPanel";
import { Pressable } from "@/components/ui/Pressable";
import {
  Database,
  Settings,
  Info,
  Loader2,
  Github,
  ExternalLink,
  Sparkles,
  Coins,
  RefreshCw,
  History,
  LayoutDashboard,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-shell";
import { useAppVersion } from "@/hooks/useAppVersion";
import { useTheme } from "@/hooks/useTheme";
import { useReducedMotion } from "@/hooks/useReducedMotion";
import { useCreditsRuntime } from "@/hooks/useCredits";
import { useInstalledTools } from "@/contexts/InstalledToolsContext";
import { updateApi } from "@/lib/api";
import { creditsApi } from "@/lib/api/credits";
import FloatingNotice, { type FloatingNoticeData } from "@/components/ui/FloatingNotice";
import appLogo from "../src-tauri/icons/128x128.png";

type Tab = "home" | "mcp" | "skills" | "credits" | "changelog" | "settings" | "about";
const CreditsPanel = lazy(() => import("@/components/credits/CreditsPanel"));
const ChangelogPanel = lazy(() => import("@/components/changelog/ChangelogPanel"));
const GITHUB_REPO_URL = "https://github.com/zenolore/quoly-ai";
const OFFICIAL_WEBSITE_URL = "https://luforge.com";
let startupUpdateCheckStarted = false;


function App() {
  const [activeTab, setActiveTab] = useState<Tab>("home");
  useCreditsRuntime(activeTab === "credits" || activeTab === "home");
  const [startupUpdateInfo, setStartupUpdateInfo] = useState<{
    version: string;
    body: string;
  } | null>(null);
  const [showStartupUpdateModal, setShowStartupUpdateModal] = useState(false);
  const [startupInstalling, setStartupInstalling] = useState(false);
  const [notice, setNotice] = useState<FloatingNoticeData | null>(null);
  const appVersion = useAppVersion();
  const theme = useTheme();
  const reduced = useReducedMotion();
  const { installedAgents, isLoading: scanningAgents, refresh: scanAgents } = useInstalledTools();

  // 启动时自己确保本地代理在跑：原先由外部守护脚本用 CDP + 系统 MessageBox
  // 弹窗完成，会阻塞并需要点“确定”。改为应用内自动启动，成功后只在窗口
  // 左侧浮一个会自动消失的提示。
  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(async () => {
      try {
        const before = await creditsApi.dashboard();
        if (cancelled) return;
        if (before.gateway.running) return;

        const status = await creditsApi.startProxy();
        if (cancelled || !status.running) return;

        const lanHint = status.baseUrl.replace("127.0.0.1", "<本机IP>");
        setNotice({
          id: Date.now(),
          title: "本地代理已自动启动",
          lines: [`本机  ${status.baseUrl}`, `局域网  ${lanHint}`],
          tone: "success",
        });
      } catch {
        // 启动失败不打扰用户，可在“积分与代理”页手动开启。
      }
    }, 1200);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, []);

  // 首次打开应用时自动检查新版本，有更新时交给用户决定是否安装。
  useEffect(() => {
    if (startupUpdateCheckStarted) return;
    startupUpdateCheckStarted = true;

    let cancelled = false;

    const checkStartupUpdate = async () => {
      try {
        const result = await updateApi.checkUpdate();

        if (!cancelled && result.available) {
          setStartupUpdateInfo({
            version: result.version,
            body: result.body || "",
          });
          setShowStartupUpdateModal(true);
        }
      } catch (err) {
        console.error("启动时检查更新失败:", err);
      }
    };

    checkStartupUpdate();

    return () => {
      cancelled = true;
    };
  }, []);

  const installStartupUpdate = async () => {
    setStartupInstalling(true);
    try {
      await updateApi.installUpdate();
      toast.success("更新下载完成，正在重启应用...");
    } catch (err) {
      console.error("安装更新失败:", err);
      toast.error(`安装更新失败: ${err}`);
    } finally {
      setStartupInstalling(false);
    }
  };

  const navItems = [
    { id: "home" as Tab, label: "总览", icon: LayoutDashboard },
    { id: "skills" as Tab, label: "Skills 管理", icon: Sparkles },
    { id: "mcp" as Tab, label: "MCP 服务器", icon: Database },
    { id: "credits" as Tab, label: "积分与代理", icon: Coins },
    { id: "changelog" as Tab, label: "更新日志", icon: History },
    { id: "settings" as Tab, label: "设置", icon: Settings },
    { id: "about" as Tab, label: "关于", icon: Info },
  ];

  return (
    <div className="glass-app flex h-full">
      {/* 侧边栏 */}
      <aside className="glass-sidebar z-10 flex w-[240px] flex-col border-y-0 border-l-0">
        {/* Logo：点击打开官网 */}
        <div className="px-6 pt-6 pb-5">
          <div className="flex items-center gap-3">
            <Pressable
              onClick={() =>
                void open(OFFICIAL_WEBSITE_URL).catch(() => toast.error("无法打开官网"))
              }
              aria-label="打开 Quoly AI 官网"
              title="打开 Quoly AI 官网"
              className="rounded-2xl"
            >
              <img src={appLogo} alt="" className="h-11 w-11 rounded-2xl" />
            </Pressable>
            <h1 className="text-base font-semibold tracking-tight text-slate-950 dark:text-white">
              Quoly AI
            </h1>
          </div>
        </div>

        {/* 导航 */}
        <nav className="relative flex-1 px-3 py-2 space-y-1">
          {navItems.map((item) => {
            const active = activeTab === item.id;
            return (
              <Pressable
                key={item.id}
                onClick={() => setActiveTab(item.id)}
                aria-current={active ? "page" : undefined}
                className={`relative z-10 flex min-h-11 w-full items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-medium transition-colors duration-200 ease-out ${
                  active
                    ? "text-white"
                    : "text-slate-500 hover:text-slate-950 dark:text-slate-400 dark:hover:text-white"
                }`}
              >
                {active && (
                  <motion.span
                    layoutId="sidebar-pill"
                    className="absolute inset-0 -z-10 rounded-xl bg-gradient-to-r from-[#0A84FF] to-[#5AC8FA] shadow-[0_8px_24px_rgba(10,132,255,0.28)]"
                    transition={{ type: "spring", bounce: 0.15, duration: 0.45 }}
                  />
                )}
                <item.icon size={18} />
                <span>{item.label}</span>
              </Pressable>
            );
          })}
        </nav>

        <div className="mx-3 mb-3 rounded-xl border border-white/40 bg-white/30 px-3 py-3 dark:border-white/10 dark:bg-white/5">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs text-slate-500 dark:text-slate-400" role="status">
              {scanningAgents ? "正在扫描本地 Agent…" : `已安装 ${installedAgents.length} 个 Agent`}
            </span>
            <Pressable
              variant="icon"
              className="glass-icon-button"
              aria-label="扫描本地 Agent"
              title="扫描本地 Agent"
              disabled={scanningAgents}
              onClick={scanAgents}
            >
              <RefreshCw size={14} className={scanningAgents ? "animate-spin" : ""} />
            </Pressable>
          </div>
          <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">仅展示本地已安装的 Agent</p>
        </div>

        {/* 版本 */}
        <div className="border-t border-white/40 px-6 py-4 text-center dark:border-white/10">
          <p className="text-[11px] font-medium tracking-wide text-slate-400 dark:text-slate-500">
            v{appVersion}
          </p>
          <p className="mt-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
            本地补丁版 · 签到自动补领
          </p>
        </div>
      </aside>

      {/* 主内容区 */}
      <main className="flex-1 overflow-hidden">
        <AnimatePresence mode="wait">
          <motion.div
            key={activeTab}
            initial={reduced ? { opacity: 0 } : { opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reduced ? { opacity: 0 } : { opacity: 0, y: -6 }}
            transition={{ type: "spring", bounce: 0, duration: reduced ? 0.18 : 0.25 }}
            className="h-full"
          >
            {activeTab === "home" && (
              <Suspense fallback={<div className="flex h-full items-center justify-center gap-3 text-sm text-slate-500"><Loader2 size={20} className="animate-spin" />正在加载总览…</div>}>
                <HomePanel />
              </Suspense>
            )}
            {activeTab === "skills" && <SkillsPanel />}
            {activeTab === "mcp" && <UnifiedMcpPanel />}
            {activeTab === "credits" && (
              <Suspense fallback={<div className="flex h-full items-center justify-center gap-3 text-sm text-slate-500"><Loader2 size={20} className="animate-spin" />正在加载积分与代理…</div>}>
                <CreditsPanel />
              </Suspense>
            )}
            {activeTab === "changelog" && (
              <Suspense fallback={<div className="flex h-full items-center justify-center gap-3 text-sm text-slate-500"><Loader2 size={20} className="animate-spin" />正在加载更新日志…</div>}>
                <ChangelogPanel />
              </Suspense>
            )}
            {activeTab === "settings" && <SettingsPanel theme={theme} />}            {activeTab === "about" && <AboutTab />}
          </motion.div>
        </AnimatePresence>
      </main>

      {/* 窗口内左侧悬浮提示（替代系统弹窗，自动消失） */}
      <FloatingNotice notice={notice} onDismiss={() => setNotice(null)} duration={7000} />

      {/* Toast 通知 */}
      {/*
        从默认的顶部右上角往下挪约 1.5 个提示框高度（96px）。
        原因：积分页的「一键领取积分」按钮就在右上角，账号连接成功的提示
        正好压在它上面，提示显示期间按钮点不到。留出顶部间距后两者不再重叠。
      */}
      <Toaster
        position="top-right"
        offset={96}
        richColors
        closeButton
        toastOptions={{
          duration: 3500,
          classNames: {
            toast: "!rounded-modal !border !border-white/30 !bg-white/70 !backdrop-blur-xl !shadow-2 dark:!border-white/10 dark:!bg-slate-950/70",
          },
        }}
      />

      <UpdateModal
        open={showStartupUpdateModal}
        onClose={() => setShowStartupUpdateModal(false)}
        version={startupUpdateInfo?.version || ""}
        body={startupUpdateInfo?.body || ""}
        onInstall={installStartupUpdate}
        installing={startupInstalling}
      />
    </div>
  );
}

// 设置页已拆到独立文件（布局紧凑化 + 悬浮窗/自启开关），这里只保留引用。

// 关于标签页
const AboutTab: React.FC = () => {
  const appVersion = useAppVersion();

  const features = [
    "MCP 服务器统一管理，支持一键启用/禁用",
    "Skills 技能同步到多个 AI 编程工具",
    "WorkBuddy / TRAE / Qoder / MiniMax 多账号积分池",
    "本地模型网关：所有账号汇聚成一个 OpenAI 兼容端点",
    "自动扫描并导入现有工具配置",
    "跨平台支持（macOS、Windows、Linux）",
    "本地 SQLite 数据库存储，开箱即用",
  ];

  return (
    <div className="glass-app flex h-full flex-col overflow-hidden">
      {/* 头部 */}
      <div className="glass-header">
        <div className="glass-kicker">
          <Info size={13} />
          About
        </div>
        <h2 className="mt-3 text-display">关于</h2>
        <p className="mt-2 text-caption text-slate-500 dark:text-slate-400">
          了解 Quoly AI 的更多信息
        </p>
      </div>

      {/* 内容 */}
      <div className="glass-content">
        <div className="max-w-2xl space-y-6">
          {/* 项目信息 */}
          <section className="glass-card p-6">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h3 className="text-base font-medium">Quoly AI</h3>
                <p className="mt-0.5 text-caption text-slate-500 dark:text-slate-400">
                  v{appVersion}· 多平台 AI 额度管理器与本地模型网关
                </p>
              </div>
              <Pressable
                onClick={() => open(GITHUB_REPO_URL)}
                className="glass-secondary-button min-h-8 px-3 py-1.5 text-xs"
              >
                <Github size={12} />
                开源仓库
                <ExternalLink size={10} />
              </Pressable>
            </div>
            <p className="text-sm leading-relaxed text-slate-500 dark:text-slate-400">
              一款基于 Tauri 2 构建的跨平台桌面应用。把散落在 WorkBuddy、TRAE、Qoder、MiniMax
              里的账号与额度集中到一处，自动领取每日积分，并通过本地网关把它们汇聚成
              一个 OpenAI 兼容端点，任何 AI 编程工具都能直接调用。同时管理 25 种工具的
              MCP 服务器与 Skills 技能。
            </p>
            <div className="mt-4 flex flex-wrap gap-2">
              <Pressable
                onClick={() => open(OFFICIAL_WEBSITE_URL)}
                className="glass-pill inline-flex items-center gap-1"
              >
                官网 luforge.com
                <ExternalLink size={10} />
              </Pressable>
              <Pressable
                onClick={() => open(OFFICIAL_WEBSITE_URL)}
                className="glass-pill inline-flex items-center gap-1"
              >
                更多工具与教程
                <ExternalLink size={10} />
              </Pressable>
            </div>
          </section>

          {/* 核心特性 */}
          <section className="glass-card p-6">
            <h3 className="text-base font-medium mb-4">核心特性</h3>
            <ul className="space-y-2.5">
              {features.map((feature) => (
                <li key={feature} className="flex items-start gap-2.5 text-sm">
                  <div className="mt-1.5 h-1.5 w-1.5 flex-shrink-0 rounded-full bg-blue-500" />
                  <span className="text-slate-700 dark:text-slate-200">
                    {feature}
                  </span>
                </li>
              ))}
            </ul>
          </section>

          {/* 技术栈 */}
          <section className="glass-card p-6">
            <h3 className="text-base font-medium mb-4">技术栈</h3>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <p className="text-xs font-medium text-[hsl(var(--muted-foreground))] uppercase tracking-wider mb-2">
                  前端
                </p>
                <div className="flex flex-wrap gap-2">
                  {["React", "TypeScript", "TailwindCSS", "TanStack Query"].map(
                    (tech) => (
                      <span
                        key={tech}
                        className="glass-pill"
                      >
                        {tech}
                      </span>
                    )
                  )}
                </div>
              </div>
              <div>
                <p className="text-xs font-medium text-[hsl(var(--muted-foreground))] uppercase tracking-wider mb-2">
                  后端
                </p>
                <div className="flex flex-wrap gap-2">
                  {["Tauri 2", "Rust", "SQLite"].map((tech) => (
                    <span
                      key={tech}
                    className="glass-pill"
                    >
                      {tech}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          </section>

          {/* 支持与反馈 */}
          <section className="glass-card p-6">
            <h3 className="text-base font-medium mb-3">支持与反馈</h3>
            <div className="space-y-3 text-sm text-slate-500 dark:text-slate-400">
              <p>
                遇到问题或有功能建议，欢迎在{" "}
                <Pressable
                  onClick={() => open(`${GITHUB_REPO_URL}/issues`)}
                  className="text-[hsl(var(--primary))] hover:underline inline-flex items-center gap-0.5 !bg-transparent !shadow-none !min-h-0 !p-0"
                >
                  GitHub Issues
                  <ExternalLink size={10} />
                </Pressable>{" "}
                提交，也可以直接到官网留言。
              </p>
              <div className="flex flex-wrap gap-2 pt-1">
                <Pressable
                  onClick={() => open(OFFICIAL_WEBSITE_URL)}
                  className="glass-secondary-button min-h-8 px-3 py-1.5 text-xs"
                >
                  访问官网
                  <ExternalLink size={10} />
                </Pressable>
                <Pressable
                  onClick={() => open(GITHUB_REPO_URL)}
                  className="glass-secondary-button min-h-8 px-3 py-1.5 text-xs"
                >
                  <Github size={12} />
                  给个 Star
                  <ExternalLink size={10} />
                </Pressable>
              </div>
            </div>
          </section>

          {/* 来源说明：MIT 许可证要求保留上游署名 */}
          <section className="glass-card p-6">
            <h3 className="text-base font-medium mb-3">开源说明</h3>
            <p className="text-sm leading-relaxed text-slate-500 dark:text-slate-400">
              Quoly AI 基于开源项目{" "}
              <Pressable
                onClick={() => open("https://github.com/whyfail/AI-Toolkit")}
                className="text-[hsl(var(--primary))] hover:underline inline-flex items-center gap-0.5 !bg-transparent !shadow-none !min-h-0 !p-0"
              >
                AI Toolkit
                <ExternalLink size={10} />
              </Pressable>
              （MIT，作者 whyfail）修改而来，并参考了 TraeWorkAssistant、cpa-multi-plugins
              等 MIT 项目的协议实现。上游版权声明保留在发行包内的 NOTICE 与 LICENSE 文件中。
            </p>
            <p className="mt-2 text-sm leading-relaxed text-slate-500 dark:text-slate-400">
              本项目同样以 MIT 许可证开源，欢迎在{" "}
              <Pressable
                onClick={() => open(GITHUB_REPO_URL)}
                className="text-[hsl(var(--primary))] hover:underline inline-flex items-center gap-0.5 !bg-transparent !shadow-none !min-h-0 !p-0"
              >
                GitHub
                <ExternalLink size={10} />
              </Pressable>{" "}
              查看源码或参与贡献。
            </p>
          </section>
        </div>
      </div>
    </div>
  );
};

export default App;
