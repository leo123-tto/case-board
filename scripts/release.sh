#!/bin/bash
# CaseBoard public macOS release builder · 2026-07-22
#
# 统一产出 Apple Silicon / Intel macOS 的 dmg + updater 包。
#
# 用法:
#   bash scripts/release.sh aarch64
#   bash scripts/release.sh x86_64
#
# 前置:
#   - 已在 ~/.cargo/bin 装好 cargo / 已在 PATH
#   - 已 pnpm install(node_modules 完整)
#
# 产出:
#   target[/<rust-target>]/release/bundle/dmg/案件看板_<version>_<arch>.dmg

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

# 读 package.json 的 version，并把两种架构统一映射到真实产物路径。
VERSION=$(node -p "require('./package.json').version")
REQUESTED_ARCH="${1:-aarch64}"
case "$REQUESTED_ARCH" in
  aarch64|arm64)
    ARCH="aarch64"
    FILE_ARCH="aarch64"
    CASEBOARD_PI_RUNTIME_TARGET="${CASEBOARD_PI_RUNTIME_TARGET:-bun-darwin-arm64}"
    BUNDLE_ROOT="target/release/bundle"
    ;;
  x86_64|x64|intel)
    ARCH="x86_64"
    FILE_ARCH="x64"
    CASEBOARD_PI_RUNTIME_TARGET="${CASEBOARD_PI_RUNTIME_TARGET:-bun-darwin-x64-baseline}"
    BUNDLE_ROOT="target/x86_64-apple-darwin/release/bundle"
    ;;
  *)
    echo "用法: bash scripts/release.sh [aarch64|x86_64]" >&2
    exit 2
    ;;
esac
export CASEBOARD_PI_RUNTIME_TARGET

echo "════════════════════════════════════════════════════════"
echo "  CaseBoard release · v${VERSION} · ${ARCH}"
echo "════════════════════════════════════════════════════════"
echo

# Tauri 会通过 beforeBuildCommand 自动跑一次 pnpm build；不要重复构建前端。
echo "▶ Step 1/2: Tauri 构建 (app + dmg)"
echo "    (首次约 5-10 分钟,后续 1-2 分钟。期间不会弹窗,可放心等)"

# 公开包必须由父 shell 注入更新签名和匿名遥测；缺一项立即失败，避免发出不可更新/无统计包。
: "${TAURI_SIGNING_PRIVATE_KEY:?缺少 TAURI_SIGNING_PRIVATE_KEY}"
: "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:?缺少 TAURI_SIGNING_PRIVATE_KEY_PASSWORD}"
: "${CASEBOARD_TELEMETRY_URL:?缺少 CASEBOARD_TELEMETRY_URL}"
: "${CASEBOARD_TELEMETRY_KEY:?缺少 CASEBOARD_TELEMETRY_KEY}"
echo "    ✓ 更新签名与匿名遥测环境已就绪"

if [ "$ARCH" = "x86_64" ]; then
  pnpm tauri build --target x86_64-apple-darwin --bundles app,dmg
else
  pnpm tauri build --bundles app,dmg
fi

# 找产出
# 注意:本项目是 cargo workspace,target 目录在仓库根而不是 src-tauri/target
DMG_PATH="$BUNDLE_ROOT/dmg/案件看板_${VERSION}_${FILE_ARCH}.dmg"
APP_PATH="$BUNDLE_ROOT/macos/案件看板.app"
UPDATER_PATH="$BUNDLE_ROOT/macos/案件看板.app.tar.gz"

echo
echo "▶ 验证安装包内置 Pi Runtime"
pnpm verify:pi-runtime-bundle \
  --bundle-dir "$APP_PATH/Contents/Resources/pi-runtime" \
  --updater "$UPDATER_PATH"

