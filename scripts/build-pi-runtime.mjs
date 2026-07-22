#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sidecarRoot = join(repositoryRoot, "sidecars", "pi-runtime");
const runtimeRelease = JSON.parse(
  readFileSync(join(sidecarRoot, "runtime-release.json"), "utf8"),
);
const targetArgIndex = process.argv.indexOf("--target");
const requestedTarget = targetArgIndex >= 0 ? process.argv[targetArgIndex + 1] : undefined;
if (targetArgIndex >= 0 && !requestedTarget) {
  throw new Error("--target 后必须提供 Bun target，例如 bun-darwin-arm64");
}

const defaultTarget = process.platform === "darwin"
  ? (process.arch === "arm64" ? "bun-darwin-arm64" : "bun-darwin-x64")
  : process.platform === "win32" && process.arch === "x64"
    ? "bun-windows-x64-baseline"
    : undefined;
const target = requestedTarget ?? defaultTarget;
const runtimeTargets = new Map([
  ["bun-darwin-arm64", "macos-aarch64"],
  ["bun-darwin-x64", "macos-x86_64"],
  ["bun-windows-x64", "windows-x86_64"],
  ["bun-windows-x64-baseline", "windows-x86_64"],
]);
const runtimeTarget = target ? runtimeTargets.get(target) : undefined;
if (!target || !runtimeTarget) {
  throw new Error(`当前平台没有受支持的 Pi Runtime 构建目标:${process.platform}/${process.arch}`);
}

const extension = target.includes("windows") ? ".exe" : "";
const targetSuffix = `-${target.replace(/^bun-/, "")}`;
const output = join(sidecarRoot, "dist", `caseboard-pi-runtime${targetSuffix}${extension}`);
mkdirSync(dirname(output), { recursive: true });

const bun = process.env.BUN_BIN || "bun";
const args = ["build", "--compile", `--target=${target}`, "src/main.ts", "--outfile", output];

function compileEnvironment() {
  if (process.platform !== "win32" || target !== "bun-windows-x64-baseline") {
    return process.env;
  }

  // Bun 1.3.10's compile downloader can repeatedly fail to extract its own
  // Windows baseline npm tarball on GitHub runners. Install that exact
  // platform package through the package manager, verify it is the pinned Bun,
  // then seed Bun's documented compile-target cache so the build never depends
  // on an implicit second download.
  const packageBinary = join(
    sidecarRoot,
    "node_modules",
    "@oven",
    "bun-windows-x64-baseline",
    "bin",
    "bun.exe",
  );
  if (!existsSync(packageBinary)) {
    throw new Error(
      "缺少 Windows baseline Bun 编译目标；请先运行 bun install --frozen-lockfile --ignore-scripts",
    );
  }
  const version = spawnSync(packageBinary, ["--version"], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (version.error) throw version.error;
  if (version.status !== 0 || version.stdout.trim() !== runtimeRelease.bun_version) {
    throw new Error(
      `Windows baseline Bun 版本不一致:期望 ${runtimeRelease.bun_version}，发现 ${version.stdout.trim() || "unknown"}`,
    );
  }

  const cacheDir = join(sidecarRoot, ".compile-target-cache");
  mkdirSync(cacheDir, { recursive: true });
  copyFileSync(
    packageBinary,
    join(cacheDir, `${target}-v${runtimeRelease.bun_version}`),
  );
  return { ...process.env, BUN_INSTALL_CACHE_DIR: cacheDir };
}

const result = spawnSync(bun, args, {
  cwd: sidecarRoot,
  stdio: "inherit",
  env: compileEnvironment(),
});
if (result.error) throw result.error;
if (result.status !== 0) process.exit(result.status ?? 1);

// Bun compile leaves a linker-level ad-hoc marker that macOS may reject after the
// executable is copied into App Resources. Re-sign the standalone Mach-O before
// packaging so both the bundled fallback and independent updater health-check cleanly.
if (runtimeTarget.startsWith("macos-")) {
  const signed = spawnSync(
    "/usr/bin/codesign",
    ["--force", "--sign", "-", "--timestamp=none", output],
    { cwd: repositoryRoot, stdio: "inherit" },
  );
  if (signed.error) throw signed.error;
  if (signed.status !== 0) process.exit(signed.status ?? 1);
  const verified = spawnSync(
    "/usr/bin/codesign",
    ["--verify", "--strict", output],
    { cwd: repositoryRoot, stdio: "inherit" },
  );
  if (verified.error) throw verified.error;
  if (verified.status !== 0) process.exit(verified.status ?? 1);
}

const bundleDir = join(sidecarRoot, "dist", "bundle");
rmSync(bundleDir, { recursive: true, force: true });
const packager = spawnSync(
  process.execPath,
  [
    join(repositoryRoot, "scripts", "package-pi-runtime.mjs"),
    "--target", runtimeTarget,
    "--binary", output,
    "--out-dir", join(sidecarRoot, "dist", "release", runtimeTarget),
    "--bundle-dir", bundleDir,
  ],
  { cwd: repositoryRoot, stdio: "inherit" },
);
if (packager.error) throw packager.error;
if (packager.status !== 0) process.exit(packager.status ?? 1);
