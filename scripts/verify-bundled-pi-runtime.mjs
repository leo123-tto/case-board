#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const repositoryRoot = path.resolve(path.dirname(scriptPath), "..");
const supportedTargets = new Map([
  ["macos-aarch64", { platform: "darwin", arch: "arm64", binary: "caseboard-pi-runtime" }],
  ["macos-x86_64", { platform: "darwin", arch: "x64", binary: "caseboard-pi-runtime" }],
  ["windows-x86_64", { platform: "win32", arch: "x64", binary: "caseboard-pi-runtime.exe" }],
]);
export const HEALTH_TIMEOUT_MS = 30_000;

function expectedRuntime(target) {
  const expected = supportedTargets.get(target);
  if (!expected) throw new Error(`unsupported Pi Runtime target: ${target}`);
  return expected;
}

export function validateBundleFiles(files, target) {
  const { binary } = expectedRuntime(target);
  const expected = [binary, "runtime-metadata.json", "THIRD_PARTY_NOTICES.txt"].sort();
  const actual = [...files].sort();
  for (const required of expected) {
    if (!actual.includes(required)) throw new Error(`missing required bundle file: ${required}`);
  }
  const unexpected = actual.filter((entry) => !expected.includes(entry));
  if (unexpected.length > 0) throw new Error(`unexpected bundle files: ${unexpected.join(", ")}`);
}

export function validateMetadata(metadata, release) {
  for (const field of [
    "runtime_version",
    "pi_sdk_version",
    "protocol_version",
    "source_commit",
  ]) {
    if (metadata[field] !== release[field]) {
      throw new Error(`${field} mismatch: expected ${release[field]}, found ${metadata[field]}`);
    }
  }
  expectedRuntime(metadata.target);
}

export function validateHealth(health, metadata) {
  const expected = expectedRuntime(metadata.target);
  const checks = [
    ["type", health.type, "health"],
    ["protocol_version", health.protocol_version, metadata.protocol_version],
    ["sidecar_version", health.sidecar_version, metadata.runtime_version],
    ["pi_sdk_version", health.pi_sdk_version, metadata.pi_sdk_version],
    ["platform", health.platform, expected.platform],
    ["architecture", health.arch, expected.arch],
  ];
  for (const [field, actual, wanted] of checks) {
    if (actual !== wanted) throw new Error(`${field} mismatch: expected ${wanted}, found ${actual}`);
  }
}

export function validateUpdaterEntries(entries, target) {
  const { binary } = expectedRuntime(target);
  for (const name of [binary, "runtime-metadata.json", "THIRD_PARTY_NOTICES.txt"]) {
    const suffix = `/Contents/Resources/pi-runtime/${name}`;
    if (!entries.some((entry) => entry.endsWith(suffix))) {
      throw new Error(`updater is missing ${name}`);
    }
  }
}

function option(name, fallback = null) {
  const index = process.argv.indexOf(name);
  if (index < 0) return fallback;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function minimalRuntimeEnvironment(home) {
  const allowlist = process.platform === "win32"
    ? ["Path", "PATH", "SystemRoot", "SYSTEMROOT", "WINDIR", "TEMP", "TMP"]
    : ["PATH", "TMPDIR"];
  const env = { HOME: home };
  for (const name of allowlist) {
    if (process.env[name]) env[name] = process.env[name];
  }
  if (process.platform === "win32") env.USERPROFILE = home;
  return env;
}

function runHealth(binary, metadata) {
  const home = mkdtempSync(path.join(tmpdir(), "caseboard-pi-health-"));
  try {
    const result = spawnSync(binary, [], {
      input: `${JSON.stringify({
        type: "health_check",
        protocol_version: metadata.protocol_version,
      })}\n`,
      encoding: "utf8",
      env: minimalRuntimeEnvironment(home),
      timeout: HEALTH_TIMEOUT_MS,
      windowsHide: true,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(`health process exited ${result.status}: ${(result.stderr ?? "").trim()}`);
    }
    const lines = result.stdout.split(/\r?\n/).filter((line) => line.trim().length > 0);
    if (lines.length !== 1) throw new Error(`health must emit exactly one JSONL line, found ${lines.length}`);
    let health;
    try {
      health = JSON.parse(lines[0]);
    } catch {
      throw new Error("health output is not valid JSON");
    }
    validateHealth(health, metadata);
    return health;
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
}

function verifyUpdater(updaterPath, target) {
  const result = spawnSync("tar", ["-tzf", updaterPath], { encoding: "utf8", timeout: 30_000 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`unable to list updater archive: ${result.stderr.trim()}`);
  validateUpdaterEntries(result.stdout.split(/\r?\n/).filter(Boolean), target);
}

export function verifyRuntimeBundle(bundleDir, updaterPath = null) {
  const release = JSON.parse(readFileSync(
    path.join(repositoryRoot, "sidecars", "pi-runtime", "runtime-release.json"),
    "utf8",
  ));
  const packageJson = JSON.parse(readFileSync(
    path.join(repositoryRoot, "sidecars", "pi-runtime", "package.json"),
    "utf8",
  ));
  const metadata = JSON.parse(readFileSync(path.join(bundleDir, "runtime-metadata.json"), "utf8"));
  validateMetadata(metadata, release);
  validateBundleFiles(readdirSync(bundleDir), metadata.target);
  if (packageJson.version !== release.runtime_version) throw new Error("Sidecar package version mismatch");
  for (const name of ["@earendil-works/pi-ai", "@earendil-works/pi-coding-agent"]) {
    if (packageJson.dependencies?.[name] !== release.pi_sdk_version) {
      throw new Error(`${name} is not pinned to ${release.pi_sdk_version}`);
    }
  }
  const notices = path.join(bundleDir, "THIRD_PARTY_NOTICES.txt");
  if (statSync(notices).size < 1_000) throw new Error("THIRD_PARTY_NOTICES.txt is unexpectedly small");
  const binary = path.join(bundleDir, expectedRuntime(metadata.target).binary);
  if (metadata.target.startsWith("macos-")) {
    const signature = spawnSync("/usr/bin/codesign", ["--verify", "--strict", binary], {
      encoding: "utf8",
      timeout: 10_000,
    });
    if (signature.error) throw signature.error;
    if (signature.status !== 0) throw new Error(`Pi Runtime code signature failed: ${signature.stderr.trim()}`);
  }
  const health = runHealth(binary, metadata);
  if (updaterPath) verifyUpdater(updaterPath, metadata.target);
  return { metadata, health };
}

function main() {
  const bundleDir = option("--bundle-dir");
  const updaterPath = option("--updater");
  if (!bundleDir) {
    throw new Error("usage: verify-bundled-pi-runtime.mjs --bundle-dir <dir> [--updater <tar.gz>]");
  }
  const result = verifyRuntimeBundle(path.resolve(bundleDir), updaterPath && path.resolve(updaterPath));
  process.stdout.write(`${JSON.stringify({
    state: "verified",
    target: result.metadata.target,
    runtime_version: result.health.sidecar_version,
    pi_sdk_version: result.health.pi_sdk_version,
  })}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`Pi Runtime bundle verification failed: ${error instanceof Error ? error.message : error}\n`);
    process.exit(1);
  }
}
