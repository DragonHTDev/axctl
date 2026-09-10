//! 从 workspace + 配置计算要监听的文件/目录集合。
//!
//! watcher 不再监听整个项目根——它接收一个精确的 `WatchSet`：
//! - 每个相关 member 的 `src/`（递归，需按 Rust 源码过滤）
//! - 每个相关 member 的 `Cargo.toml` + `build.rs`（单文件）
//! - 用户配置的 `extra_watch_dirs`（递归，**任何变化都触发重启**，
//!   不过滤 .rs——它们通常是模板/迁移等资源目录）
//! - 永远忽略：target、node_modules、.git、dist、.axctl
//!
//! "相关 member" 由 backend 的 path dependency 闭包决定，
//! 所以改 src-tauri 不会触发 sealantern-server 重启。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::backend;
use crate::config::AxctlConfig;
use crate::workspace::{MemberInfo, WorkspaceInfo};

/// 要监听的文件/目录集合。
#[derive(Debug, Default, Clone)]
pub struct WatchSet {
    /// 递归监听的 Rust 源码目录（每个相关 member 的 src/）。
    /// 这些目录里的变化需按 Rust 源码过滤（.rs / Cargo.toml / build.rs）。
    pub recursive_dirs: Vec<PathBuf>,
    /// 递归监听、**任意变化都触发**的目录（extra_watch_dirs）。
    pub trigger_all_dirs: Vec<PathBuf>,
    /// 单文件监听（每个相关 member 的 Cargo.toml + build.rs）。
    pub files: Vec<PathBuf>,
    /// 路径中只要出现这些名字就被忽略（事件层兜底）。
    pub ignored_dir_names: Vec<String>,
    /// 命中前缀即忽略（target_dir 等）。
    pub auto_ignored: Vec<PathBuf>,
}

impl WatchSet {
    /// 从 workspace + 配置 + cwd 计算 WatchSet。
    ///
    /// 算法：
    /// 1. 解析 backend member（config.backend_package → cmd 的 -p → cmd 的 --bin
    ///    → cwd 所在 member → 唯一 binary package）
    /// 2. 取 dependency_closure（仅 workspace 内 path 依赖）
    /// 3. 对闭包内每个 member：监听 src/ + Cargo.toml + build.rs
    /// 4. extra_watch_dirs（相对 workspace root）追加到 trigger_all_dirs
    /// 5. ignored 兜底
    pub fn from_config(ws: &WorkspaceInfo, cfg: &AxctlConfig, start_dir: &Path) -> Result<Self> {
        let backend = backend::resolve_backend_member(ws, cfg, start_dir).ok_or_else(|| {
            anyhow!(
                "cannot determine backend package: configure either\n  \
                 [workspace.metadata.axctl.dev].backend_package = \"<name>\"\n  \
                 or backend_command = \"cargo run -p <name>\""
            )
        })?;

        let closure = ws.dependency_closure(&backend.package_id);

        let mut set = WatchSet {
            recursive_dirs: Vec::new(),
            trigger_all_dirs: Vec::new(),
            files: Vec::new(),
            ignored_dir_names: default_ignored_names(),
            auto_ignored: vec![ws.target_dir.clone()],
        };

        for member in closure {
            add_member_to_watch_set(&mut set, member);
        }

        for extra in &cfg.extra_watch_dirs {
            let p = ws.root.join(extra);
            if p.is_dir() {
                set.trigger_all_dirs.push(p);
            } else {
                tracing::warn!(
                    target: "axctl.watcher",
                    path = %p.display(),
                    "extra_watch_dirs entry does not exist or is not a directory; skipped"
                );
            }
        }

        Ok(set)
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.recursive_dirs.is_empty() && self.trigger_all_dirs.is_empty() && self.files.is_empty()
    }

    /// 某路径是否落在"任意变化都触发"的目录下。
    pub fn is_in_trigger_all(&self, path: &Path) -> bool {
        self.trigger_all_dirs
            .iter()
            .any(|dir| path.starts_with(dir))
    }
}

/// 把 member 加入 WatchSet。
fn add_member_to_watch_set(set: &mut WatchSet, member: &MemberInfo) {
    let src = member.root.join("src");
    if src.is_dir() {
        set.recursive_dirs.push(src);
    }
    set.files.push(member.manifest_path.clone());

    // build.rs：cargo metadata 把 build script 也作为一个 target（kind=custom-build）
    for target in &member.targets {
        let is_custom_build = target
            .kind
            .iter()
            .any(|k| matches!(k, cargo_metadata::TargetKind::CustomBuild));
        if is_custom_build {
            let build_script: PathBuf = target.src_path.clone().into();
            if build_script.is_file() {
                set.files.push(build_script);
            }
        }
    }
}

