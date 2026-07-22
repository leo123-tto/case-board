#!/bin/bash
# Windows EXE 在 GitHub 只做编译;更新签名始终在本机完成,私钥不上传。

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
INSTALLER="${1:-}"
SIGN_KEY="$HOME/.tauri/caseboard.key"
SIGN_PW="$HOME/.tauri/caseboard.key.pw"

if [ ! -s "$INSTALLER" ] || [[ "$INSTALLER" != *.exe ]]; then
  echo "用法:bash scripts/sign-windows-installer.sh <安装包.exe>" >&2
  exit 2
fi
for path in "$SIGN_KEY" "$SIGN_PW"; do
  [ -s "$path" ] || { echo "缺少本机签名文件:$path" >&2; exit 1; }
done

rm -f "$INSTALLER.sig"
TAURI_SIGNING_PRIVATE_KEY_PATH="$SIGN_KEY" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$SIGN_PW")" \
  pnpm --dir "$ROOT_DIR" tauri signer sign "$INSTALLER"

[ -s "$INSTALLER.sig" ] || { echo "Windows updater 签名生成失败" >&2; exit 1; }
echo "Windows updater 签名已在本机生成:$INSTALLER.sig"
