//! Package resolution and dependency management

use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::cache;
use crate::config::Config;
use crate::context::Lockfile;
use crate::package::{Package, PackageRequest};

/// Resolved set of packages
#[derive(Debug)]
pub struct ResolvedPackages {
    packages: Vec<Package>,
}

impl ResolvedPackages {
    /// Get the merged environment from all packages.
    ///
    /// Emits warnings when a variable explicitly set by one package is
    /// overridden (not appended to) by a later package.
    pub fn environment(&self) -> HashMap<String, String> {
        let mut env: HashMap<String, String> = std::env::vars().collect();

        // Track which package explicitly set each key so we can detect overrides.
        let mut owners: HashMap<String, String> = HashMap::new();

        for package in &self.packages {
            for (key, raw_value) in &package.environment {
                if let Some(prev_pkg) = owners.get(key) {
                    let is_append = raw_value.contains(&format!("${{{}}}", key));
                    if !is_append {
                        warn!(
                            "{} overrides {} (previously set by {})",
                            package.id(),
                            key,
                            prev_pkg
                        );
                    }
                }
                owners.insert(key.clone(), package.id());
            }

            let pkg_env = package.resolved_environment(&env);
            env.extend(pkg_env);
        }

        env
    }

    /// Get list of resolved packages
    pub fn packages(&self) -> &[Package] {
        &self.packages
    }

    /// Build a merged command alias map from all resolved packages.
    pub fn commands(&self) -> HashMap<String, String> {
        let env = self.environment();
        let mut commands = HashMap::new();

        for package in &self.packages {
            for (alias, target) in &package.commands {
                let expanded = package.expand_env_value(target, &env);
                commands.insert(alias.clone(), expanded);
            }
        }

        commands
    }
}

/// Package resolver
pub struct Resolver {
    config: Config,
    /// Cache of loaded packages: name -> version -> Package
    package_cache: HashMap<String, HashMap<String, Package>>,
    /// Version pins from a lockfile (empty when unlocked).
    pins: HashMap<String, String>,
}

impl Resolver {
    /// Create a new resolver, automatically loading `anvil.lock` if present.
    pub fn new(config: &Config) -> Result<Self> {
        let pins = if let Some(lock_path) = Lockfile::find() {
            let lockfile = Lockfile::load(&lock_path)?;
            info!("Using lockfile: {:?}", lock_path);
            lockfile.pins
        } else {
            HashMap::new()
        };

        let mut resolver = Resolver {
            config: config.clone(),
            package_cache: HashMap::new(),
            pins,
        };
        resolver.load_packages()?;
        Ok(resolver)
    }

    /// Create a resolver that ignores any existing lockfile.
    pub fn new_unlocked(config: &Config) -> Result<Self> {
        let mut resolver = Resolver {
            config: config.clone(),
            package_cache: HashMap::new(),
            pins: HashMap::new(),
        };
        resolver.load_packages()?;
        Ok(resolver)
    }

    /// Load packages: try the cache first, fall back to a full scan.
    fn load_packages(&mut self) -> Result<()> {
        let paths = self.config.all_package_paths();
        // Include config state in the cache key so different configs
        // don't share a cache (e.g. different filters or package paths).
        let salt = format!("{:?}{:?}", self.config.package_paths, self.config.filters);

        // Try cache
        if let Some(cached) = cache::load(&paths, &salt) {
            self.package_cache = cached;
            self.apply_filters();
            info!("Loaded {} packages (cached)", self.package_cache.len());
            return Ok(());
        }

        // Full scan
        self.scan_packages()?;
        self.apply_filters();

        // Save to cache (best-effort, before filter so cache stores everything)
        if let Err(e) = cache::save(&paths, &salt, &self.package_cache) {
            debug!("Failed to save cache: {}", e);
        }

        Ok(())
    }

    /// Apply include/exclude filters from config.
    fn apply_filters(&mut self) {
        let filters = &self.config.filters;
        if filters.include.is_empty() && filters.exclude.is_empty() {
            return;
        }

        self.package_cache
            .retain(|name, _| filters.allows(name));
    }