fn default_ignored_names() -> Vec<String> {
    ["target", "node_modules", ".git", "dist", ".axctl"]
        .into_iter()
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// extra_watch_dirs 内的任意文件都应 is_in_trigger_all 命中。
    #[test]
    fn trigger_all_matches_any_file_under_extra_dir() {
        let set = WatchSet {
            recursive_dirs: vec![PathBuf::from("/proj/server/src")],
            trigger_all_dirs: vec![PathBuf::from("/proj/templates")],
            files: vec![],
            ignored_dir_names: vec![],
            auto_ignored: vec![],
        };

        // member src 下的 .rs 属于"需过滤"类（不在 trigger_all）
        assert!(!set.is_in_trigger_all(Path::new("/proj/server/src/main.rs")));
        // extra 目录下任意文件都触发（含非 .rs、含子目录深层）
        assert!(set.is_in_trigger_all(Path::new("/proj/templates/index.html")));
        assert!(set.is_in_trigger_all(Path::new("/proj/templates/partials/header.html")));
        // extra 目录外不触发
        assert!(!set.is_in_trigger_all(Path::new("/proj/other/file.txt")));
    }

    /// 构造一个落在 `root/<name>`、含指定 workspace 内依赖的 member。
    fn member_at(root: &Path, name: &str, deps: &[&str]) -> MemberInfo {
        MemberInfo {
            package_id: cargo_metadata::PackageId { repr: name.to_string() },
            name: name.to_string(),
            root: root.join(name),
            manifest_path: root.join(name).join("Cargo.toml"),
            targets: vec![],
            path_dep_ids: deps
                .iter()
                .map(|d| cargo_metadata::PackageId { repr: (*d).to_string() })
                .collect(),
        }
    }

    /// from_config：按 backend path 闭包收集 src/ 与 Cargo.toml，
    /// 并把存在的 extra_watch_dirs 纳入 trigger_all_dirs。
    #[test]
    fn from_config_collects_closure_and_extra_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in ["a", "b"] {
            std::fs::create_dir_all(root.join(name).join("src")).unwrap();
            std::fs::write(root.join(name).join("Cargo.toml"), "").unwrap();
        }
        std::fs::create_dir_all(root.join("templates")).unwrap();

        let ws = WorkspaceInfo::from_members_for_test(
            root.to_path_buf(),
            root.join("target"),
            vec![member_at(root, "a", &["b"]), member_at(root, "b", &[])],
        );
        let cfg = AxctlConfig {
            backend_package: Some("a".into()),
            extra_watch_dirs: vec!["templates".into()],
            ..Default::default()
        };

        let set = WatchSet::from_config(&ws, &cfg, root).unwrap();

        // 闭包 a（backend）+ b（其依赖）的 src 与 Cargo.toml 均被监听
        assert!(set.recursive_dirs.contains(&root.join("a").join("src")));
        assert!(set.recursive_dirs.contains(&root.join("b").join("src")));
        assert!(set.files.contains(&root.join("a").join("Cargo.toml")));
        assert!(set.files.contains(&root.join("b").join("Cargo.toml")));
        // extra_watch_dirs 命中存在目录 → trigger_all
        assert_eq!(set.trigger_all_dirs, vec![root.join("templates")]);
        // target_dir 进 auto_ignored
        assert!(set.auto_ignored.contains(&root.join("target")));
    }

    /// from_config：extra_watch_dirs 指向不存在的目录 → 跳过而非报错。
    #[test]
    fn from_config_skips_missing_extra_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a").join("src")).unwrap();
        std::fs::write(root.join("a").join("Cargo.toml"), "").unwrap();

        let ws = WorkspaceInfo::from_members_for_test(
            root.to_path_buf(),
            root.join("target"),
            vec![member_at(root, "a", &[])],
        );
        let cfg = AxctlConfig {
            backend_package: Some("a".into()),
            extra_watch_dirs: vec!["nope".into()],
            ..Default::default()
        };

        let set = WatchSet::from_config(&ws, &cfg, root).unwrap();
        assert!(set.trigger_all_dirs.is_empty());
    }

    /// from_config：无法解析 backend（无配置、无唯一 bin）→ 报错并给引导。
    #[test]
    fn from_config_errors_without_backend() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws =
            WorkspaceInfo::from_members_for_test(root.to_path_buf(), root.join("target"), vec![]);
        let err = WatchSet::from_config(&ws, &AxctlConfig::default(), root).unwrap_err();
        assert!(format!("{err:#}").contains("cannot determine backend package"));
    }
}