# 3. 后处理:往 dmg 里塞「请先阅读.txt」+ AppleScript 设置窗口布局
# 原因:macOS 15.1+ 苹果封死「右键 → 打开」绕过 ad-hoc 签名的路径,
# 用户必须走「系统设置 → 隐私与安全 → 仍要打开」。
# 早期试过 .command 脚本调 xattr,但 quarantine 后 Terminal 行为不稳定,
# 改用纯文本指引 + AppleScript 把指引放在 dmg 窗口顶部最显眼位置。
if [ -f "$DMG_PATH" ]; then
  echo
  echo "▶ Step 2/2: 嵌入「请先阅读.txt」+ 设置 dmg 窗口布局"
  WRITABLE_DMG="$BUNDLE_ROOT/dmg/_writable.dmg"
  VOLNAME="案件看板"
  README="scripts/请先阅读.txt"

  hdiutil detach "/Volumes/$VOLNAME" 2>/dev/null || true
  rm -f "$WRITABLE_DMG"

  hdiutil convert "$DMG_PATH" -format UDRW -o "$WRITABLE_DMG" -ov -quiet
  hdiutil attach "$WRITABLE_DMG" -quiet
  sleep 2

  VOL="/Volumes/$VOLNAME"
  rm -f "$VOL/.DS_Store"
  cp "$README" "$VOL/请先阅读.txt"
  # 2026-05-25 V0.1.10 删:之前塞过 安装助手.command,但 macOS 15.1+ 也会拦 .command(同 quarantine),
  # 用户照样打不开。改成「请先阅读.txt」主推一行终端命令,更稳。

  if [ "${CI:-false}" = "true" ] || [ "${CASEBOARD_SKIP_DMG_FINDER:-0}" = "1" ]; then
    echo "  · 无界面构建：跳过 Finder 窗口布局"
  else
    osascript <<APPLESCRIPT
tell application "Finder"
    tell disk "$VOLNAME"
        open
        delay 1
        set current view of container window to icon view
        set toolbar visible of container window to false
        set statusbar visible of container window to false
        set the bounds of container window to {400, 120, 1120, 600}
        set viewOptions to the icon view options of container window
        set arrangement of viewOptions to not arranged
        set icon size of viewOptions to 96
        set text size of viewOptions to 14
        set label position of viewOptions to bottom
        set position of item "请先阅读.txt" of container window to {360, 100}
        set position of item "案件看板.app" of container window to {175, 280}
        set position of item "Applications" of container window to {545, 280}
        update without registering applications
        delay 2
        close
    end tell
end tell
APPLESCRIPT
  fi

  sleep 2
  sync
  hdiutil detach "$VOL" -quiet
  rm "$DMG_PATH"
  hdiutil convert "$WRITABLE_DMG" -format UDZO -imagekey zlib-level=9 -o "$DMG_PATH" -quiet
  rm "$WRITABLE_DMG"
  echo "  ✓ 请先阅读.txt + 窗口布局已嵌入"
fi

if [ ! -f "$UPDATER_PATH" ] || [ ! -s "$UPDATER_PATH.sig" ]; then
  echo "  ❌ updater 包或签名缺失: $UPDATER_PATH(.sig)" >&2
  exit 1
fi
TELEMETRY_HOST="${CASEBOARD_TELEMETRY_URL#*://}"
TELEMETRY_HOST="${TELEMETRY_HOST%%/*}"
if [ -z "$TELEMETRY_HOST" ] ||
   ! grep -aFq "$TELEMETRY_HOST" "$APP_PATH/Contents/MacOS/caseboard"; then
  echo "  ❌ 公开包未检测到匿名遥测 endpoint" >&2
  exit 1
fi

echo
echo "════════════════════════════════════════════════════════"
if [ -f "$DMG_PATH" ]; then
  SIZE=$(du -sh "$DMG_PATH" | cut -f1)
  echo "  ✅ DMG 产出成功"
  echo "  位置: $DMG_PATH"
  echo "  更新包: $UPDATER_PATH"
  echo "  大小: $SIZE"
  echo
  echo "  下一步:交给 release manifest 统一上传 GitHub Release 与 lawtools.top"
  echo "  提示:未签名 dmg 在 macOS 15.1+ 需用户跑 xattr -cr,见 scripts/请先阅读.txt"
  [ "${CASEBOARD_NO_REVEAL:-0}" = "1" ] || open -R "$DMG_PATH"
else
  echo "  ❌ DMG 未找到,可能在别的路径(检查 build 日志)"
  echo "  期望位置: $DMG_PATH"
  ls -la "$BUNDLE_ROOT/dmg/" 2>/dev/null || echo "  bundle/dmg 目录不存在"
  exit 1
fi
echo "════════════════════════════════════════════════════════"
