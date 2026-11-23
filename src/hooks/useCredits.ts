import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { creditsApi } from "@/lib/api/credits";

export const creditsQueryKey = ["credits", "dashboard"] as const;
export const floatStatsQueryKey = ["credits", "floatStats"] as const;
export const usageDailyQueryKey = (days: number) =>
  ["credits", "usageDaily", days] as const;

/**
 * 悬浮窗专用：只取 4 个数字。
 *
 * 不要改回 `useCreditsDashboard` —— 那个接口每次返 2.42MB（158 个账号的
 * 完整快照），而悬浮窗只用得到 4 个数字。更麻烦的是它是同步命令，
 * 70ms 占着 Tauri 主线程，期间悬浮窗拖不动（第一次 pointerdown 被堵住）。
 *
 * 这里刻意 **不订阅** `credits-updated` 事件：那个事件在账号刷新时高频触发
 * （实测间隔中位数 211ms），悬浮窗跟着刷毫无意义 —— 它显示的是日累计数字，
 * 秒级变化肉眼看不出来。用 3 秒轮询足够，且不会和主界面抢主线程。
 */
export function useFloatStats() {
  return useQuery({
    queryKey: floatStatsQueryKey,
    queryFn: creditsApi.floatStats,
    staleTime: 2_000,
    refetchInterval: 3_000,
    refetchOnWindowFocus: false,
  });
}

export function useCreditsRuntime(panelActive: boolean) {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    let pending: ReturnType<typeof setTimeout> | undefined;
    const invalidate = () => {
      if (pending) return;
      pending = setTimeout(() => {
        pending = undefined;
        void queryClient.invalidateQueries({ queryKey: creditsQueryKey });
      }, 100);
    };
    const subscription = listen("credits-updated", invalidate);
    // Also read once after subscribing, covering updates during initialization.
    void subscription.then((unlisten) => {
      if (disposed) unlisten();
      else { stopListening = unlisten; invalidate(); }
    }).catch(() => {});
    return () => { disposed = true; stopListening?.(); if (pending) clearTimeout(pending); };
  }, [queryClient]);

  useEffect(() => {
    const update = () => {
      void creditsApi.setViewActive(panelActive && document.visibilityState === "visible" && document.hasFocus()).catch(() => {});
    };
    const online = () => { void creditsApi.networkRestored().catch(() => {}); };
    update();
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    window.addEventListener("online", online);
    document.addEventListener("visibilitychange", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      window.removeEventListener("online", online);
      document.removeEventListener("visibilitychange", update);
      void creditsApi.setViewActive(false).catch(() => {});
    };
  }, [panelActive]);
}

export function useCreditsDashboard() {
  return useQuery({
    queryKey: creditsQueryKey,
    queryFn: creditsApi.dashboard,
    staleTime: 10_000,
    refetchInterval: 30_000,
    refetchOnWindowFocus: true,
  });
}

/**
 * Per-day token / request series for the overview trend chart.
 *
 * `days` is part of the key so switching range refetches, but the series is
 * cheap enough to keep the previous window on screen while the next loads —
 * otherwise every range click blanks the chart.
 */
export function useUsageDaily(days: number) {
  return useQuery({
    queryKey: usageDailyQueryKey(days),
    queryFn: () => creditsApi.usageDaily(days),
    staleTime: 60_000,
    placeholderData: (previous) => previous,
  });
}

export function useCreditsAction<TInput, TOutput>(action: (input: TInput) => Promise<TOutput>) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: action,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: creditsQueryKey }),
    onError: (error) => toast.error(error instanceof Error ? error.message : String(error)),
  });
}
