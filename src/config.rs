//! Configuration loading and management

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// Global configuration for pipeline-config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Paths to search for packages
    #[serde(default)]
    pub package_paths: Vec<String>,
    
    /// Default shell for `pconfig shell`
    pub default_shell: Option<String>,
    
    /// Package set aliases
    #[serde(default)]
    pub aliases: std::collections::HashMap<String, Vec<String>>,
    
    /// Platform-specific overrides
    #[serde(default)]
    pub platform: PlatformConfig,
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

impl Config {
    /// Load configuration from default locations
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();
        
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config: {:?}", config_path))?;
            let mut config: Config = serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse config: {:?}", config_path))?;
            
            // Apply platform overrides
            config.apply_platform_overrides();
            
            // Expand paths
            config.expand_paths();
            
            Ok(config)
        } else {
            // Return default config
            let mut config = Config::default();
            config.package_paths = Self::default_package_paths();
            Ok(config)
        }
    }
    
    /// Get config file path
    pub fn config_path() -> PathBuf {
        // Check environment variable first
        if let Ok(path) = std::env::var("PCONFIG_CONFIG") {
            return PathBuf::from(path);
        }
        
        // Check home directory
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".pconfig.yaml");
            if path.exists() {
                return path;
            }
            // Also check .config/pconfig/config.yaml
            let xdg_path = home.join(".config/pconfig/config.yaml");
            if xdg_path.exists() {
                return xdg_path;
            }
        }
        
        // Default to home directory
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".pconfig.yaml")
    }
    
    /// Default package search paths
    fn default_package_paths() -> Vec<String> {
        let mut paths = Vec::new();
        
        // Environment variable
        if let Ok(pkg_path) = std::env::var("PCONFIG_PACKAGES") {
            for p in pkg_path.split(':') {
                paths.push(p.to_string());
            }
        }
        
        // Home directory packages
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join("packages").to_string_lossy().to_string());
            paths.push(home.join(".local/share/pconfig/packages").to_string_lossy().to_string());
        }
        
        // System paths
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
}
