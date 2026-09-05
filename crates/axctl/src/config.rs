//! 项目配置读取。
//!
//! 配置来源（优先级从高到低）：
//! 1. `[package.metadata.axctl.dev]`（当前 member 的 dev 嵌套段）
//! 2. `[package.metadata.axctl]`（当前 member 的扁平字段）
//! 3. `[workspace.metadata.axctl.dev]`（workspace 根的 dev 嵌套段）
//! 4. `[workspace.metadata.axctl]`（workspace 根的扁平字段）
//! 5. 默认值
//!
//! 解析由 `cargo metadata` 完成（避免手撸 Cargo.toml 格式细节）。
//! 找不到 metadata 时 fallback 到单 package 模式（直接读 cwd 下的 Cargo.toml）。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo_metadata::{Metadata, MetadataCommand, Package};
use serde::Deserialize;

/// axctl 项目配置。
///
/// 字段保持扁平——`.dev` 嵌套段在读取时被 flatten 到同一结构里。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AxctlConfig {
    /// dev 模式：后端监听端口（让开代理端口）。
    pub backend_port: Option<u16>,
    /// dev 模式：前端 dev server 地址（对应 tauri 的 devUrl）。
    pub frontend_dev_url: Option<String>,
    /// 代理监听地址（默认 127.0.0.1:3000）。
    pub proxy_addr: Option<String>,
    /// 前端根目录（默认 workspace 根）。
    pub frontend_root: Option<String>,
    /// 前端构建命令（默认 pnpm 或 npm）。
    pub frontend_build_command: Option<String>,
    /// dev 模式：后端启动命令模板（支持 `{addr}` 占位符）。
    pub backend_command: Option<String>,
    /// 后端 package 名（必填或自动推断）。
    pub backend_package: Option<String>,
    /// 额外的监听目录（相对 workspace root）。
    pub extra_watch_dirs: Vec<String>,
}

impl AxctlConfig {
    pub fn backend_port_or(&self) -> u16 {
        self.backend_port.unwrap_or(3001)
    }
    pub fn proxy_addr_or(&self) -> String {
        self.proxy_addr
            .clone()
            .unwrap_or_else(|| "127.0.0.1:3000".to_string())
    }
}

/// 解析代理地址字符串（host:port）为 (host, port)。
///
/// host 做字符白名单校验（只允许 hostname / IPv4 / IPv6 字面量合法字符），
/// 拒绝 shell 元字符等——dev 的 proxy_addr / frontend_dev_url 来自项目
/// 配置，host 可能被拼进命令串（serve 的 vite --host），需防注入。
pub fn parse_addr(addr: &str) -> Result<(String, u16)> {
    let (host, port) = addr
        .rsplit_once(':')
        .with_context(|| format!("invalid addr (expected host:port): {addr}"))?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid port in addr: {addr}"))?;
    validate_host(host).with_context(|| format!("invalid host in addr: {addr}"))?;
    Ok((host.to_string(), port))
}

/// host 白名单校验：只允许 hostname / IPv4 / IPv6 字面量字符。
///
/// 允许：字母数字、`.`、`-`、`_`、`:`（IPv6）、`[`/`]`（IPv6 字面量）。
/// 拒绝 `&`、`|`、`;`、空格、`%` 等一切 shell 元字符与路径字符。
fn validate_host(host: &str) -> Result<()> {
    if host.is_empty() {
        anyhow::bail!("host is empty");
    }
    let ok = host.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']')
    });
    if ok {
        Ok(())
    } else {
        anyhow::bail!("host contains disallowed characters")
    }
}

/// 从 start 目录加载 axctl 配置。
///
/// 优先用 cargo metadata（能识别虚拟 manifest / workspace / 当前 member）；
/// 失败时 fallback 单 package 模式。
///
/// 若调用方已有 `cargo_metadata::Metadata`（如 WorkspaceInfo 已 exec 过），
/// 请用 [`load_from_metadata`] 避免重复跑 `cargo metadata`。
pub fn load_from_project(start: &Path) -> Result<AxctlConfig> {
    match MetadataCommand::new()
        .current_dir(start)
        .no_deps()
        .exec()
    {
        Ok(meta) => load_from_metadata_impl(&meta, start),
        Err(error) => {
            tracing::debug!(target: "axctl.config", ?error, "cargo metadata failed; falling back to single-package mode");
            load_from_single_package(start)
        }
    }
}

/// 从已有的 `cargo metadata` 结果加载配置（复用，避免重复 exec）。
pub fn load_from_metadata(meta: &Metadata, start: &Path) -> Result<AxctlConfig> {
    load_from_metadata_impl(meta, start)
}

/// 从 cargo metadata 加载：workspace + 当前 member 两层合并。
fn load_from_metadata_impl(meta: &Metadata, start: &Path) -> Result<AxctlConfig> {
    let workspace_root: PathBuf = meta.workspace_root.clone().into();

    // 1) workspace metadata
    let workspace_cfg =
        read_axctl_section(&workspace_root.join("Cargo.toml"), "workspace")
            .context("failed to parse workspace Cargo.toml")?;

    // 2) 当前 member metadata
    let member_cfg = match find_current_package(meta, start) {
        Some(pkg) => {
            let manifest_path: PathBuf = pkg.manifest_path.clone().into();
            read_axctl_section(&manifest_path, "package")
                .with_context(|| format!("failed to parse {}", manifest_path.display()))?
        }
        None => AxctlConfig::default(),
    };

    // member 覆盖 workspace
    Ok(merge_configs(workspace_cfg, member_cfg))
}

