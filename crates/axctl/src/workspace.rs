//! workspace 元数据解析。
//!
//! 整个模块只做一件事：把 `cargo metadata` 的输出整理成 Rust 友好的结构，
//! 并提供基于它的查询（按目录找 member、按名字找、按 binary 反查、闭包计算）。
//!
//! 不在这里写配置解析（那是 `config.rs` 的事），也不在这里建监听集合
//! （那是 `watch_set.rs` 的事）。本模块保持纯函数风格，便于测试。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo_metadata::{Metadata, MetadataCommand, PackageId, Target};

/// workspace 元数据：从 `cargo metadata` 解析得到。
#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    /// workspace 根（含 `[workspace]` 的 Cargo.toml 所在目录）。
    pub root: PathBuf,
    /// target 目录（可能在 workspace 外——用户可配 `CARGO_TARGET_DIR`）。
    pub target_dir: PathBuf,
    /// 所有 workspace member。
    pub members: Vec<MemberInfo>,
    /// PackageId → 在 members 中的下标（加速闭包计算）。
    by_id: HashMap<PackageId, usize>,
}

/// workspace 中的单个 package（即 Cargo 的 member）。
#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub package_id: PackageId,
    pub name: String,
    /// member 根目录（manifest_path 的父目录，已 canonicalize）。
    pub root: PathBuf,
    /// Cargo.toml 绝对路径。
    pub manifest_path: PathBuf,
    /// binary / lib targets。
    pub targets: Vec<Target>,
    /// 该 member 依赖的 workspace 内 member（PackageId 列表）。
    ///
    /// 来源是 `cargo metadata` 的 resolve.nodes（已解析的依赖图），
    /// 而非 package.dependencies 的 TOML 声明——这样：
    /// - rename 依赖（`foo = { path, package = "bar" }`）能正确解析到 bar；
    /// - dev-dependencies **不计入**（构建 backend 不需要 dev-dep 的源码，
    ///   且 dev-dep 常引入非 workspace 的测试依赖，监听它们无意义）。
    ///   只统计 normal + build 依赖。
    pub path_dep_ids: Vec<PackageId>,
}

impl WorkspaceInfo {
    /// 在 `start` 目录运行 `cargo metadata`，返回 workspace 信息。
    ///
    /// `start` 通常是用户运行 axctl 的 cwd；Cargo 会自动向上找 workspace root。
    ///
    /// 注意：不能加 `--no-deps`——本模块需要 `resolve` 字段（依赖图）来
    /// 解析 path 依赖闭包，`no_deps` 会让 `meta.resolve` 为空、闭包失效。
    pub fn load(start: &Path) -> Result<Self> {
        let meta = MetadataCommand::new()
            .current_dir(start)
            .exec()
            .context("failed to run `cargo metadata`")?;

        Self::from_metadata(&meta)
    }

    /// 从已有的 Metadata 构造 WorkspaceInfo（供复用 cargo metadata 结果）。
    pub fn from_metadata(meta: &Metadata) -> Result<Self> {
        let workspace_root: PathBuf = meta.workspace_root.clone().into();
        let target_dir: PathBuf = meta.target_directory.clone().into();

        let workspace_ids: HashSet<&PackageId> =
            meta.workspace_packages().iter().map(|p| &p.id).collect();

        let mut members = Vec::with_capacity(meta.workspace_packages().len());
        let mut by_id = HashMap::new();

        for (idx, pkg) in meta.workspace_packages().iter().enumerate() {
            let manifest_path: PathBuf = pkg.manifest_path.clone().into();
            let root = manifest_path
                .parent()
                .with_context(|| format!("package {} has no parent dir", pkg.name))?
                .to_path_buf();
            let root_canon = normalize_path(root.canonicalize().unwrap_or(root));

            // 从 resolve.nodes 解析 path 依赖（normal + build，排除 dev）
            let path_dep_ids = collect_path_dep_ids(meta, &pkg.id, &workspace_ids);

            by_id.insert(pkg.id.clone(), idx);

            members.push(MemberInfo {
                package_id: pkg.id.clone(),
                name: pkg.name.clone(),
                root: root_canon,
                manifest_path,
                targets: pkg.targets.clone(),
                path_dep_ids,
            });
        }

        Ok(Self {
            root: workspace_root,
            target_dir,
            members,
            by_id,
        })
    }

