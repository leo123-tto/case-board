#!/usr/bin/env node

import { cpSync, existsSync, readdirSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

function fail(message) {
  console.error(`prepare-public-edition: ${message}`);
  process.exit(1);
}

const args = process.argv.slice(2);
const targetArg = args.shift();
if (!targetArg) fail("用法:node scripts/prepare-public-edition.mjs <target> [--template-root <path>] [--allow-git-worktree]");

let allowGitWorktree = false;
let templateRoot;
while (args.length > 0) {
  const arg = args.shift();
  if (arg === "--allow-git-worktree") {
    allowGitWorktree = true;
  } else if (arg === "--template-root") {
    templateRoot = args.shift();
    if (!templateRoot) fail("--template-root 缺少路径");
  } else {
    fail(`未知参数:${arg}`);
  }
}

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const target = path.resolve(targetArg);
const templates = path.resolve(templateRoot ?? path.join(repoRoot, "release", "public-stubs"));

if (!existsSync(target) || !statSync(target).isDirectory()) fail(`目标不是目录:${target}`);
if (existsSync(path.join(target, ".git")) && !allowGitWorktree) {
  fail("拒绝直接修改 Git 工作区;仅 GitHub 临时 checkout 可显式传 --allow-git-worktree");
}

const frontendTarget = path.join(target, "src", "private", "index.tsx");
const backendTarget = path.join(target, "src-tauri", "src", "private", "mod.rs");
const privateUiDir = path.join(target, "src", "private", "dokuritsu");
const frontendStub = path.join(templates, "src", "private", "index.tsx");
const backendStub = path.join(templates, "src-tauri", "src", "private", "mod.rs");

for (const required of [frontendTarget, backendTarget, frontendStub, backendStub]) {
  if (!existsSync(required)) fail(`缺少必要文件:${required}`);
}

rmSync(privateUiDir, { recursive: true, force: true });
cpSync(frontendStub, frontendTarget);
cpSync(backendStub, backendTarget);

const remainingFrontendFiles = readdirSync(path.dirname(frontendTarget)).filter(
  (name) => name !== "index.tsx",
);
if (remainingFrontendFiles.length > 0) {
  fail(`免费版仍有未隔离的前端私人文件:${remainingFrontendFiles.join(",")}`);
}
if (existsSync(privateUiDir)) fail("私人前端目录删除失败");

console.log("免费版工作区已准备:私人前端目录已移除,前后端接缝已替换为不可用桩。");
