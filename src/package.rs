//! Package definition and parsing

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Platform-native path-list separator, exposed in yaml as `${PATHSEP}`.
#[cfg(target_os = "windows")]
pub const PATHSEP: &str = ";";
#[cfg(not(target_os = "windows"))]
pub const PATHSEP: &str = ":";

/// Platform-native executable suffix, exposed in yaml as `${EXE_SUFFIX}`.
#[cfg(target_os = "windows")]
pub const EXE_SUFFIX: &str = ".exe";
#[cfg(not(target_os = "windows"))]
pub const EXE_SUFFIX: &str = "";

/// A package definition
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    
    /// Path to the package root (set after loading, omitted from package.yaml)
    #[serde(default)]
    pub root: PathBuf,

    /// Path to the YAML file this package was loaded from.  Populated by
    /// the loader; used to compute a content hash for lockfile drift
    /// detection.  Skipped from package.yaml itself but kept in the scan
    /// cache so we don't have to rediscover it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<PathBuf>,
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
    /// Load a package from a directory containing package.yaml
    pub fn load(path: &Path) -> Result<Self> {
        let package_file = path.join("package.yaml");

        if !package_file.exists() {
            anyhow::bail!("Package file not found: {:?}", package_file);
        }

        Self::load_from_file(&package_file, Some(path))
    }

    /// Load a package from a YAML file directly.
    /// If `root` is None, the parent directory of the file is used as the package root.
    ///
    /// Variants are NOT applied here.  The caller (typically the resolver)
    /// chooses a target platform and calls `with_variant_for` to materialise
    /// a per-platform copy.  This lets the resolver do cross-platform lock
    /// resolution from a single cached scan.
    pub fn load_from_file(file_path: &Path, root: Option<&Path>) -> Result<Self> {
        if !file_path.exists() {
            anyhow::bail!("Package file not found: {:?}", file_path);
        }

        let content = std::fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read package: {:?}", file_path))?;

        let mut package: Package = serde_yaml::from_str(&content)
            .with_context(|| format!("Failed to parse package: {:?}", file_path))?;

        package.root = root
            .map(|p| p.to_path_buf())
            .or_else(|| file_path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        package.source_path = Some(file_path.to_path_buf());

        Ok(package)
    }

    /// Get the full package identifier (name-version)
    pub fn id(&self) -> String {
        format!("{}-{}", self.name, self.version)
    }

    /// Compute a SHA-256 hex digest of the package definition file.
    /// Returns None if the source path isn't set or the file can't be read.
    pub fn content_hash(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        let path = self.source_path.as_ref()?;
        let bytes = std::fs::read(path).ok()?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        Some(format!("{:x}", hasher.finalize()))
    }
    
    /// The platform name the running binary identifies as
    /// (linux/macos/windows), or None on unsupported targets.
    pub fn current_platform() -> Option<&'static str> {
        if cfg!(target_os = "linux") {
            Some("linux")
        } else if cfg!(target_os = "windows") {
            Some("windows")
        } else if cfg!(target_os = "macos") {
            Some("macos")
        } else {
            None
        }
    }

    /// Return a copy of this package with the variant for `platform`
    /// merged into its requires/environment.  If `platform` is None,
    /// the current target's platform is used; on unsupported platforms
    /// no variant is applied.
    pub fn with_variant_for(&self, platform: Option<&str>) -> Self {
        let mut out = self.clone();
        let target = platform.or(Self::current_platform());
        if let Some(target) = target {
            for variant in &self.variants {
                if variant.platform.as_deref() == Some(target) {
                    out.requires.extend(variant.requires.clone());
                    for (key, value) in &variant.environment {
                        out.environment.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        out
    }
    
    /// Expand environment variables and tilde in a value
    pub fn expand_env_value(&self, value: &str, env: &HashMap<String, String>) -> String {
        let mut result = value.to_string();

        // Replace ${PACKAGE_ROOT} with actual path
        result = result.replace("${PACKAGE_ROOT}", &self.root.to_string_lossy());

        // Replace ${VERSION} with package version
        result = result.replace("${VERSION}", &self.version);

        // Replace ${NAME} with package name
        result = result.replace("${NAME}", &self.name);

        // Platform-aware builtins so a single yaml line can compose path
        // lists or binary names without a `variants:` fork per platform.
        result = result.replace("${PATHSEP}", PATHSEP);
        result = result.replace("${EXE_SUFFIX}", EXE_SUFFIX);

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

        // Expand `~/` everywhere it appears at a segment boundary
        // (start-of-value, or after `:` / `;`).  Path-list values like
        // `~/USD/bin;~/USD/lib` need every occurrence expanded, not just
        // the first.  `dirs::home_dir()` resolves via `USERPROFILE` on
        // Windows when `HOME` is unset (PowerShell sessions).
        if let Some(home) = dirs::home_dir() {
            let home_str = home.to_string_lossy();
            let tilde_re = regex::Regex::new(r"(^|[:;])~/").unwrap();
            result = tilde_re
                .replace_all(&result, |caps: &regex::Captures| {
                    format!("{}{}/", &caps[1], home_str)
                })
                .to_string();
        }

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

/// Tokenize a command-alias value into `[program, args...]`.
///
/// If the whole value — after tilde expansion — names an existing file, it's
/// treated as a single executable path (so paths with spaces like
/// `/Applications/Houdini 20/bin/hython` work without quoting).  Otherwise the
/// value is split with POSIX shell rules and each token is tilde-expanded.
pub fn tokenize_command(raw: &str) -> Result<Vec<String>> {
    let whole = shellexpand::tilde(raw).into_owned();
    if std::path::Path::new(&whole).is_file() {
        return Ok(vec![whole]);
    }

    let tokens: Vec<String> = shell_words::split(raw)?
        .into_iter()
        .map(|t| shellexpand::tilde(&t).into_owned())
        .collect();
    Ok(tokens)
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

impl VersionConstraint {
    /// Check if a version satisfies this constraint.
    pub fn matches(&self, version: &str) -> bool {
        match self {
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

impl PackageRequest {
    /// Parse a package request string.
    ///
    /// Splits on the last `-` only when the suffix looks like a version (starts
    /// with an ASCII digit).  This allows hyphenated package names such as
    /// `studio-blender-tools` to be used without being misinterpreted.
    pub fn parse(s: &str) -> Result<Self> {
        // Try to split name and version on the last '-'
        if let Some(idx) = s.rfind('-') {
            let name = &s[..idx];
            let version_part = &s[idx + 1..];

            // Only treat the suffix as a version when it begins with a digit.
            // This prevents "studio-blender-tools" from being parsed as
            // name="studio-blender" version="tools".
            let starts_with_digit = version_part
                .chars()
                .next()
                .map_or(false, |c| c.is_ascii_digit());

            if starts_with_digit {
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
                    let versions: Vec<String> =
                        version_part.split('|').map(|s| s.to_string()).collect();
                    VersionConstraint::OneOf(versions)
                } else {
                    VersionConstraint::Exact(version_part.to_string())
                };

                return Ok(PackageRequest {
                    name: name.to_string(),
                    version_constraint: constraint,
                });
            }
        }

        // No hyphen, or suffix doesn't look like a version → any version
        Ok(PackageRequest {
            name: s.to_string(),
            version_constraint: VersionConstraint::Any,
        })
    }
    
    /// Check if a version matches this request's constraint.
    pub fn matches(&self, version: &str) -> bool {
        self.version_constraint.matches(version)
    }
}

impl std::fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionConstraint::Exact(v) => write!(f, "{}", v),
            VersionConstraint::Minimum(v) => write!(f, "{}+", v),
            VersionConstraint::Range(a, b) => write!(f, "{}..{}", a, b),
            VersionConstraint::OneOf(vs) => write!(f, "{}", vs.join("|")),
            VersionConstraint::Any => write!(f, "*"),
        }
    }
}

impl std::fmt::Display for PackageRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.version_constraint {
            VersionConstraint::Any => write!(f, "{}", self.name),
            c => write!(f, "{}-{}", self.name, c),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- PackageRequest::parse ----

    #[test]
    fn parse_exact_version() {
        let req = PackageRequest::parse("maya-2024").unwrap();
        assert_eq!(req.name, "maya");
        assert!(matches!(req.version_constraint, VersionConstraint::Exact(v) if v == "2024"));
    }

    #[test]
    fn parse_semver_exact() {
        let req = PackageRequest::parse("arnold-7.2").unwrap();
        assert_eq!(req.name, "arnold");
        assert!(matches!(req.version_constraint, VersionConstraint::Exact(v) if v == "7.2"));
    }

    #[test]
    fn parse_minimum_version() {
        let req = PackageRequest::parse("maya-2024+").unwrap();
        assert_eq!(req.name, "maya");
        assert!(matches!(req.version_constraint, VersionConstraint::Minimum(v) if v == "2024"));
    }

    #[test]
    fn parse_range_version() {
        let req = PackageRequest::parse("maya-2024..2025").unwrap();
        assert_eq!(req.name, "maya");
        assert!(matches!(req.version_constraint, VersionConstraint::Range(a, b) if a == "2024" && b == "2025"));
    }

    #[test]
    fn parse_oneof_version() {
        let req = PackageRequest::parse("python-3.10|3.11").unwrap();
        assert_eq!(req.name, "python");
        assert!(matches!(req.version_constraint, VersionConstraint::OneOf(ref v) if v == &["3.10", "3.11"]));
    }

    #[test]
    fn parse_any_version() {
        let req = PackageRequest::parse("maya").unwrap();
        assert_eq!(req.name, "maya");
        assert!(matches!(req.version_constraint, VersionConstraint::Any));
    }

    #[test]
    fn parse_hyphenated_name_no_version() {
        let req = PackageRequest::parse("studio-blender-tools").unwrap();
        assert_eq!(req.name, "studio-blender-tools");
        assert!(matches!(req.version_constraint, VersionConstraint::Any));
    }

    #[test]
    fn parse_hyphenated_name_with_version() {
        let req = PackageRequest::parse("studio-blender-tools-1.0.0").unwrap();
        assert_eq!(req.name, "studio-blender-tools");
        assert!(matches!(req.version_constraint, VersionConstraint::Exact(v) if v == "1.0.0"));
    }

    #[test]
    fn parse_hyphenated_name_with_minimum() {
        let req = PackageRequest::parse("studio-python-1.0+").unwrap();
        assert_eq!(req.name, "studio-python");
        assert!(matches!(req.version_constraint, VersionConstraint::Minimum(v) if v == "1.0"));
    }

    #[test]
    fn parse_double_hyphen_no_version() {
        let req = PackageRequest::parse("my-cool-package").unwrap();
        assert_eq!(req.name, "my-cool-package");
        assert!(matches!(req.version_constraint, VersionConstraint::Any));
    }

    // ---- Version matching ----

    #[test]
    fn match_exact() {
        let req = PackageRequest::parse("maya-2024").unwrap();
        assert!(req.matches("2024"));
        assert!(!req.matches("2025"));
    }

    #[test]
    fn match_minimum() {
        let req = PackageRequest::parse("maya-2024+").unwrap();
        assert!(req.matches("2024"));
        assert!(req.matches("2025"));
        assert!(!req.matches("2023"));
    }

    #[test]
    fn match_range() {
        let req = PackageRequest::parse("maya-2024..2025").unwrap();
        assert!(req.matches("2024"));
        assert!(req.matches("2025"));
        assert!(!req.matches("2023"));
        assert!(!req.matches("2026"));
    }

    #[test]
    fn match_oneof() {
        let req = PackageRequest::parse("python-3.10|3.11").unwrap();
        assert!(req.matches("3.10"));
        assert!(req.matches("3.11"));
        assert!(!req.matches("3.9"));
    }

    #[test]
    fn match_any() {
        let req = PackageRequest::parse("maya").unwrap();
        assert!(req.matches("2024"));
        assert!(req.matches("2025"));
        assert!(req.matches("anything"));
    }

    #[test]
    fn match_semver_minimum() {
        let req = PackageRequest::parse("studio-python-1.0.0+").unwrap();
        assert!(req.matches("1.0.0"));
        assert!(req.matches("1.2.0"));
        assert!(req.matches("2.0.0"));
        assert!(!req.matches("0.9.0"));
    }

    // ---- Variable expansion ----

    #[test]
    fn expand_package_root() {
        let pkg = Package {
            name: "test".into(),
            version: "1.0".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/opt/test/1.0"), source_path: None,
        };
        let env = HashMap::new();
        assert_eq!(
            pkg.expand_env_value("${PACKAGE_ROOT}/bin", &env),
            "/opt/test/1.0/bin"
        );
    }

    #[test]
    fn expand_version_and_name() {
        let pkg = Package {
            name: "maya".into(),
            version: "2024".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/opt/maya"), source_path: None,
        };
        let env = HashMap::new();
        assert_eq!(pkg.expand_env_value("${NAME}-${VERSION}", &env), "maya-2024");
    }

    #[test]
    fn expand_pathsep_builtin() {
        let pkg = Package {
            name: "test".into(),
            version: "1.0".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/tmp"), source_path: None,
        };
        let env = HashMap::new();
        let expected = if cfg!(target_os = "windows") {
            "/a;/b;/c"
        } else {
            "/a:/b:/c"
        };
        assert_eq!(
            pkg.expand_env_value("/a${PATHSEP}/b${PATHSEP}/c", &env),
            expected
        );
    }

    #[test]
    fn expand_exe_suffix_builtin() {
        let pkg = Package {
            name: "test".into(),
            version: "1.0".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/tmp"), source_path: None,
        };
        let env = HashMap::new();
        let expected = if cfg!(target_os = "windows") {
            "blender.exe"
        } else {
            "blender"
        };
        assert_eq!(pkg.expand_env_value("blender${EXE_SUFFIX}", &env), expected);
    }

    #[test]
    fn expand_tilde_at_every_segment() {
        // `~` should expand at the start of every path segment, not just the
        // first occurrence in the value.  Path-list values like
        // `~/USD/bin;~/USD/lib` were leaving the second `~` literal before.
        let pkg = Package {
            name: "test".into(),
            version: "1.0".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/tmp"), source_path: None,
        };
        let env = HashMap::new();
        let home = dirs::home_dir().expect("test needs a HOME");
        let home_str = home.to_string_lossy();

        // Unix-style separator
        let unix_in = "~/a:~/b:~/c";
        let unix_out = pkg.expand_env_value(unix_in, &env);
        assert_eq!(
            unix_out,
            format!("{h}/a:{h}/b:{h}/c", h = home_str),
            "Unix-style path list should expand every ~"
        );

        // Windows-style separator
        let win_in = "~/a;~/b;~/c";
        let win_out = pkg.expand_env_value(win_in, &env);
        assert_eq!(
            win_out,
            format!("{h}/a;{h}/b;{h}/c", h = home_str),
            "Windows-style path list should expand every ~"
        );
    }

    #[test]
    fn expand_tilde_only_at_segment_boundary() {
        // A `~` that's not at a segment boundary (e.g. embedded in a word)
        // should be left alone.
        let pkg = Package {
            name: "test".into(),
            version: "1.0".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/tmp"), source_path: None,
        };
        let env = HashMap::new();
        // No `~/` at start or after `:` / `;`, so nothing should change.
        assert_eq!(pkg.expand_env_value("backup~/file", &env), "backup~/file");
    }

    #[test]
    fn expand_from_env_map() {
        let pkg = Package {
            name: "test".into(),
            version: "1.0".into(),
            description: None,
            requires: vec![],
            environment: IndexMap::new(),
            commands: HashMap::new(),
            variants: vec![],
            root: PathBuf::from("/tmp"), source_path: None,
        };
        let mut env = HashMap::new();
        env.insert("HFS".into(), "/opt/houdini".into());
        assert_eq!(
            pkg.expand_env_value("${HFS}/bin:${HFS}/python", &env),
            "/opt/houdini/bin:/opt/houdini/python"
        );
    }
}
