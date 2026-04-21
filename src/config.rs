//! Configuration loading and management

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

/// Global configuration for anvil
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Paths to search for packages
    #[serde(default)]
    pub package_paths: Vec<String>,

    /// Default shell for `anvil shell`
    pub default_shell: Option<String>,

    /// Package set aliases
    #[serde(default)]
    pub aliases: std::collections::HashMap<String, Vec<String>>,

    /// Platform-specific overrides
    #[serde(default)]
    pub platform: PlatformConfig,

    /// Lifecycle hooks
    #[serde(default)]
    pub hooks: HooksConfig,

    /// Package include/exclude filters
    #[serde(default)]
    pub filters: FiltersConfig,

    /// `anvil shell` behaviour
    #[serde(default)]
    pub shell: ShellConfig,
}

/// Controls how `anvil shell` composes the interactive subshell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellConfig {
    /// Materialize `commands:` declarations as PATH shims inside the subshell.
    #[serde(default = "default_true")]
    pub inject_commands: bool,

    /// Orphaned shim tempdirs older than this (seconds) are swept on
    /// `anvil shell` entry.  Orphans happen when a shell is SIGKILL'd before
    /// its tempdir handle is cleaned up.
    #[serde(default = "default_orphan_ttl")]
    pub orphan_ttl: u64,
}

fn default_true() -> bool {
    true
}

fn default_orphan_ttl() -> u64 {
    3600
}

impl Default for ShellConfig {
    fn default() -> Self {
        ShellConfig {
            inject_commands: true,
            orphan_ttl: 3600,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformConfig {
    pub linux: Option<PlatformOverrides>,
    pub windows: Option<PlatformOverrides>,
    pub macos: Option<PlatformOverrides>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformOverrides {
    pub package_paths: Option<Vec<String>>,
}

/// Lifecycle hooks: shell commands run at specific points.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    /// Run before package resolution.
    #[serde(default)]
    pub pre_resolve: Vec<String>,
    /// Run after package resolution (receives resolved package list as env).
    #[serde(default)]
    pub post_resolve: Vec<String>,
    /// Run before a command is executed via `anvil run`.
    #[serde(default)]
    pub pre_run: Vec<String>,
    /// Run after a command finishes via `anvil run`.
    #[serde(default)]
    pub post_run: Vec<String>,
}

/// Package include/exclude filters.  When `include` is non-empty, only
/// matching packages are visible.  `exclude` patterns are applied after
/// include.  Patterns use glob syntax (`*`, `?`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FiltersConfig {
    /// Only allow packages whose names match at least one pattern.
    #[serde(default)]
    pub include: Vec<String>,
    /// Hide packages whose names match any pattern.
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl FiltersConfig {
    /// Return true if the package name passes the include/exclude filters.
    pub fn allows(&self, name: &str) -> bool {
        // If include list is non-empty, name must match at least one pattern.
        if !self.include.is_empty() {
            let included = self.include.iter().any(|pat| glob_match(pat, name));
            if !included {
                return false;
            }
        }
        // Exclude overrides include.
        !self.exclude.iter().any(|pat| glob_match(pat, name))
    }
}

/// Simple glob matching supporting `*` (any chars) and `?` (single char).
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner(&p, &t, 0, 0)
}

fn glob_match_inner(pattern: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }
    if pattern[pi] == '*' {
        // '*' matches zero or more characters
        for skip in ti..=text.len() {
            if glob_match_inner(pattern, text, pi + 1, skip) {
                return true;
            }
        }
        return false;
    }
    if ti == text.len() {
        return false;
    }
    if pattern[pi] == '?' || pattern[pi] == text[ti] {
        return glob_match_inner(pattern, text, pi + 1, ti + 1);
    }
    false
}

impl Config {
    /// Load configuration from default locations, then merge any project-local
    /// `.anvil.yaml` found in the current directory or its ancestors.
    pub fn load() -> Result<Self> {
        let global_path = Self::config_path();

        let mut config = if global_path.exists() {
            let content = std::fs::read_to_string(&global_path)
                .with_context(|| format!("Failed to read config: {:?}", global_path))?;
            serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse config: {:?}", global_path))?
        } else {
            let mut c = Config::default();
            c.package_paths = Self::default_package_paths();
            c
        };

        // Merge project-local config if present (walks CWD upward)
        let global_canonical = global_path.canonicalize().ok();
        if let Some(project_path) = Self::find_project_config(global_canonical.as_deref()) {
            info!("Loading project config: {:?}", project_path);
            let content = std::fs::read_to_string(&project_path)
                .with_context(|| format!("Failed to read project config: {:?}", project_path))?;
            let project: Config = serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse project config: {:?}", project_path))?;
            config.merge(project);
        }

        // Apply platform overrides and expand paths after merging
        config.apply_platform_overrides();
        config.expand_paths();

        Ok(config)
    }

