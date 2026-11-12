import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-shell';
import { AnimatePresence, motion } from 'motion/react';
import { toast } from 'sonner';
import {
  ArrowUpCircle,
  Check,
  CheckCircle,
  Database,
  ExternalLink,
  FileWarning,
  FolderOpen,
  Github,
  Loader2,
  Loader,
  Power,
  Settings,
  Share2,
} from 'lucide-react';

import { AgentIcon } from '@/components/ui/AgentIcon';
import { Pressable } from '@/components/ui/Pressable';
import UpdateModal from '@/components/mcp/UpdateModal';
import { useInstalledTools } from '@/contexts/InstalledToolsContext';
import { useAppVersion } from '@/hooks/useAppVersion';
import type { ThemeMode, useTheme } from '@/hooks/useTheme';
import { appApi, updateApi } from '@/lib/api';
import type { SkillsInstallPreferences } from '@/lib/api';
import { shellApi } from '@/lib/api/shell';

/**
 * 设置页。
 *
 * 布局按 workbuddy-switch 的样子重排：分区标题在卡片**外面**（小而灰），
 * 卡片内是一行行「左标签+说明 / 右控件」，行间用极细的分隔线，上下留白收窄。
 *
 * 之前那版每块都是 `p-6` 的大卡片 + 大标题，一屏只能放两组设置，滚动条很长；
 * 现在同样的内容一屏能看四五组，扫读时也不用在每块之间重新定位。
 */

const GITHUB_REPO_URL = 'https://github.com/zenolore/quoly-ai';
const OFFICIAL_WEBSITE_URL = 'https://quoly-ai.luforge.com';

const copyText = async (text: string) => {
  try {
    await navigator.clipboard?.writeText(text);
    return;
  } catch {
    // 剪贴板权限被拒时退回 execCommand，桌面端这两种路径通常有一种能成
  }
  const area = document.createElement('textarea');
  area.value = text;
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.select();
  document.execCommand('copy');
  document.body.removeChild(area);
};

interface SettingsPanelProps {
  theme: ReturnType<typeof useTheme>;
}

