import { useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import { CheckCircle2, Info, X, AlertTriangle } from 'lucide-react';
import { useReducedMotion } from '@/hooks/useReducedMotion';

export type ToastTone = 'info' | 'success' | 'warn';

export interface FloatingNoticeData {
  id: number;
  title: string;
  lines: string[];
  tone?: ToastTone;
}

interface Props {
  notice: FloatingNoticeData | null;
  onDismiss: () => void;
  /** 自动消失毫秒数，默认 6000 */
  duration?: number;
}

const TONE_STYLES: Record<ToastTone, { ring: string; icon: string; bar: string }> = {
  info: {
    ring: 'border-blue-200/80 dark:border-blue-800/60',
    icon: 'text-blue-500 dark:text-blue-400',
    bar: 'bg-blue-500',
  },
  success: {
    ring: 'border-emerald-200/80 dark:border-emerald-800/60',
    icon: 'text-emerald-500 dark:text-emerald-400',
    bar: 'bg-emerald-500',
  },
  warn: {
    ring: 'border-amber-200/80 dark:border-amber-800/60',
    icon: 'text-amber-500 dark:text-amber-400',
    bar: 'bg-amber-500',
  },
};

function ToneIcon({ tone }: { tone: ToastTone }) {
  if (tone === 'success') return <CheckCircle2 size={15} />;
  if (tone === 'warn') return <AlertTriangle size={15} />;
  return <Info size={15} />;
}

/**
 * 窗口内左侧悬浮提示：替代系统 MessageBox，短暂显示后自动消失。
 * 不阻塞操作、不需要点"确定"。
 */
export default function FloatingNotice({ notice, onDismiss, duration = 6000 }: Props) {
  const reduced = useReducedMotion();
  const [progress, setProgress] = useState(100);

  useEffect(() => {
    if (!notice) return;
    setProgress(100);
    const started = Date.now();
    const tick = window.setInterval(() => {
      const elapsed = Date.now() - started;
      const left = Math.max(0, 100 - (elapsed / duration) * 100);
      setProgress(left);
      if (left <= 0) window.clearInterval(tick);
    }, 60);
    const timer = window.setTimeout(onDismiss, duration);
    return () => {
      window.clearInterval(tick);
      window.clearTimeout(timer);
    };
  }, [notice, duration, onDismiss]);

  const tone = notice?.tone ?? 'info';
  const styles = TONE_STYLES[tone];

  return (
    <AnimatePresence>
      {notice && (
        <motion.div
          key={notice.id}
          initial={reduced ? { opacity: 0 } : { opacity: 0, x: -24 }}
          animate={{ opacity: 1, x: 0 }}
          exit={reduced ? { opacity: 0 } : { opacity: 0, x: -16 }}
          transition={{ type: 'spring', bounce: 0, duration: reduced ? 0.15 : 0.32 }}
          className={`fixed left-3 top-3 z-[60] w-[290px] overflow-hidden rounded-xl border bg-white/95 shadow-lg backdrop-blur-xl dark:bg-slate-900/95 ${styles.ring}`}
          role="status"
          aria-live="polite"
        >
          {/* 顶部进度条 */}
          <div className="h-0.5 w-full bg-slate-100 dark:bg-slate-800">
            <div
              className={`h-full ${styles.bar} transition-[width] duration-100 ease-linear`}
              style={{ width: `${progress}%` }}
            />
          </div>

          <div className="flex items-start gap-2 px-3 py-2.5">
            <span className={`mt-0.5 flex-shrink-0 ${styles.icon}`}>
              <ToneIcon tone={tone} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[12.5px] font-semibold text-slate-700 dark:text-slate-100">
                {notice.title}
              </div>
              {notice.lines.length > 0 && (
                <div className="mt-1 space-y-0.5">
                  {notice.lines.map((line, index) => (
                    <div
                      key={index}
                      className="break-all font-mono text-[10.5px] leading-relaxed text-slate-500 dark:text-slate-400"
                    >
                      {line}
                    </div>
                  ))}
                </div>
              )}
            </div>
            <button
              onClick={onDismiss}
              className="-mr-1 -mt-0.5 flex-shrink-0 rounded-md p-1 text-slate-400 transition hover:bg-slate-100 hover:text-slate-600 dark:hover:bg-slate-800 dark:hover:text-slate-200"
              aria-label="关闭提示"
            >
              <X size={13} />
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