    /// Get config file path
    pub fn config_path() -> PathBuf {
        if let Ok(path) = std::env::var("ANVIL_CONFIG") {
            return PathBuf::from(path);
        }

        if let Some(home) = dirs::home_dir() {
            let path = home.join(".anvil.yaml");
            if path.exists() {
                return path;
            }
            let xdg_path = home.join(".config/anvil/config.yaml");
            if xdg_path.exists() {
                return xdg_path;
            }
        }

        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".anvil.yaml")
    }

    /// Default package search paths
    fn default_package_paths() -> Vec<String> {
        let mut paths = Vec::new();

        if let Ok(pkg_path) = std::env::var("ANVIL_PACKAGES") {
            for p in pkg_path.split(':') {
                paths.push(p.to_string());
            }
        }

        if let Some(home) = dirs::home_dir() {
            paths.push(home.join("packages").to_string_lossy().to_string());
            paths.push(
                home.join(".local/share/anvil/packages")
                    .to_string_lossy()
                    .to_string(),
            );
        }

        paths.push("/opt/packages".to_string());

        paths
    }

    /// Apply platform-specific overrides
    fn apply_platform_overrides(&mut self) {
        let overrides = if cfg!(target_os = "linux") {
            self.platform.linux.as_ref()
        } else if cfg!(target_os = "windows") {
            self.platform.windows.as_ref()
        } else if cfg!(target_os = "macos") {
            self.platform.macos.as_ref()
        } else {
            None
        };

        if let Some(overrides) = overrides {
            if let Some(paths) = &overrides.package_paths {
                self.package_paths.extend(paths.clone());
            }
        }
    }