export function SettingsPanel({ theme }: SettingsPanelProps) {
  const { installedAgents } = useInstalledTools();
  const appVersion = useAppVersion();
  const isWindows = navigator.userAgent.includes('Windows');
  const dbPath = isWindows ? '%USERPROFILE%\\.ai-toolkit\\ai-toolkit.db' : '~/.ai-toolkit/ai-toolkit.db';
  // 路径由主进程给（跨平台 + 不受打包环境影响），前端不猜 HOME

  const [checking, setChecking] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<{ version: string; body: string } | null>(null);
  const [isLatest, setIsLatest] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [showModal, setShowModal] = useState(false);
  const [copied, setCopied] = useState(false);

  const [skillsPrefs, setSkillsPrefs] = useState<SkillsInstallPreferences | null>(null);
  const [savingSkills, setSavingSkills] = useState(false);

  // 自启状态以主进程为准（用户可能在任务管理器里手动禁用过），
  // 前端只做乐观切换。
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [shellBusy, setShellBusy] = useState<'autostart' | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const prefs = await appApi.getSkillsInstallPreferences();
        if (!cancelled) setSkillsPrefs(prefs);
      } catch (err) {
        console.error('获取设置失败:', err);
      }
      try {
        const shell = await shellApi.settings();
        if (!cancelled) setAutostartEnabled(shell.autostartEnabled);
      } catch (err) {
        console.error('读取界面设置失败:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const selectedSkillsPath = skillsPrefs?.options.find((o) => o.id === skillsPrefs.selected)?.path;

  const toggleAutostart = async (next: boolean) => {
    setAutostartEnabled(next);
    setShellBusy('autostart');
    try {
      const s = await shellApi.setAutostart(next);
      setAutostartEnabled(s.autostartEnabled);
      toast.success(next ? '已设为开机静默启动到托盘' : '已取消开机启动');
    } catch (err) {
      setAutostartEnabled(!next);
      toast.error(`设置开机启动失败: ${err}`);
    } finally {
      setShellBusy(null);
    }
  };

  const changeSkillsLocation = async (id: string) => {
    if (!skillsPrefs) return;
    const previous = skillsPrefs.selected;
    setSkillsPrefs({ ...skillsPrefs, selected: id });
    setSavingSkills(true);
    try {
      await appApi.setSkillsInstallLocation(id);
      toast.success('Skills 安装位置已更新');
    } catch (err) {
      setSkillsPrefs({ ...skillsPrefs, selected: previous });
      toast.error(`保存失败: ${err}`);
    } finally {
      setSavingSkills(false);
    }
  };

  const checkUpdate = async () => {
    setChecking(true);
    setUpdateInfo(null);
    setIsLatest(false);
    try {
      const result = await updateApi.checkUpdate();
      if (result.available) {
        setUpdateInfo({ version: result.version, body: result.body || '' });
        setShowModal(true);
      } else {
        setIsLatest(true);
        setTimeout(() => setIsLatest(false), 3000);
      }
    } catch (err) {
      toast.error(`检查更新失败: ${err}`);
    } finally {
      setChecking(false);
    }
  };

  const installUpdate = async () => {
    setInstalling(true);
    try {
      await updateApi.installUpdate();
      toast.success('更新下载完成，正在重启应用...');
    } catch (err) {
      toast.error(`安装更新失败: ${err}`);
    } finally {
      setInstalling(false);
    }
  };

  const copyShareUrl = async () => {
    try {
      await copyText(OFFICIAL_WEBSITE_URL);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error('复制失败，请稍后重试');
    }
  };

  const themeOptions: Array<{ id: ThemeMode; label: string }> = [
    { id: 'system', label: '跟随系统' },
    { id: 'light', label: '浅色' },
    { id: 'dark', label: '深色' },
  ];

  return (
    <div className="glass-app flex h-full flex-col overflow-hidden">
      <div className="glass-header">
        <div className="glass-kicker">
          <Settings size={13} />
          PREFERENCES
        </div>
        <h2 className="mt-3 text-display">设置</h2>
        <p className="mt-1.5 text-sm text-slate-500 dark:text-slate-400">
          界面外观、开机启动、数据存储与错误日志
        </p>
      </div>

      <div className="glass-content">
        <div className="max-w-[680px] space-y-5 pb-4">
          {/* ── 外观 ───────────────────────────────── */}
          <Group title="外观">
            <Row label="主题" hint="跟随系统时会自动切换到系统当前的深浅色">
              <div className="relative inline-flex rounded-lg border border-white/60 bg-white/50 p-0.5 dark:border-white/10 dark:bg-white/5">
                {themeOptions.map((option) => {
                  const active = theme.mode === option.id;
                  return (
                    <Pressable
                      key={option.id}
                      type="button"
                      onClick={() => theme.setMode(option.id)}
                      className={`relative z-10 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                        active
                          ? 'text-white'
                          : 'text-slate-500 hover:text-slate-800 dark:text-slate-400 dark:hover:text-white'
                      }`}
                    >
                      {active && (
                        <motion.span
                          layoutId="settings-theme-pill"
                          className="absolute inset-0 -z-10 rounded-md bg-gradient-to-r from-[#0A84FF] to-[#5AC8FA] shadow-[0_2px_8px_rgba(10,132,255,0.28)]"
                          transition={{ type: 'spring', bounce: 0.15, duration: 0.35 }}
                        />
                      )}
                      {option.label}
                    </Pressable>
                  );
                })}
              </div>
            </Row>
          </Group>

          {/* ── 启动 ───────────────────────────────── */}
          <Group title="启动">
            {/*
              桌面悬浮窗入口暂时收起：窗口本身能建出来，但 WebView2 下
              「加载同一份前端产物」这条路走不通（窗口里是错误页），
              留个开关只会让人以为功能可用。等悬窗单独做一页再放回来。
              相关代码保留在 float_window.rs / FloatWidget.tsx，未删。
            */}
            <Row
              label="开机时静默启动到托盘"
              hint="开机不弹主窗口，只留托盘图标；从托盘可随时打开主界面"
              icon={<Power size={13} />}
            >
              <Switch
                checked={autostartEnabled}
                busy={shellBusy === 'autostart'}
                onChange={(next) => void toggleAutostart(next)}
              />
            </Row>
          </Group>

          {/* ── 软件更新 ───────────────────────────── */}
          <Group title="软件更新">
            <Row label={`当前版本 v${appVersion}`} hint="更新源：GitHub Releases" icon={<ArrowUpCircle size={13} />}>
              <div className="flex items-center gap-2">
                {isLatest && (
                  <span className="flex items-center gap-1 text-[11px] text-emerald-600 dark:text-emerald-400">
                    <CheckCircle size={11} />
                    已是最新
                  </span>
                )}
                <Pressable
                  onClick={checkUpdate}
                  disabled={checking}
                  className="flex items-center gap-1.5 rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white disabled:opacity-60 dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
                >
                  {checking ? <Loader2 size={11} className="animate-spin" /> : <ArrowUpCircle size={11} />}
                  {checking ? '检查中' : '检查更新'}
                </Pressable>
              </div>
            </Row>
            <Row label="项目主页" hint="源码、更新日志与问题反馈" icon={<Github size={13} />}>
              <Pressable
                onClick={() => void open(GITHUB_REPO_URL)}
                className="flex items-center gap-1 rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
              >
                GitHub
                <ExternalLink size={10} />
              </Pressable>
            </Row>
            <Row label="更多工具" hint="官网提供其他效率工具与使用教程" icon={<Share2 size={13} />}>
              <Pressable
                onClick={() => void open(OFFICIAL_WEBSITE_URL)}
                className="flex items-center gap-1 rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
              >
                quoly-ai.luforge.com
                <ExternalLink size={10} />
              </Pressable>
            </Row>
            <Row
              label="开源致谢"
              hint="基于 whyfail/AI-Toolkit（MIT）修改，上游署名见发行包 NOTICE"
              icon={<Github size={13} />}
            >
              <Pressable
                onClick={() => void open('https://github.com/whyfail/AI-Toolkit')}
                className="flex items-center gap-1 rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
              >
                上游项目
                <ExternalLink size={10} />
              </Pressable>
            </Row>
          </Group>

          {/* ── 数据与存储 ─────────────────────────── */}
          <Group title="数据与存储">
            <Row label="数据库" hint={dbPath} icon={<Database size={13} />}>
              <Pressable
                onClick={async () => {
                  try {
                    await copyText(dbPath);
                    toast.success('路径已复制');
                  } catch {
                    toast.error('复制失败');
                  }
                }}
                className="rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
              >
                复制路径
              </Pressable>
            </Row>
            <Row
              label="Skills 安装位置"
              hint={selectedSkillsPath || '加载中…'}
              icon={<FolderOpen size={13} />}
            >
              <div
                className={`relative inline-flex rounded-lg border border-white/60 bg-white/50 p-0.5 dark:border-white/10 dark:bg-white/5 ${
                  savingSkills ? 'pointer-events-none opacity-60' : ''
                }`}
              >
                {skillsPrefs?.options.map((option) => {
                  const active = skillsPrefs.selected === option.id;
                  return (
                    <Pressable
                      key={option.id}
                      type="button"
                      onClick={() => void changeSkillsLocation(option.id)}
                      className={`relative z-10 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                        active
                          ? 'text-white'
                          : 'text-slate-500 hover:text-slate-800 dark:text-slate-400 dark:hover:text-white'
                      }`}
                    >
                      {active && (
                        <motion.span
                          layoutId="skills-location-pill"
                          className="absolute inset-0 -z-10 rounded-md bg-gradient-to-r from-[#0A84FF] to-[#5AC8FA] shadow-[0_2px_8px_rgba(10,132,255,0.28)]"
                          transition={{ type: 'spring', bounce: 0.15, duration: 0.35 }}
                        />
                      )}
                      {option.label}
                    </Pressable>
                  );
                })}
                {!skillsPrefs && <span className="px-2.5 py-1 text-xs text-slate-400">加载中…</span>}
              </div>
            </Row>
          </Group>

          {/* ── 分享 ───────────────────────────────── */}
          <Group title="分享">
            <Row label="官网地址" hint={OFFICIAL_WEBSITE_URL} icon={<Share2 size={13} />}>
              <Pressable
                onClick={copyShareUrl}
                className="flex items-center gap-1.5 rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
              >
                <AnimatePresence mode="wait" initial={false}>
                  {copied ? (
                    <motion.span
                      key="copied"
                      className="flex items-center gap-1.5"
                      initial={{ opacity: 0, y: 3 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -3 }}
                      transition={{ duration: 0.15 }}
                    >
                      <Check size={11} />
                      已复制
                    </motion.span>
                  ) : (
                    <motion.span
                      key="copy"
                      className="flex items-center gap-1.5"
                      initial={{ opacity: 0, y: 3 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: -3 }}
                      transition={{ duration: 0.15 }}
                    >
                      <Share2 size={11} />
                      复制
                    </motion.span>
                  )}
                </AnimatePresence>
              </Pressable>
            </Row>
          </Group>

          {/* ── 错误日志 ───────────────────────────── */}
          <Group title="错误日志">
            <Row
              label="崩溃与未捕获错误"
              hint="记录在 ~/.ai-toolkit/crash-trail.log，反馈问题时直接附上"
              icon={<FileWarning size={13} />}
            >
              <Pressable
                onClick={async () => {
                  try {
                    await shellApi.openLogFolder();
                  } catch (err) {
                    toast.error(`打开失败: ${err}`);
                  }
                }}
                className="flex items-center gap-1 rounded-lg border border-slate-200/80 bg-white/70 px-2.5 py-1 text-[11px] font-medium text-slate-600 transition hover:bg-white dark:border-slate-700 dark:bg-slate-800/60 dark:text-slate-300"
              >
                <FolderOpen size={10} />
                打开日志位置
              </Pressable>
            </Row>
          </Group>

          {/* ── 已检测到的本地 Agent ───────────────── */}
          <Group title="本地已安装的 Agent">
            <div className="flex flex-wrap gap-2 px-3.5 py-3">
              {installedAgents.length === 0 && (
                <p className="text-xs text-slate-400 dark:text-slate-500">未检测到已安装的 Agent</p>
              )}
              {installedAgents.map((agent) => (
                <div
                  key={agent.id}
                  title={agent.name}
                  className="flex h-9 w-9 items-center justify-center rounded-lg border border-white/55 bg-white/45 p-1.5 transition-colors hover:bg-white/75 dark:border-white/10 dark:bg-white/5 dark:hover:bg-white/10"
                >
                  <AgentIcon id={agent.id} name={agent.name} className="h-5 w-5" />
                </div>
              ))}
            </div>
          </Group>
        </div>
      </div>

      <UpdateModal
        open={showModal}
        onClose={() => setShowModal(false)}
        version={updateInfo?.version || ''}
        body={updateInfo?.body || ''}
        onInstall={installUpdate}
        installing={installing}
      />
    </div>
  );
}

/** 分区：小标题在卡片外面，和 workbuddy-switch 一致。 */
function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <h3 className="mb-2 px-0.5 text-[11px] font-semibold tracking-wide text-slate-500 dark:text-slate-400">
        {title}
      </h3>
      <div className="glass-card divide-y divide-slate-200/60 !rounded-xl dark:divide-slate-700/50">
        {children}
      </div>
    </section>
  );
}

