#!/bin/bash
# 从私有发布 tag 导出临时工作区,移除私人实现后在本机构建 Mac Apple Silicon 免费版。

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REF="${1:-}"

if [[ ! "$REF" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "用法:bash scripts/build-free-macos.sh v<x.y.z>" >&2
  exit 2
fi
if [ "$(uname -m)" != "arm64" ]; then
  echo "免费 Mac 包当前只构建 Apple Silicon(arm64)" >&2
  exit 2
fi

git -C "$ROOT_DIR" rev-parse --verify "$REF^{commit}" >/dev/null
VERSION="${REF#v}"
TAG_VERSION="$(git -C "$ROOT_DIR" show "$REF:package.json" | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).version))')"
if [ "$TAG_VERSION" != "$VERSION" ]; then
  echo "tag $REF 与 package.json 版本 $TAG_VERSION 不一致" >&2
  exit 1
fi
if [ ! -d "$ROOT_DIR/node_modules" ]; then
  echo "缺少 node_modules,请先在私有主仓运行 pnpm install" >&2
  exit 1
fi

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/caseboard-free-macos.XXXXXX")"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

git -C "$ROOT_DIR" archive "$REF" | tar -x -C "$STAGE"
node "$ROOT_DIR/scripts/prepare-public-edition.mjs" "$STAGE"
ln -s "$ROOT_DIR/node_modules" "$STAGE/node_modules"
if [ ! -d "$ROOT_DIR/sidecars/pi-runtime/node_modules" ]; then
  echo "缺少 Pi Runtime node_modules,请先在私有主仓执行 bun install --cwd sidecars/pi-runtime --frozen-lockfile --ignore-scripts" >&2
  exit 1
fi
ln -s "$ROOT_DIR/sidecars/pi-runtime/node_modules" "$STAGE/sidecars/pi-runtime/node_modules"

if [ -f "$ROOT_DIR/telemetry/.env.telemetry" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ROOT_DIR/telemetry/.env.telemetry"
  set +a
fi

export CASEBOARD_BUILD_EDITION=free
export CASEBOARD_NO_REVEAL=1
export CARGO_TARGET_DIR="$ROOT_DIR/target/free-build/aarch64"

rm -f \
  "$CARGO_TARGET_DIR/release/bundle/dmg/案件看板_${VERSION}_aarch64.dmg" \
  "$CARGO_TARGET_DIR/release/bundle/macos/案件看板.app.tar.gz" \
  "$CARGO_TARGET_DIR/release/bundle/macos/案件看板.app.tar.gz.sig"

bash "$STAGE/scripts/release.sh"

SOURCE_DMG="$CARGO_TARGET_DIR/release/bundle/dmg/案件看板_${VERSION}_aarch64.dmg"
SOURCE_UPDATER="$CARGO_TARGET_DIR/release/bundle/macos/案件看板.app.tar.gz"
SOURCE_SIG="$SOURCE_UPDATER.sig"
OUT_DIR="$ROOT_DIR/target/private-release/$REF/free-macos"

for file in "$SOURCE_DMG" "$SOURCE_UPDATER" "$SOURCE_SIG"; do
  if [ ! -s "$file" ]; then
    echo "免费 Mac 构建缺少产物:$file" >&2
    exit 1
  fi
done

mkdir -p "$OUT_DIR"
cp "$SOURCE_DMG" "$SOURCE_UPDATER" "$SOURCE_SIG" "$OUT_DIR/"
shasum -a 256 "$OUT_DIR"/*
echo "免费 Mac Apple Silicon 产物已收集到:$OUT_DIR"
