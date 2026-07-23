#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sidecarRoot = path.join(root, "sidecars", "pi-runtime");

function option(name, fallback) {
  const index = process.argv.indexOf(name);
  if (index < 0) return fallback;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} 后缺少值`);
  return value;
}

const target = option("--target");
const binaryPath = option("--binary");
const outDir = path.resolve(option("--out-dir", path.join(sidecarRoot, "dist", "release")));
const bundleDirArg = option("--bundle-dir", null);
if (!target || !binaryPath) throw new Error("用法:--target <platform> --binary <path> [--out-dir <dir>]");
if (!new Set(["macos-aarch64", "macos-x86_64", "windows-x86_64"]).has(target)) {
  throw new Error(`不支持的 Runtime target:${target}`);
}
if (!existsSync(binaryPath) || !statSync(binaryPath).isFile()) {
  throw new Error(`Runtime 二进制不存在:${binaryPath}`);
}

const release = JSON.parse(readFileSync(path.join(sidecarRoot, "runtime-release.json"), "utf8"));
const packageJson = JSON.parse(readFileSync(path.join(sidecarRoot, "package.json"), "utf8"));
if (packageJson.version !== release.runtime_version) {
  throw new Error("package.json 与 runtime-release.json 的 Runtime 版本不一致");
}
for (const dependency of ["@earendil-works/pi-ai", "@earendil-works/pi-coding-agent"]) {
  if (packageJson.dependencies?.[dependency] !== release.pi_sdk_version) {
    throw new Error(`${dependency} 必须精确锁定为 ${release.pi_sdk_version}`);
  }
}

const binaryName = target.startsWith("windows")
  ? "caseboard-pi-runtime.exe"
  : "caseboard-pi-runtime";
const metadata = {
  runtime_version: release.runtime_version,
  pi_sdk_version: release.pi_sdk_version,
  protocol_version: release.protocol_version,
  source_commit: release.source_commit,
  target,
};
const metadataBytes = Buffer.from(`${JSON.stringify(metadata, null, 2)}\n`);
const noticeBytes = Buffer.from(generateThirdPartyNotices());
const binaryBytes = readFileSync(binaryPath);
const entries = [
  { name: binaryName, bytes: binaryBytes, mode: 0o100755 },
  { name: "runtime-metadata.json", bytes: metadataBytes, mode: 0o100644 },
  { name: "THIRD_PARTY_NOTICES.txt", bytes: noticeBytes, mode: 0o100644 },
];
const archive = createStoredZip(entries);

mkdirSync(outDir, { recursive: true });
const zipName = `pi-runtime-${target}.zip`;
const zipPath = path.join(outDir, zipName);
writeFileSync(zipPath, archive);
const sha256 = createHash("sha256").update(archive).digest("hex");
const releasedAt = process.env.SOURCE_DATE_EPOCH
  ? new Date(Number(process.env.SOURCE_DATE_EPOCH) * 1000).toISOString()
  : new Date().toISOString();
const manifest = {
  manifest_version: 1,
  runtime_version: release.runtime_version,
  pi_sdk_version: release.pi_sdk_version,
  protocol_version: release.protocol_version,
  minimum_caseboard_version: release.minimum_caseboard_version,
  source_repository: release.source_repository,
  source_commit: release.source_commit,
  released_at: releasedAt,
  notes: `Pi SDK ${release.pi_sdk_version} · CaseBoard Sidecar ${release.runtime_version}`,
  artifacts: {
    [target]: {
      url: `https://lawtools.top/caseboard/pi-runtime/${release.runtime_version}/${zipName}`,
      size: archive.length,
      sha256,
      signature: "SIGN_SHA256_WITH_CASEBOARD_MINISIGN_KEY",
    },
  },
};
writeFileSync(path.join(outDir, "manifest.draft.json"), `${JSON.stringify(manifest, null, 2)}\n`);

if (bundleDirArg) {
  const bundleDir = path.resolve(bundleDirArg);
  mkdirSync(bundleDir, { recursive: true });
  copyFileSync(binaryPath, path.join(bundleDir, binaryName));
  writeFileSync(path.join(bundleDir, "runtime-metadata.json"), metadataBytes);
  writeFileSync(path.join(bundleDir, "THIRD_PARTY_NOTICES.txt"), noticeBytes);
  if (!target.startsWith("windows")) chmodSync(path.join(bundleDir, binaryName), 0o755);
}

process.stdout.write(`${JSON.stringify({ zip: zipPath, size: archive.length, sha256 })}\n`);

