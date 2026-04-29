//! Package resolution and dependency management
//!
//! The resolver is depth-first and deterministic: package requests resolve in
//! the order they're given, transitive dependencies before their parent, and
//! the first version chosen for a name is the version that ships.  It does
//! not backtrack on conflict — instead, every constraint encountered for a
//! name is recorded against the chosen version, and a mismatch produces a
//! diagnostic naming both sides ("X chose 1.0 because A required *, but B
//! requires =2.0").

use std::collections::HashMap;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::cache;
use crate::config::Config;
use crate::context::{Lockfile, Pin};
use crate::package::{tokenize_command, Package, PackageRequest, VersionConstraint};

/// One constraint asked for a package, plus who asked.
#[derive(Debug, Clone)]
struct Requester {
    who: String,
    constraint: VersionConstraint,
}

/// A package name that has already been picked.  The `requesters` list
/// grows as more parts of the graph ask for the same name.
#[derive(Debug)]
struct ChosenPackage {
    version: String,
    requesters: Vec<Requester>,
}

/// Mutable state carried through depth-first resolution.
#[derive(Debug, Default)]
struct ResolveState {
    /// Packages output in dependency order.  Each one has the variant
    /// for `target_platform` already merged.
    resolved: Vec<Package>,
    /// Already-pushed package ids (`name-version`), for cycle short-circuit.
    seen: std::collections::HashSet<String>,
    /// Picked version per package name, plus every constraint seen for it.
    chosen: HashMap<String, ChosenPackage>,
    /// Platform whose variants should be merged into chosen packages.
    /// None means "do not apply any variant."
    target_platform: Option<String>,
}

/// Build a conflict message that names the chosen version, every requester
/// of that name (with their constraints), and pinpoints the failing one.
fn format_conflict(name: &str, chosen: &ChosenPackage) -> String {
    let mut msg = format!(
        "version conflict for '{}': chose {} but a later request is incompatible\n",
        name, chosen.version,
    );
    for r in &chosen.requesters {
        let satisfied = if r.constraint.matches(&chosen.version) {
            "ok"
        } else {
            "INCOMPATIBLE"
        };
        msg.push_str(&format!(
            "  - {} required {}-{}  [{}]\n",
            r.who, name, r.constraint, satisfied,
        ));
    }
    msg.push_str(
        "Resolve by relaxing one side, or pinning the other in anvil.lock.",
    );
    msg
}

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
    /// Pins from a lockfile (empty when unlocked).  Carries version
    /// plus optional content hash for drift detection.
    pins: HashMap<String, Pin>,
}

impl Resolver {
    /// Create a new resolver, automatically loading `anvil.lock` if present.
    /// When `refresh` is true, the package scan cache is bypassed.
    pub fn new(config: &Config, refresh: bool) -> Result<Self> {
        let pins = if let Some(lock_path) = Lockfile::find() {
            let lockfile = Lockfile::load(&lock_path)?;
            info!("Using lockfile: {:?}", lock_path);
            // Overlay per-platform pins so a single lockfile resolved
            // for multiple platforms picks the right entry on each.
            lockfile.effective_pins(Package::current_platform())
        } else {
            HashMap::new()
        };

        let mut resolver = Resolver {
            config: config.clone(),
            package_cache: HashMap::new(),
            pins,
        };
        resolver.load_packages(refresh)?;
        resolver.verify_pin_hashes();
        Ok(resolver)
    }

    /// Create a resolver that ignores any existing lockfile.
    pub fn new_unlocked(config: &Config, refresh: bool) -> Result<Self> {
        let mut resolver = Resolver {
            config: config.clone(),
            package_cache: HashMap::new(),
            pins: HashMap::new(),
        };
        resolver.load_packages(refresh)?;
        Ok(resolver)
    }

    /// Compare each pin's recorded content hash against the package
    /// definition currently on disk.  Mismatches produce a warning so
    /// teams sharing a `package_paths` filesystem can detect upstream
    /// edits that would otherwise silently change resolution behaviour.
    fn verify_pin_hashes(&self) {
        for (name, pin) in &self.pins {
            let Some(expected) = pin.content_hash.as_deref() else {
                continue;
            };
            let Some(versions) = self.package_cache.get(name) else {
                continue;
            };
            let Some(pkg) = versions.get(&pin.version) else {
                continue;
            };
            if let Some(actual) = pkg.content_hash() {
                if actual != expected {
                    warn!(
                        "lockfile drift: {}-{} content hash differs from anvil.lock \
                         (expected {}, got {}) -- re-run `anvil lock` to refresh",
                        name,
                        pin.version,
                        &expected[..12.min(expected.len())],
                        &actual[..12.min(actual.len())],
                    );
                }
            }
        }
    }

