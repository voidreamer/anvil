//! Package scan caching for faster repeated resolution.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::package::Package;

/// Cached package scan results.
#[derive(Serialize, Deserialize)]
struct ScanCache {
    /// Fingerprint of the package paths at the time of caching.
    fingerprint: u64,
    /// The cached package data: name -> version -> Package.
    packages: HashMap<String, HashMap<String, Package>>,
}

/// Return the cache file path (`~/.cache/anvil/packages.json`).
fn cache_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "anvil").map(|d| d.cache_dir().join("packages.json"))
}

/// Compute a fingerprint of all package paths by hashing directory entries
/// and their mtimes.  Walks two levels deep to cover both flat files and
/// nested `{name}/{version}/package.yaml` layouts.
///
/// The `config_salt` is hashed in so that config changes (e.g. different
/// filters or package paths) invalidate the cache.
pub fn compute_fingerprint(package_paths: &[PathBuf], config_salt: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config_salt.hash(&mut hasher);

    for base in package_paths {
        if !base.exists() {
            continue;
        }
        base.hash(&mut hasher);
        hash_dir_entries(base, &mut hasher);

        // One level deeper for nested packages
        if let Ok(entries) = std::fs::read_dir(base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    hash_dir_entries(&path, &mut hasher);

                    // Check package.yaml files inside version dirs
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub in sub_entries.flatten() {
                            if sub.path().is_dir() {
                                let pkg_file = sub.path().join("package.yaml");
                                if pkg_file.exists() {
                                    hash_file_mtime(&pkg_file, &mut hasher);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    hasher.finish()
}

fn hash_dir_entries(dir: &Path, hasher: &mut impl Hasher) {
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            // Tuple: (name, mtime_nanos, size).  Including size catches
            // edits where mtime didn't advance at second-resolution, and
            // the count of entries is implicit in the vec length.
            let mut items: Vec<(String, u128, u64)> = entries
                .flatten()
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let meta = e.metadata().ok();
                    let mtime = meta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    (name, mtime, size)
                })
                .collect();
            items.sort();
            items.len().hash(hasher);
            items.hash(hasher);
        }
        Err(_) => {
            // Signal "unreadable" so the hash still changes if a dir flips
            // between readable and not.
            "ERR".hash(hasher);
        }
    }
}

fn hash_file_mtime(path: &Path, hasher: &mut impl Hasher) {
    path.hash(hasher);
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            mtime
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .hash(hasher);
        }
        meta.len().hash(hasher);
    }
}

/// Try to load cached packages.  Returns `Some(packages)` if the cache
/// is valid (fingerprint matches), `None` otherwise.
pub fn load(package_paths: &[PathBuf], config_salt: &str) -> Option<HashMap<String, HashMap<String, Package>>> {
    let path = cache_path()?;
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;
    let cached: ScanCache = serde_json::from_str(&content).ok()?;

    let current_fp = compute_fingerprint(package_paths, config_salt);
    if cached.fingerprint != current_fp {
        debug!("Cache fingerprint mismatch, re-scanning");
        return None;
    }

    info!("Using cached package scan");
    Some(cached.packages)
}

/// Save the scanned packages to the cache file.
pub fn save(
    package_paths: &[PathBuf],
    config_salt: &str,
    packages: &HashMap<String, HashMap<String, Package>>,
) -> Result<()> {
    let path = match cache_path() {
        Some(p) => p,
        None => return Ok(()), // No cache dir available, skip silently
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let cached = ScanCache {
        fingerprint: compute_fingerprint(package_paths, config_salt),
        packages: packages.clone(),
    };

    let content = serde_json::to_string(&cached)?;
    std::fs::write(&path, content)?;
    debug!("Saved package cache to {:?}", path);

    Ok(())
}
