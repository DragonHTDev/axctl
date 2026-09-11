#!/usr/bin/env node
// 把编译好的 axctl 二进制组装进对应平台 npm 包（npm/platforms/<pkg>/）。
//
// 本地（当前平台）：
//   cargo build --release -p axctl-rs
//   node scripts/build-npm-packages.mjs
//
// CI（交叉/指定 target）：
//   cargo build --release -p axctl-rs --target <triple>
//   node scripts/build-npm-packages.mjs --triple <triple>
import { chmodSync, copyFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// Rust target triple → { 平台包名, 二进制名 }
const TARGETS = {
  "x86_64-pc-windows-msvc": { pkg: "axctl-win32-x64", bin: "axctl.exe" },
  "x86_64-apple-darwin": { pkg: "axctl-darwin-x64", bin: "axctl" },
  "aarch64-apple-darwin": { pkg: "axctl-darwin-arm64", bin: "axctl" },
  "x86_64-unknown-linux-gnu": { pkg: "axctl-linux-x64", bin: "axctl" },
  "aarch64-unknown-linux-gnu": { pkg: "axctl-linux-arm64", bin: "axctl" },
};

// 当前平台 → triple（未显式 --triple 时使用）。
function detectTriple() {
  const map = {
    "win32-x64": "x86_64-pc-windows-msvc",
    "darwin-x64": "x86_64-apple-darwin",
    "darwin-arm64": "aarch64-apple-darwin",
    "linux-x64": "x86_64-unknown-linux-gnu",
    "linux-arm64": "aarch64-unknown-linux-gnu",
  };
  const triple = map[`${process.platform}-${process.arch}`];
  if (!triple) {
    console.error(`[build-npm] unsupported platform: ${process.platform}-${process.arch}`);
    process.exit(1);
  }
  return triple;
}

// 解析 --triple。
const args = process.argv.slice(2);
const idx = args.indexOf("--triple");
let triple;
if (idx !== -1) {
  triple = args[idx + 1];
  if (!triple) {
    console.error("[build-npm] --triple requires a value");
    process.exit(1);
  }
} else {
  triple = detectTriple();
}

const target = TARGETS[triple];
if (!target) {
  console.error(`[build-npm] unknown triple: ${triple}`);
  console.error(`known: ${Object.keys(TARGETS).join(", ")}`);
  process.exit(1);
}

// 产物目录：优先 target/<triple>/release，回退 target/release（本地未用 --target）。
const src = [
  join(root, "target", triple, "release", target.bin),
  join(root, "target", "release", target.bin),
].find(existsSync);
if (!src) {
  console.error("[build-npm] binary not found. Build it first, e.g.");
  console.error(`  cargo build --release -p axctl-rs --target ${triple}`);
  process.exit(1);
}

const pkgDir = join(root, "npm", "platforms", target.pkg);
const dest = join(pkgDir, target.bin);
mkdirSync(pkgDir, { recursive: true });
copyFileSync(src, dest);
if (process.platform !== "win32") {
  chmodSync(dest, 0o755);
}
const size = (statSync(dest).size / 1024 / 1024).toFixed(1);
console.log(
  `[build-npm] ${triple} -> npm/platforms/${target.pkg}/${target.bin} (${size} MiB)`,
);