    /// Load packages: try the cache first (unless `refresh`), fall back to a full scan.
    fn load_packages(&mut self, refresh: bool) -> Result<()> {
        let paths = self.config.all_package_paths();
        // Include config state in the cache key so different configs
        // don't share a cache (e.g. different filters or package paths).
        // The schema tag invalidates caches written by older anvil binaries
        // that pre-merged platform variants into the cached Package.
        let salt = format!(
            "schema=v2|{:?}|{:?}",
            self.config.package_paths, self.config.filters,
        );

        // Try cache
        if !refresh {
            if let Some(cached) = cache::load(&paths, &salt) {
                self.package_cache = cached;
                self.apply_filters();
                info!("Loaded {} packages (cached)", self.package_cache.len());
                return Ok(());
            }
        } else {
            info!("Bypassing package scan cache (--refresh)");
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

    /// Resolve a list of package requests for the current platform.
    pub fn resolve(&self, requests: &[String]) -> Result<ResolvedPackages> {
        self.resolve_for_platform(requests, Package::current_platform())
    }

    /// Resolve a list of package requests as if running on `platform`.
    /// `None` means "no variant filter" (treat variants as inert).
    pub fn resolve_for_platform(
        &self,
        requests: &[String],
        platform: Option<&str>,
    ) -> Result<ResolvedPackages> {
        let mut state = ResolveState {
            target_platform: platform.map(|s| s.to_string()),
            ..ResolveState::default()
        };

        // Expand aliases
        let mut expanded_requests: Vec<String> = Vec::new();
        for req in requests {
            if let Some(alias_packages) = self.config.resolve_alias(req) {
                expanded_requests.extend(alias_packages);
            } else {
                expanded_requests.push(req.clone());
            }
        }

        // Resolve each top-level request
        for req_str in &expanded_requests {
            let request = PackageRequest::parse(req_str)
                .with_context(|| format!("Invalid package request: {}", req_str))?;
            self.resolve_request(&request, "<request>", &mut state)?;
        }

        Ok(ResolvedPackages {
            packages: state.resolved,
        })
    }

    /// Resolve a single package request (with dependencies).
    ///
    /// `requester` is the id of the package that asked for this one
    /// (or `"<request>"` for top-level requests / `"<lockfile>"` for pins).
    /// It's used for conflict diagnostics — every constraint encountered for
    /// a package name is attributed back to whoever asked for it.
    fn resolve_request(
        &self,
        request: &PackageRequest,
        requester: &str,
        state: &mut ResolveState,
    ) -> Result<()> {
        // If this name has already been chosen, verify the new constraint
        // is satisfied by the chosen version.  No backtracking — the first
        // version wins, and incompatible later constraints become errors.
        if let Some(existing) = state.chosen.get_mut(&request.name) {
            existing.requesters.push(Requester {
                who: requester.to_string(),
                constraint: request.version_constraint.clone(),
            });
            if !request.matches(&existing.version) {
                anyhow::bail!(format_conflict(&request.name, existing));
            }
            return Ok(());
        }

        // Pick a version, then merge in the variant for the target
        // platform so transitive `requires` and `environment` reflect
        // what the locked-for platform actually pulls.
        let raw = self.find_package(request, requester)?;
        let package = raw.with_variant_for(state.target_platform.as_deref());
        let pkg_id = package.id();

        // Record the choice before recursing into deps, so a cycle
        // (A requires B requires A) terminates instead of looping.
        state.chosen.insert(
            request.name.clone(),
            ChosenPackage {
                version: package.version.clone(),
                requesters: vec![Requester {
                    who: requester.to_string(),
                    constraint: request.version_constraint.clone(),
                }],
            },
        );

        if state.seen.contains(&pkg_id) {
            return Ok(());
        }
        state.seen.insert(pkg_id.clone());

        // Resolve dependencies first so parents land after their deps.
        for dep_str in &package.requires {
            let dep_request = PackageRequest::parse(dep_str)
                .with_context(|| format!("Invalid dependency in {}: {}", pkg_id, dep_str))?;
            self.resolve_request(&dep_request, &pkg_id, state)?;
        }

        state.resolved.push(package);
        Ok(())
    }

    /// Find a package matching a request, preferring a pinned version.
    fn find_package(&self, request: &PackageRequest, requester: &str) -> Result<Package> {
        let Some(versions) = self.package_cache.get(&request.name) else {
            anyhow::bail!(
                "Package not found: '{}' (required by {})",
                request.name,
                requester,
            );
        };

        // Lockfile pin takes priority — but only if it satisfies the
        // request's constraint, otherwise we'd silently break the request.
        if let Some(pin) = self.pins.get(&request.name) {
            if let Some(pkg) = versions.get(&pin.version) {
                if request.matches(&pkg.version) {
                    debug!("Using pinned version: {}-{}", request.name, pin.version);
                    return Ok(pkg.clone());
                }
                warn!(
                    "Pinned version {}-{} does not satisfy {} (required by {}); resolving normally",
                    request.name, pin.version, request, requester,
                );
            } else {
                warn!(
                    "Pinned version {}-{} not found; resolving normally",
                    request.name, pin.version,
                );
            }
        }

        let mut matching: Vec<&Package> = versions
            .values()
            .filter(|pkg| request.matches(&pkg.version))
            .collect();

        if matching.is_empty() {
            let mut available: Vec<&String> = versions.keys().collect();
            available.sort();
            let constraint_note = match &request.version_constraint {
                VersionConstraint::Any => String::new(),
                c => format!(" matching '{}'", c),
            };
            anyhow::bail!(
                "No version of '{}'{} (required by {}). Available: [{}]",
                request.name,
                constraint_note,
                requester,
                available
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
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

    /// Get a specific package, with the variant for the current
    /// platform already merged in.  Callers that want the raw package
    /// (no variant applied) can use `find_package` via the resolver's
    /// internal API.
    pub fn get_package(&self, id: &str) -> Result<Package> {
        let request = PackageRequest::parse(id)?;
        Ok(self
            .find_package(&request, "<lookup>")?
            .with_variant_for(Package::current_platform()))
    }

    /// Validate a package definition.  Returns `Err` for fatal problems
    /// (missing deps, parse errors) and `Ok(problems)` listing any
    /// non-fatal command-target issues (caller decides how to surface them).
    pub fn validate_package_report(&self, id: &str) -> Result<Vec<String>> {
        let request = PackageRequest::parse(id)?;
        // Validate against the current platform's view of the package so
        // variant-only commands and requires are checked.
        let package = self
            .find_package(&request, "<validate>")?
            .with_variant_for(Package::current_platform());

        for dep_str in &package.requires {
            let dep_request = PackageRequest::parse(dep_str)?;
            self.find_package(&dep_request, &package.id())
                .with_context(|| format!("Missing dependency: {}", dep_str))?;
        }

        // Check command targets.  Expand ${PACKAGE_ROOT}, ${NAME}, etc.
        // against the package's own env, then tokenize and check the program.
        let base_env: HashMap<String, String> = std::env::vars().collect();
        let pkg_env = package.resolved_environment(&base_env);
        let mut problems: Vec<String> = Vec::new();
        for (alias, target) in &package.commands {
            let expanded = package.expand_env_value(target, &pkg_env);
            let tokens = match tokenize_command(&expanded) {
                Ok(t) => t,
                Err(e) => {
                    problems.push(format!("{}: failed to parse ({})", alias, e));
                    continue;
                }
            };
            let Some(program) = tokens.first() else {
                problems.push(format!("{}: alias resolved to empty string", alias));
                continue;
            };
            if let Err(msg) = check_executable(program) {
                problems.push(format!("{} -> {:?}: {}", alias, program, msg));
            }
        }

        Ok(problems)
    }

    /// Back-compat shim: treat any command-target problems as errors.
    #[cfg(test)]
    pub fn validate_package(&self, id: &str) -> Result<()> {
        let problems = self.validate_package_report(id)?;
        if !problems.is_empty() {
            anyhow::bail!("Command problems:\n  - {}", problems.join("\n  - "));
        }
        Ok(())
    }
}

/// Check that `program` is an existing file that is executable.  Looks up
/// bare names (no slash) on `PATH` via the `which` crate.
fn check_executable(program: &str) -> std::result::Result<(), String> {
    let path = std::path::Path::new(program);
    let resolved: std::path::PathBuf = if path.components().count() > 1 || path.is_absolute() {
        if !path.exists() {
            return Err("file does not exist".into());
        }
        path.to_path_buf()
    } else {
        match which::which(program) {
            Ok(p) => p,
            Err(_) => return Err("not found on PATH".into()),
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&resolved)
            .map_err(|e| format!("stat failed: {}", e))?;
        if !meta.is_file() {
            return Err("not a regular file".into());
        }
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("not executable (no +x bit)".into());
        }
    }
    #[cfg(not(unix))]
    {
        let meta = std::fs::metadata(&resolved)
            .map_err(|e| format!("stat failed: {}", e))?;
        if !meta.is_file() {
            return Err("not a regular file".into());
        }
    }

    Ok(())
}
