//! Package definition and parsing

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A package definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package name
    pub name: String,
    
    /// Package version
    pub version: String,
    
    /// Human-readable description
    pub description: Option<String>,
    
    /// Required packages (dependencies)
    #[serde(default)]
    pub requires: Vec<String>,
    
    /// Environment variables to set
    #[serde(default)]
    pub environment: IndexMap<String, String>,
    
    /// Command aliases
    #[serde(default)]
    pub commands: HashMap<String, String>,
    
    /// Platform-specific variants
    #[serde(default)]
    pub variants: Vec<PackageVariant>,
    
    /// Path to the package root (set after loading)
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVariant {
    /// Platform filter (linux, windows, macos)
    pub platform: Option<String>,
    
    /// Additional requires for this variant
    #[serde(default)]
    pub requires: Vec<String>,
    
    /// Additional environment for this variant
    #[serde(default)]
    pub environment: IndexMap<String, String>,
}

impl Package {
    /// Load a package from a directory
    pub fn load(path: &Path) -> Result<Self> {
        let package_file = path.join("package.yaml");
        
        if !package_file.exists() {
            anyhow::bail!("Package file not found: {:?}", package_file);
        }
        
        let content = std::fs::read_to_string(&package_file)
            .with_context(|| format!("Failed to read package: {:?}", package_file))?;
        
        let mut package: Package = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse package: {:?}", package_file))?;
        
        package.root = path.to_path_buf();
        
        // Apply variant for current platform
        package.apply_current_variant();
        
        Ok(package)
    }
    
    /// Get the full package identifier (name-version)
    pub fn id(&self) -> String {
        format!("{}-{}", self.name, self.version)
    }
    
    /// Apply the variant matching the current platform
    fn apply_current_variant(&mut self) {
        let current_platform = if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            return;
        };
        
        for variant in &self.variants {
            if variant.platform.as_deref() == Some(current_platform) {
                // Merge variant requires
                self.requires.extend(variant.requires.clone());
                
                // Merge variant environment
                for (key, value) in &variant.environment {
                    self.environment.insert(key.clone(), value.clone());
                }
            }
        }
    }
    
    /// Expand environment variables in a value
    pub fn expand_env_value(&self, value: &str, env: &HashMap<String, String>) -> String {
        let mut result = value.to_string();
        
        // Replace ${PACKAGE_ROOT} with actual path
        result = result.replace("${PACKAGE_ROOT}", &self.root.to_string_lossy());
        
        // Replace ${VERSION} with package version
        result = result.replace("${VERSION}", &self.version);
        
        // Replace ${NAME} with package name
        result = result.replace("${NAME}", &self.name);
        
        // Replace other ${VAR} references
        for (key, val) in env {
            result = result.replace(&format!("${{{}}}", key), val);
        }
        
        // Replace remaining ${VAR} with current environment
        let re = regex::Regex::new(r"\$\{([^}]+)\}").unwrap();
        result = re.replace_all(&result, |caps: &regex::Captures| {
            let var = &caps[1];
            std::env::var(var).unwrap_or_default()
        }).to_string();
        
        result
    }
    
    /// Get resolved environment for this package
    pub fn resolved_environment(&self, base_env: &HashMap<String, String>) -> HashMap<String, String> {
        let mut env = base_env.clone();
        
        for (key, value) in &self.environment {
            let expanded = self.expand_env_value(value, &env);
            env.insert(key.clone(), expanded);
        }
        
        env
    }
}

/// Parse a package request string (e.g., "maya-2024", "arnold-7.2+")
#[derive(Debug, Clone)]
pub struct PackageRequest {
    pub name: String,
    pub version_constraint: VersionConstraint,
}

#[derive(Debug, Clone)]
pub enum VersionConstraint {
    /// Exact version
    Exact(String),
    /// Minimum version (>=)
    Minimum(String),
    /// Range (inclusive)
    Range(String, String),
    /// Multiple options (|)
    OneOf(Vec<String>),
    /// Any version
    Any,
}

impl PackageRequest {
    /// Parse a package request string
    pub fn parse(s: &str) -> Result<Self> {
        // Try to split name and version
        if let Some(idx) = s.rfind('-') {
            let name = &s[..idx];
            let version_part = &s[idx + 1..];
            
            // Parse version constraint
            let constraint = if version_part.ends_with('+') {
                VersionConstraint::Minimum(version_part.trim_end_matches('+').to_string())
            } else if version_part.contains("..") {
                let parts: Vec<&str> = version_part.split("..").collect();
                if parts.len() == 2 {
                    VersionConstraint::Range(parts[0].to_string(), parts[1].to_string())
                } else {
                    anyhow::bail!("Invalid version range: {}", version_part);
                }
            } else if version_part.contains('|') {
                let versions: Vec<String> = version_part.split('|').map(|s| s.to_string()).collect();
                VersionConstraint::OneOf(versions)
            } else {
                VersionConstraint::Exact(version_part.to_string())
            };
            
            Ok(PackageRequest {
                name: name.to_string(),
                version_constraint: constraint,
            })
        } else {
            // Just a name, any version
            Ok(PackageRequest {
                name: s.to_string(),
                version_constraint: VersionConstraint::Any,
            })
        }
    }
    
    /// Check if a version matches this constraint
    pub fn matches(&self, version: &str) -> bool {
        match &self.version_constraint {
            VersionConstraint::Exact(v) => version == v,
            VersionConstraint::Minimum(min) => {
                version_compare(version, min) >= std::cmp::Ordering::Equal
            }
            VersionConstraint::Range(min, max) => {
                version_compare(version, min) >= std::cmp::Ordering::Equal
                    && version_compare(version, max) <= std::cmp::Ordering::Equal
            }
            VersionConstraint::OneOf(versions) => versions.contains(&version.to_string()),
            VersionConstraint::Any => true,
        }
    }
}

/// Simple version comparison
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    // Try semver first
    if let (Ok(va), Ok(vb)) = (semver::Version::parse(a), semver::Version::parse(b)) {
        return va.cmp(&vb);
    }
    
    // Fall back to string comparison
    a.cmp(b)
}
