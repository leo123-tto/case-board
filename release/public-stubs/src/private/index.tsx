// 免费版私人功能接缝。
// 该文件只会被复制到临时构建工作区，不会覆盖私有主仓实现。
import type { ComponentType, ReactNode } from "react";

export interface PrivateTopTab {
  id: string;
  label: string;
  icon: ComponentType<{ className?: string }>;
  render: () => ReactNode;
}

export function getPrivateTopTabs(): PrivateTopTab[] {
  return [];
}