    /// Scan package paths and load all packages.
    fn scan_packages(&mut self) -> Result<()> {
        for base_path in self.config.all_package_paths() {
            debug!("Scanning packages in {:?}", base_path);

            if !base_path.exists() {
                continue;
            }

            for entry in std::fs::read_dir(&base_path)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "yaml" || ext == "yml" {
                        match Package::load_from_file(&path, None) {
                            Ok(pkg) => {
                                debug!("Loaded package (flat): {}-{}", pkg.name, pkg.version);
                                self.package_cache
                                    .entry(pkg.name.clone())
                                    .or_default()
                                    .insert(pkg.version.clone(), pkg);
                            }
                            Err(e) => {
                                warn!("Failed to load package {:?}: {}", path, e);
                            }
                        }
                    }
                } else if path.is_dir() {
                    for version_entry in std::fs::read_dir(&path)? {
                        let version_entry = version_entry?;
                        let version_dir = version_entry.path();

                        if !version_dir.is_dir() {
                            continue;
                        }

                        let package_file = version_dir.join("package.yaml");
                        if !package_file.exists() {
                            continue;
                        }

                        match Package::load(&version_dir) {
                            Ok(pkg) => {
                                debug!("Loaded package (nested): {}-{}", pkg.name, pkg.version);
                                self.package_cache
                                    .entry(pkg.name.clone())
                                    .or_default()
                                    .insert(pkg.version.clone(), pkg);
                            }
                            Err(e) => {
                                warn!("Failed to load package {:?}: {}", version_dir, e);
                            }
                        }
                    }
                }
            }
        }

        info!("Loaded {} packages", self.package_cache.len());
        Ok(())
    }

    /// Resolve a list of package requests
    pub fn resolve(&self, requests: &[String]) -> Result<ResolvedPackages> {
        let mut resolved: Vec<Package> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Expand aliases
        let mut expanded_requests: Vec<String> = Vec::new();
        for req in requests {
            if let Some(alias_packages) = self.config.resolve_alias(req) {
                expanded_requests.extend(alias_packages);
            } else {
                expanded_requests.push(req.clone());
            }
        }

        // Resolve each request
        for req_str in &expanded_requests {
            let request = PackageRequest::parse(req_str)
                .with_context(|| format!("Invalid package request: {}", req_str))?;

            self.resolve_request(&request, &mut resolved, &mut seen)?;
        }

        Ok(ResolvedPackages { packages: resolved })
    }

    /// Resolve a single package request (with dependencies)
    fn resolve_request(
        &self,
        request: &PackageRequest,
        resolved: &mut Vec<Package>,
        seen: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        let package = self.find_package(request)?;
        let pkg_id = package.id();

        if seen.contains(&pkg_id) {
            return Ok(());
        }

        // Resolve dependencies first
        for dep_str in &package.requires {
            let dep_request = PackageRequest::parse(dep_str)
                .with_context(|| format!("Invalid dependency: {}", dep_str))?;
            self.resolve_request(&dep_request, resolved, seen)?;
        }

        seen.insert(pkg_id);
        resolved.push(package);

        Ok(())
    }

    /// Find a package matching a request, preferring a pinned version.
    fn find_package(&self, request: &PackageRequest) -> Result<Package> {
        let versions = self.package_cache.get(&request.name)
            .ok_or_else(|| anyhow::anyhow!("Package not found: {}", request.name))?;

        // Lockfile pin takes priority
        if let Some(pinned) = self.pins.get(&request.name) {
            if let Some(pkg) = versions.get(pinned) {
                debug!("Using pinned version: {}-{}", request.name, pinned);
                return Ok(pkg.clone());
            }
            warn!(
                "Pinned version {}-{} not found, resolving normally",
                request.name, pinned
            );
        }

        let mut matching: Vec<&Package> = versions
            .values()
            .filter(|pkg| request.matches(&pkg.version))
            .collect();

        if matching.is_empty() {
            anyhow::bail!(
                "No matching version for {}: available versions are {:?}",
                request.name,
                versions.keys().collect::<Vec<_>>()
            );
        }

        matching.sort_by(|a, b| {
            if let (Ok(va), Ok(vb)) = (
                semver::Version::parse(&a.version),
                semver::Version::parse(&b.version),
            ) {
                vb.cmp(&va)
            } else {
                b.version.cmp(&a.version)
            }
        });

        Ok(matching[0].clone())
    }

    /// List all available packages
    pub fn list_packages(&self) -> Result<Vec<String>> {
        let mut packages: Vec<String> = self.package_cache.keys().cloned().collect();
        packages.sort();
        Ok(packages)
    }

    /// List versions of a specific package
    pub fn list_versions(&self, name: &str) -> Result<Vec<String>> {
        let versions = self.package_cache.get(name)
            .ok_or_else(|| anyhow::anyhow!("Package not found: {}", name))?;

        let mut version_list: Vec<String> = versions.keys().cloned().collect();
        version_list.sort();
        Ok(version_list)
    }

    /// Get a specific package
    pub fn get_package(&self, id: &str) -> Result<Package> {
        let request = PackageRequest::parse(id)?;
        self.find_package(&request)
    }

    /// Validate a package definition
    pub fn validate_package(&self, id: &str) -> Result<()> {
        let request = PackageRequest::parse(id)?;
        let package = self.find_package(&request)?;

        for dep_str in &package.requires {
            let dep_request = PackageRequest::parse(dep_str)?;
            self.find_package(&dep_request)
                .with_context(|| format!("Missing dependency: {}", dep_str))?;
        }

        Ok(())
    }
}
