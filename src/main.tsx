import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";
import { installConsoleTap } from "@/lib/console-tap";
import { applyFontScale } from "@/lib/uiScale";
import { applyPlatformIdentity } from "@/lib/platform";

// 2026-05-26 V0.1.11:在 React 启动前装 console.error/warn + window.onerror tap,
// 反馈弹窗打开时一次性把累积的报错回传给 Rust 端写进 MD。
installConsoleTap();

// 平台标记必须早于首屏渲染,避免 Windows 字体 / 滚动条样式后切换造成抖动。
applyPlatformIdentity();

// 2026-06-16:React 启动前应用界面字号缩放,避免默认 16px 先渲染再跳变(闪烁)。
applyFontScale();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
