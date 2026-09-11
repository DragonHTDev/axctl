#!/usr/bin/env node
// 版本单一来源：Cargo.toml 的 [workspace.package] version。
//
// 同步到根 package.json 与 npm/** 下所有 package.json（含 optionalDependencies
// 里对平台包的版本约束），避免多处手动维护导致发版版本不一致。
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// 1. 从 Cargo.toml 读 [workspace.package] version。
const cargoToml = readFileSync(join(root, "Cargo.toml"), "utf8");
const match = cargoToml.match(
  /\[workspace\.package\][\s\S]*?\bversion\s*=\s*"([^"]+)"/,
);
if (!match) {
  console.error("[sync-version] cannot find [workspace.package] version in Cargo.toml");
  process.exit(1);
}
const version = match[1];

// 2. 收集所有待同步的 package.json。
const targets = [join(root, "package.json")];
for (const dir of ["npm", join("npm", "platforms")]) {
  const base = join(root, dir);
  if (!existsSync(base)) continue;
  for (const name of readdirSync(base)) {
    const pkgJson = join(base, name, "package.json");
    if (existsSync(pkgJson)) targets.push(pkgJson);
  }
}

// 3. 写入 version，并同步 optionalDependencies 的平台包版本。
for (const file of targets) {
  const pkg = JSON.parse(readFileSync(file, "utf8"));
  const previous = pkg.version;
  pkg.version = version;
  if (pkg.optionalDependencies) {
    for (const dep of Object.keys(pkg.optionalDependencies)) {
      pkg.optionalDependencies[dep] = version;
    }
  }
  writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
  const rel = file.slice(root.length + 1).replaceAll("\\", "/");
  console.log(`[sync-version] ${rel}: ${previous} -> ${version}`);
}
