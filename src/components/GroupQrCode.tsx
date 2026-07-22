/**
 * 微信交流群二维码(已裁成方形,撑满不变形)。
 *
 * 微信群二维码 7 天失效、无永久码 —— 所以**托管在 lawtools.top**,过期换图不用重新发版。
 * 更新必须按 `docs/群二维码更新SOP.md` 执行:同步私有主仓兜底图与私有分发仓,
 * 等 OSS workflow 成功后再以线上哈希 + 可解码性验收。
 * `?d=今天` 每日 cache-bust(绕 Cloudflare 缓存)。
 * 远程加载失败(离线 / 未上线)→ 回退打包的本地副本 `public/group-qr.jpg`。
 *
 * size 给定 → 固定方形;不给 → 用 className 控制(如 w-full 撑满容器)。
 */
import { useState } from "react";

import { todayIsoLocal } from "@/lib/date";
import { cn } from "@/lib/utils";

const REMOTE = "https://lawtools.top/caseboard/group-qr.jpg";
const LOCAL = "/group-qr.jpg";

export function GroupQrCode({
  size,
  className,
}: {
  size?: number;
  className?: string;
}) {
  const today = todayIsoLocal();
  const [src, setSrc] = useState(`${REMOTE}?d=${today}`);
  const [usedLocal, setUsedLocal] = useState(false);
  // 本地兜底也带 cache-bust:换图后(同名文件)绕开浏览器旧缓存,避免显示裁剪前的旧图
  return (
    <img
      src={src}
      alt="案件看板交流群二维码"
      // max-w-none:抵消 Tailwind preflight 的 img{max-width:100%}(会把 size 指定的宽度
      // 限到窄父容器宽度、高度仍按 size → 把二维码拉成竖条变形)
      className={cn("max-w-none bg-white", className)}
      style={size != null ? { width: size, height: size } : undefined}
      onError={() => {
        if (!usedLocal) {
          setUsedLocal(true);
          setSrc(`${LOCAL}?d=${today}`);
        }
      }}
    />
  );
}
