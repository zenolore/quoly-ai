if (import.meta.env.DEV) {
  import("react-grab");
}

import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import "./index.css";
import { InstalledToolsProvider } from "./contexts/InstalledToolsContext";
import { DesktopRuntimeGuard } from "./components/DesktopRuntimeGuard";
import { FloatWidget } from "./components/float/FloatWidget";
import { getCurrentWindow } from "@tauri-apps/api/window";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 分钟
      refetchOnWindowFocus: false,
    },
  },
});

/**
 * 判断当前是主界面还是悬浮窗。
 *
 * 用窗口标签而不是 URL hash：`WebviewUrl::App("index.html#/float")` 里的 `#`
 * 会被 PathBuf 编码成路径字符，窗口里直接变成「无法访问此页面」。窗口标签是
 * 创建时写死的 `float`，拿它判断最稳，也不需要给 vite 加多入口。
 *
 * 加 try/catch 是为了浏览器里跑 dev server 时也能工作（那里没有 Tauri 内部对象）。
 */
function isFloatWindow(): boolean {
  try {
    return getCurrentWindow().label === "float";
  } catch {
    return false;
  }
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isFloatWindow() ? (
      // 悬浮窗不要 StrictMode 的双挂载动画抖动，也不要整套 Provider：
      // 它只读一个 dashboard 查询，少挂东西启动更快。
      <QueryClientProvider client={queryClient}>
        <FloatWidget />
      </QueryClientProvider>
    ) : (
      <DesktopRuntimeGuard>
        <QueryClientProvider client={queryClient}>
          <InstalledToolsProvider>
            <App />
          </InstalledToolsProvider>
        </QueryClientProvider>
      </DesktopRuntimeGuard>
    )}
  </React.StrictMode>
);