function generateThirdPartyNotices() {
  const nodeModules = path.join(sidecarRoot, "node_modules");
  if (!existsSync(nodeModules)) throw new Error("缺少 Sidecar node_modules；请先执行 bun install --frozen-lockfile");
  const packages = new Map();
  const visitedDirectories = new Set();

  function visitDirectory(directory) {
    let real;
    try {
      real = realpathSync(directory);
    } catch {
      return;
    }
    if (visitedDirectories.has(real)) return;
    visitedDirectories.add(real);
    const packagePath = path.join(real, "package.json");
    if (existsSync(packagePath)) {
      const pkg = JSON.parse(readFileSync(packagePath, "utf8"));
      if (pkg.name && pkg.version) {
        const key = `${pkg.name}@${pkg.version}`;
        const licenseFiles = readdirSync(real)
          .filter((name) => /^(licen[cs]e|copying|notice)(\.|$)/i.test(name))
          .filter((name) => {
            try { return statSync(path.join(real, name)).isFile(); } catch { return false; }
          })
          .map((name) => readFileSync(path.join(real, name), "utf8").trim())
          .filter(Boolean);
        packages.set(key, { license: pkg.license ?? "UNKNOWN", licenseFiles });
      }
      const nested = path.join(real, "node_modules");
      if (existsSync(nested)) visitNodeModules(nested);
      return;
    }
    for (const entry of readdirSync(real)) {
      if (entry === ".bin") continue;
      const child = path.join(real, entry);
      try {
        if (lstatSync(child).isDirectory() || lstatSync(child).isSymbolicLink()) visitDirectory(child);
      } catch {
        // An optional platform dependency may be a dangling link; omit it from this target's notice.
      }
    }
  }

  function visitNodeModules(directory) {
    for (const entry of readdirSync(directory)) {
      if (entry === ".bin") continue;
      visitDirectory(path.join(directory, entry));
    }
  }

  visitNodeModules(nodeModules);
  const inventory = [...packages.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, data]) => `${name}\t${data.license}`)
    .join("\n");
  const licenseGroups = new Map();
  for (const [name, data] of packages) {
    for (const text of data.licenseFiles) {
      const digest = createHash("sha256").update(text).digest("hex");
      const group = licenseGroups.get(digest) ?? { packages: [], text };
      group.packages.push(name);
      licenseGroups.set(digest, group);
    }
  }
  const piLicense = readFileSync(path.join(sidecarRoot, "PI_LICENSE.txt"), "utf8").trim();
  const fullTexts = [...licenseGroups.values()]
    .sort((a, b) => a.packages[0].localeCompare(b.packages[0]))
    .map((group) => `Packages: ${group.packages.sort().join(", ")}\n\n${group.text}`)
    .join(`\n\n${"=".repeat(78)}\n\n`);
  return [
    "CaseBoard Pi Runtime · Third-Party Notices",
    "Generated from the exact Bun-locked dependency tree used for this binary.",
    "",
    "PACKAGE INVENTORY",
    inventory,
    "",
    "PI PROJECT LICENSE (@earendil-works/pi-*)",
    piLicense,
    "",
    "DISCOVERED LICENSE AND NOTICE FILES",
    fullTexts,
    "",
  ].join("\n");
}

function createStoredZip(entries) {
  const localParts = [];
  const centralParts = [];
  let offset = 0;
  for (const entry of entries) {
    const name = Buffer.from(entry.name, "utf8");
    const crc = crc32(entry.bytes);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0, 6);
    local.writeUInt16LE(0, 8);
    local.writeUInt16LE(0, 10);
    local.writeUInt16LE(0x21, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(entry.bytes.length, 18);
    local.writeUInt32LE(entry.bytes.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28);
    localParts.push(local, name, entry.bytes);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(0x0314, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0, 8);
    central.writeUInt16LE(0, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(0x21, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(entry.bytes.length, 20);
    central.writeUInt32LE(entry.bytes.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt16LE(0, 30);
    central.writeUInt16LE(0, 32);
    central.writeUInt16LE(0, 34);
    central.writeUInt16LE(0, 36);
    central.writeUInt32LE((entry.mode << 16) >>> 0, 38);
    central.writeUInt32LE(offset, 42);
    centralParts.push(central, name);
    offset += local.length + name.length + entry.bytes.length;
  }
  const centralDirectory = Buffer.concat(centralParts);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);
  return Buffer.concat([...localParts, centralDirectory, end]);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}
