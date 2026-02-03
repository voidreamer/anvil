//! Package resolution and dependency management

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::package::{Package, PackageRequest};

/// Resolved set of packages
#[derive(Debug)]
pub struct ResolvedPackages {
    packages: Vec<Package>,
}

impl ResolvedPackages {
    /// Get the merged environment from all packages
    pub fn environment(&self) -> HashMap<String, String> {
        let mut env: HashMap<String, String> = std::env::vars().collect();
        
        for package in &self.packages {
            let pkg_env = package.resolved_environment(&env);
            env.extend(pkg_env);
        }
        
        env
    }
    
    /// Get list of resolved packages
    pub fn packages(&self) -> &[Package] {
        &self.packages
    }
}

/// Package resolver
pub struct Resolver {
    config: Config,
    /// Cache of loaded packages: name -> version -> Package
    package_cache: HashMap<String, HashMap<String, Package>>,
}

impl Resolver {
    /// Create a new resolver
    pub fn new(config: &Config) -> Result<Self> {
        let mut resolver = Resolver {
            config: config.clone(),
            package_cache: HashMap::new(),
        };
        
        resolver.scan_packages()?;
        
        Ok(resolver)
    }
    
    /// Scan package paths and load all packages
    fn scan_packages(&mut self) -> Result<()> {
        for base_path in self.config.all_package_paths() {
            debug!("Scanning packages in {:?}", base_path);
            
            if !base_path.exists() {
                continue;
            }
            
            // Iterate over package directories
            for entry in std::fs::read_dir(&base_path)? {
                let entry = entry?;
                let pkg_dir = entry.path();
                
                if !pkg_dir.is_dir() {
                    continue;
                }
                
                let pkg_name = pkg_dir.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());
                
                let pkg_name = match pkg_name {
                    Some(n) => n,
                    None => continue,
                };
                
                // Iterate over versions
                for version_entry in std::fs::read_dir(&pkg_dir)? {
                    let version_entry = version_entry?;
                    let version_dir = version_entry.path();
                    
                    if !version_dir.is_dir() {
                        continue;
                    }
                    
                    // Check for package.yaml
                    let package_file = version_dir.join("package.yaml");
                    if !package_file.exists() {
                        continue;
                    }
                    
                    match Package::load(&version_dir) {
                        Ok(pkg) => {
                            debug!("Loaded package: {}-{}", pkg.name, pkg.version);
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
        // Find matching package
        let package = self.find_package(request)?;
        let pkg_id = package.id();
        
        // Skip if already resolved
        if seen.contains(&pkg_id) {
            return Ok(());
        }
        
        // Resolve dependencies first
        for dep_str in &package.requires {
            let dep_request = PackageRequest::parse(dep_str)
                .with_context(|| format!("Invalid dependency: {}", dep_str))?;
            self.resolve_request(&dep_request, resolved, seen)?;
        }
        
        // Add this package
        seen.insert(pkg_id);
        resolved.push(package);
        
        Ok(())
    }
    
    /// Find a package matching a request
    fn find_package(&self, request: &PackageRequest) -> Result<Package> {
        let versions = self.package_cache.get(&request.name)
            .ok_or_else(|| anyhow::anyhow!("Package not found: {}", request.name))?;
        
        // Find matching version
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
        
        // Sort by version and take the highest
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
        
        // Check dependencies exist
        for dep_str in &package.requires {
            let dep_request = PackageRequest::parse(dep_str)?;
            self.find_package(&dep_request)
                .with_context(|| format!("Missing dependency: {}", dep_str))?;
        }
        
        Ok(())
    }
}