/// 单 package fallback：直接读 cwd 下的 Cargo.toml 的 `[package.metadata.axctl]`。
fn load_from_single_package(root: &Path) -> Result<AxctlConfig> {
    let cargo_toml = root.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return Ok(AxctlConfig::default());
    }
    read_axctl_section(&cargo_toml, "package")
}

/// 在指定顶层段（"workspace" 或 "package"）下读 metadata.axctl。
///
/// 同时识别扁平字段（`[..metadata.axctl]`）和嵌套段（`[..metadata.axctl.dev]`），
/// 嵌套段优先。
fn read_axctl_section(cargo_toml: &Path, top: &str) -> Result<AxctlConfig> {
    if !cargo_toml.is_file() {
        return Ok(AxctlConfig::default());
    }
    let text = std::fs::read_to_string(cargo_toml)
        .with_context(|| format!("failed to read {}", cargo_toml.display()))?;
    let value: toml::Value = toml::from_str(&text)
        .with_context(|| format!("failed to parse {}", cargo_toml.display()))?;

    let section = value
        .get(top)
        .and_then(|s| s.get("metadata"))
        .and_then(|m| m.get("axctl"));

    let flat = match section {
        Some(v) => v
            .clone()
            .try_into()
            .context("failed to parse axctl metadata")?,
        None => AxctlConfig::default(),
    };

    // 嵌套段覆盖
    if let Some(dev_value) = value
        .get(top)
        .and_then(|s| s.get("metadata"))
        .and_then(|m| m.get("axctl"))
        .and_then(|a| a.get("dev"))
    {
        let dev_cfg: AxctlConfig = dev_value
            .clone()
            .try_into()
            .context("failed to parse [..metadata.axctl.dev]")?;
        return Ok(merge_configs(flat, dev_cfg));
    }

    Ok(flat)
}

/// 合并两层配置：标量字段后者覆盖前者，列表字段（extra_watch_dirs）
/// **拼接去重**——member 层追加，不覆盖丢弃 workspace 层配置。
fn merge_configs(base: AxctlConfig, over: AxctlConfig) -> AxctlConfig {
    let mut extra_watch_dirs = base.extra_watch_dirs;
    for dir in over.extra_watch_dirs {
        if !extra_watch_dirs.contains(&dir) {
            extra_watch_dirs.push(dir);
        }
    }

    AxctlConfig {
        backend_port: over.backend_port.or(base.backend_port),
        frontend_dev_url: over.frontend_dev_url.or(base.frontend_dev_url),
        proxy_addr: over.proxy_addr.or(base.proxy_addr),
        frontend_root: over.frontend_root.or(base.frontend_root),
        frontend_build_command: over.frontend_build_command.or(base.frontend_build_command),
        backend_command: over.backend_command.or(base.backend_command),
        backend_package: over.backend_package.or(base.backend_package),
        extra_watch_dirs,
    }
}

/// 用 cargo_metadata 的 packages 找出包含 start 目录的 package。
fn find_current_package<'a>(meta: &'a Metadata, start: &Path) -> Option<&'a Package> {
    let start_abs = crate::workspace::normalize_path(start.canonicalize().ok()?);
    meta.packages.iter().find(|pkg| {
        pkg.manifest_path
            .as_std_path()
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| {
                let pkg_root = crate::workspace::normalize_path(p);
                start_abs.starts_with(&pkg_root)
            })
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_addr_valid_hosts() {
        assert_eq!(parse_addr("127.0.0.1:3000").unwrap(), ("127.0.0.1".into(), 3000));
        assert_eq!(parse_addr("localhost:8080").unwrap(), ("localhost".into(), 8080));
        assert_eq!(parse_addr("my-host.local:3000").unwrap(), ("my-host.local".into(), 3000));
        assert_eq!(parse_addr("[::1]:3000").unwrap(), ("[::1]".into(), 3000));
    }

    #[test]
    fn parse_addr_rejects_injection_host() {
        // shell 元字符与路径字符必须被白名单拒绝（host 可能拼进命令串）
        assert!(parse_addr("127.0.0.1&calc:3000").is_err());
        assert!(parse_addr("x|cmd:3000").is_err());
        assert!(parse_addr("x;y:3000").is_err());
        assert!(parse_addr("x y:3000").is_err());
        assert!(parse_addr(":3000").is_err()); // 空 host
        assert!(parse_addr("host:notaport").is_err());
    }

    #[test]
    fn merge_overrides() {
        let base = AxctlConfig {
            backend_port: Some(3001),
            frontend_dev_url: Some("http://a".into()),
            ..Default::default()
        };
        let over = AxctlConfig {
            backend_port: Some(4000),
            backend_package: Some("foo".into()),
            ..Default::default()
        };
        let merged = merge_configs(base, over);
        assert_eq!(merged.backend_port, Some(4000));
        assert_eq!(merged.frontend_dev_url, Some("http://a".into()));
        assert_eq!(merged.backend_package, Some("foo".into()));
    }

    #[test]
    fn merge_keeps_base_when_over_empty() {
        let base = AxctlConfig {
            backend_port: Some(3001),
            extra_watch_dirs: vec!["templates".into()],
            ..Default::default()
        };
        let over = AxctlConfig::default();
        let merged = merge_configs(base, over);
        assert_eq!(merged.backend_port, Some(3001));
        assert_eq!(merged.extra_watch_dirs, vec!["templates".to_string()]);
    }
}