/** 一行设置：左边标签+说明，右边控件。 */
function Row({
  label,
  hint,
  icon,
  children,
}: {
  label: string;
  hint?: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 px-3.5 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-[12.5px] font-medium text-slate-700 dark:text-slate-200">
          {icon ? <span className="text-slate-400 dark:text-slate-500">{icon}</span> : null}
          <span className="truncate">{label}</span>
        </div>
        {hint ? (
          <div className="mt-0.5 truncate text-[10.5px] text-slate-400 dark:text-slate-500" title={hint}>
            {hint}
          </div>
        ) : null}
      </div>
      <div className="flex flex-shrink-0 items-center">{children}</div>
    </div>
  );
}

/** 开关。自己画而不是用原生 checkbox：需要和玻璃质感统一，也要 busy 态。 */
function Switch({
  checked,
  busy,
  onChange,
}: {
  checked: boolean;
  busy?: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={busy}
      onClick={() => onChange(!checked)}
      className={`relative h-[22px] w-[38px] flex-shrink-0 rounded-full transition-colors duration-200 ${
        checked ? 'bg-emerald-500' : 'bg-slate-300 dark:bg-slate-600'
      } ${busy ? 'opacity-60' : ''}`}
    >
      <span
        className={`absolute top-[2px] flex h-[18px] w-[18px] items-center justify-center rounded-full bg-white shadow-sm transition-all duration-200 ${
          checked ? 'left-[18px]' : 'left-[2px]'
        }`}
      >
        {busy ? <Loader size={9} className="animate-spin text-slate-400" /> : null}
      </span>
    </button>
  );
}