    /// 用目录找所属 member（最长前缀匹配）。
    pub fn member_for_dir(&self, dir: &Path) -> Option<&MemberInfo> {
        let dir_canon = normalize_path(dir.canonicalize().ok()?);
        self.members
            .iter()
            .filter(|m| dir_canon.starts_with(&m.root))
            .max_by_key(|m| m.root.components().count())
    }

    pub fn member_by_name(&self, name: &str) -> Option<&MemberInfo> {
        self.members.iter().find(|m| m.name == name)
    }

    pub fn member_by_id(&self, id: &PackageId) -> Option<&MemberInfo> {
        self.by_id.get(id).map(|&i| &self.members[i])
    }

    /// 按 binary target 名反查所属 member。
    pub fn package_for_bin(&self, bin_name: &str) -> Option<&MemberInfo> {
        self.members.iter().find(|m| {
            m.targets.iter().any(|t| {
                t.kind
                    .iter()
                    .any(|k| matches!(k, cargo_metadata::TargetKind::Bin))
                    && t.name == bin_name
            })
        })
    }

    /// 计算以 `start_pkg` 为根的 workspace 内 path dependency 闭包（含自身）。
    ///
    /// BFS，只走 workspace 内的 path 依赖（normal + build，排除 dev）。
    pub fn dependency_closure(&self, start_pkg: &PackageId) -> Vec<&MemberInfo> {
        // 先从 PackageId 找成员下标
        let start_idx = self.by_id.get(start_pkg).copied();
        let Some(start_idx) = start_idx else {
            return Vec::new();
        };

        let mut visited: HashSet<usize> = HashSet::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        queue.push_back(start_idx);

        while let Some(idx) = queue.pop_front() {
            if !visited.insert(idx) {
                continue;
            }
            for dep_id in &self.members[idx].path_dep_ids {
                if let Some(&dep_idx) = self.by_id.get(dep_id) {
                    queue.push_back(dep_idx);
                }
            }
        }

        visited.into_iter().map(|i| &self.members[i]).collect()
    }
}

/// 从 resolve.nodes 收集 pkg 的 workspace 内 path 依赖（normal + build，排除 dev）。
///
/// `meta.resolve` 为 None 时（cargo metadata 异常）返回空；正常情况下
/// resolve.nodes 里每个 node 的 deps 已指向解析后的真实 PackageId。
fn collect_path_dep_ids(
    meta: &Metadata,
    pkg_id: &PackageId,
    workspace_ids: &HashSet<&PackageId>,
) -> Vec<PackageId> {
    let Some(resolve) = &meta.resolve else {
        return Vec::new();
    };
    let Some(node) = resolve.nodes.iter().find(|n| &n.id == pkg_id) else {
        return Vec::new();
    };

    node.deps
        .iter()
        .filter(|dep| {
            // 只统计 normal + build 依赖（排除 dev）
            dep.dep_kinds.iter().any(|k| {
                matches!(
                    k.kind,
                    cargo_metadata::DependencyKind::Normal | cargo_metadata::DependencyKind::Build
                )
            })
        })
        .map(|dep| &dep.pkg)
        .filter(|dep_id| workspace_ids.contains(dep_id))
        .cloned()
        .collect()
}

/// 归一化路径：canonicalize 在 Windows 会产出 `\\?\` 前缀的 extended 路径，
/// 与其他来源（配置、cwd）的普通路径格式不一致，导致 `starts_with` 失效。
///
/// 处理规则：
/// - `\\?\C:\...`（盘符）→ 剥前缀，还原为 `C:\...`
/// - `\\?\UNC\host\share`（UNC）→ 还原为 `\\host\share`（网络共享路径）
/// - 其他输入原样返回
pub(crate) fn normalize_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    // UNC 路径的 extended 形式：\\?\UNC\host\share → \\host\share
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_plain_path_unchanged() {
        assert_eq!(
            normalize_path(PathBuf::from(r"D:\proj\server")),
            PathBuf::from(r"D:\proj\server")
        );
    }

    #[test]
    fn normalize_extended_drive_path() {
        assert_eq!(
            normalize_path(PathBuf::from(r"\\?\D:\proj\server")),
            PathBuf::from(r"D:\proj\server")
        );
    }

    #[test]
    fn normalize_extended_unc_path() {
        // \\?\UNC\host\share 应还原为 \\host\share，不能剥成 UNC\host\share
        assert_eq!(
            normalize_path(PathBuf::from(r"\\?\UNC\server\share\dir")),
            PathBuf::from(r"\\server\share\dir")
        );
    }
}