    /// Expand environment variables and ~ in paths
    fn expand_paths(&mut self) {
        self.package_paths = self
            .package_paths
            .iter()
            .map(|p| {
                shellexpand::full(p)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| p.clone())
            })
            .collect();
    }

    /// Get all package paths (with deduplication)
    pub fn all_package_paths(&self) -> Vec<PathBuf> {
        let mut seen = std::collections::HashSet::new();
        self.package_paths
            .iter()
            .filter_map(|p| {
                let path = PathBuf::from(p);
                if seen.insert(path.clone()) && path.exists() {
                    Some(path)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Resolve an alias to package list
    pub fn resolve_alias(&self, name: &str) -> Option<Vec<String>> {
        self.aliases.get(name).cloned()
    }

    /// Walk from the current directory upward looking for `.anvil.yaml`.
    /// Skips the loaded global config **and** all well-known global config
    /// locations so that they are never mistaken for a project-local config.
    fn find_project_config(global_config: Option<&std::path::Path>) -> Option<PathBuf> {
        let mut dir = std::env::current_dir().ok()?;

        let home = dirs::home_dir();
        let skip: Vec<PathBuf> = [
            global_config.map(|p| p.to_path_buf()),
            home.as_ref().map(|h| h.join(".anvil.yaml")),
            home.as_ref().map(|h| h.join(".config/anvil/config.yaml")),
        ]
        .into_iter()
        .flatten()
        .filter_map(|p| p.canonicalize().ok())
        .collect();

        loop {
            let candidate = dir.join(".anvil.yaml");
            if candidate.exists() {
                if let Some(canonical) = candidate.canonicalize().ok() {
                    if skip.iter().any(|s| *s == canonical) {
                        return None;
                    }
                }
                return Some(candidate);
            }
            if !dir.pop() {
                return None;
            }
        }
    }

    /// Merge another config into this one.
    fn merge(&mut self, project: Config) {
        // Project paths come first (higher priority)
        let mut merged = project.package_paths;
        merged.append(&mut self.package_paths);
        self.package_paths = merged;

        // Project aliases override global ones with the same name
        self.aliases.extend(project.aliases);

        // Project shell overrides global
        if project.default_shell.is_some() {
            self.default_shell = project.default_shell;
        }

        // Hooks: project hooks are prepended
        let mut pre_resolve = project.hooks.pre_resolve;
        pre_resolve.extend(self.hooks.pre_resolve.drain(..));
        self.hooks.pre_resolve = pre_resolve;

        let mut post_resolve = project.hooks.post_resolve;
        post_resolve.extend(self.hooks.post_resolve.drain(..));
        self.hooks.post_resolve = post_resolve;

        let mut pre_run = project.hooks.pre_run;
        pre_run.extend(self.hooks.pre_run.drain(..));
        self.hooks.pre_run = pre_run;

        let mut post_run = project.hooks.post_run;
        post_run.extend(self.hooks.post_run.drain(..));
        self.hooks.post_run = post_run;

        // Filters: project filters replace global (not merged)
        if !project.filters.include.is_empty() || !project.filters.exclude.is_empty() {
            self.filters = project.filters;
        }

        // Shell: project shell config replaces global only if it differs from
        // the default (serde fills in the default when the project omits `shell:`).
        if project.shell != ShellConfig::default() {
            self.shell = project.shell;
        }

        // Merge per-platform paths (project first)
        Self::merge_platform(&mut self.platform.linux, project.platform.linux);
        Self::merge_platform(&mut self.platform.macos, project.platform.macos);
        Self::merge_platform(&mut self.platform.windows, project.platform.windows);
    }

    fn merge_platform(base: &mut Option<PlatformOverrides>, project: Option<PlatformOverrides>) {
        if let Some(proj) = project {
            if let Some(proj_paths) = proj.package_paths {
                let base_override = base.get_or_insert_with(PlatformOverrides::default);
                let mut merged = proj_paths;
                if let Some(existing) = base_override.package_paths.take() {
                    merged.extend(existing);
                }
                base_override.package_paths = Some(merged);
            }
        }
    }

    /// Run a list of hook commands. Returns Err if any hook exits non-zero.
    pub fn run_hooks(hooks: &[String], env: &std::collections::HashMap<String, String>) -> Result<()> {
        for cmd in hooks {
            let shell = if cfg!(target_os = "windows") {
                "cmd"
            } else {
                "sh"
            };
            let flag = if cfg!(target_os = "windows") {
                "/C"
            } else {
                "-c"
            };

            let status = std::process::Command::new(shell)
                .arg(flag)
                .arg(cmd)
                .envs(env)
                .status()
                .with_context(|| format!("Failed to run hook: {}", cmd))?;

            if !status.success() {
                anyhow::bail!("Hook failed (exit {}): {}", status.code().unwrap_or(-1), cmd);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_exact() {
        assert!(glob_match("maya", "maya"));
        assert!(!glob_match("maya", "nuke"));
    }

    #[test]
    fn glob_star() {
        assert!(glob_match("maya-*", "maya-2024"));
        assert!(glob_match("maya-*", "maya-2025"));
        assert!(!glob_match("maya-*", "nuke-15"));
        assert!(glob_match("*-tools", "studio-blender-tools"));
    }

    #[test]
    fn glob_question() {
        assert!(glob_match("maya-202?", "maya-2024"));
        assert!(glob_match("maya-202?", "maya-2025"));
        assert!(!glob_match("maya-202?", "maya-20245"));
    }

    #[test]
    fn filter_include() {
        let f = FiltersConfig {
            include: vec!["maya-*".into(), "arnold-*".into()],
            exclude: vec![],
        };
        assert!(f.allows("maya-2024"));
        assert!(f.allows("arnold-7.2"));
        assert!(!f.allows("nuke-15"));
    }

    #[test]
    fn filter_exclude() {
        let f = FiltersConfig {
            include: vec![],
            exclude: vec!["*-dev".into()],
        };
        assert!(f.allows("maya-2024"));
        assert!(!f.allows("maya-dev"));
    }

    #[test]
    fn filter_include_and_exclude() {
        let f = FiltersConfig {
            include: vec!["maya-*".into()],
            exclude: vec!["*-dev".into()],
        };
        assert!(f.allows("maya-2024"));
        assert!(!f.allows("maya-dev"));
        assert!(!f.allows("nuke-15"));
    }
}
