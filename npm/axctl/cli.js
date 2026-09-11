#!/usr/bin/env node
// axctl npm launcher：按当前平台/架构解析对应平台子包内的二进制并执行。
//
// 真正的 axctl 二进制由平台子包（axctl-<os>-<arch>）携带，通过
// optionalDependencies 只安装当前平台那一个；本文件只做定位与 spawn。
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");

// process.platform + process.arch → 平台子包名。
const PLATFORM_PACKAGES = {
  "win32-x64": "axctl-win32-x64",
  "darwin-x64": "axctl-darwin-x64",
  "darwin-arm64": "axctl-darwin-arm64",
  "linux-x64": "axctl-linux-x64",
  "linux-arm64": "axctl-linux-arm64",
};

// 解析当前平台二进制路径；平台不受支持或平台包缺失时报错退出。
function resolveBinary() {
  const key = `${process.platform}-${process.arch}`;
  const pkg = PLATFORM_PACKAGES[key];
  if (!pkg) {
    console.error(`[axctl] unsupported platform: ${key}`);
    process.exit(1);
  }

  let pkgJsonPath;
  try {
    pkgJsonPath = require.resolve(`${pkg}/package.json`);
  } catch {
    console.error(
      `[axctl] missing platform package "${pkg}" for ${key}.\n` +
        `Reinstall axctl (e.g. \`npm install axctl\`) so the correct ` +
        `platform package is fetched, or build from source.`,
    );
    process.exit(1);
  }

  const bin = process.platform === "win32" ? "axctl.exe" : "axctl";
  return path.join(path.dirname(pkgJsonPath), bin);
}

const result = spawnSync(resolveBinary(), process.argv.slice(2), {
  stdio: "inherit",
});

if (result.error) {
  console.error(`[axctl] failed to run binary: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
