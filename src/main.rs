//! Anvil - Environment resolver for VFX pipelines
//!
//! A fast, lightweight alternative to Rez for managing DCC environments.

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cache;
mod cli;
mod config;
mod context;
mod package;
mod resolver;
mod shell;

use cli::{Cli, Commands, ContextAction};
use config::Config;
use context::{ContextPackage, Lockfile, Pin, SavedContext};
use resolver::Resolver;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Default to WARN so casual `anvil env <pkg>` invocations don't litter
    // stderr with "Loaded N packages" and similar. `-v` / `-vv` step up
    // to info / debug; `RUST_LOG` still wins when set.
    let default_filter = match cli.verbose {
        0 => "anvil=warn",
        1 => "anvil=info",
        _ => "anvil=debug",
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        )
        .init();

    // Load config
    let config = Config::load()?;
    let refresh = cli.refresh;
    let frozen = cli.frozen;

    // --locked: re-resolve the locked request set fresh and diff
    // against the pins on disk before running any command.  Any drift
    // fails the run.
    if cli.locked {
        verify_lockfile_fresh(&config, refresh)?;
    }

    match cli.command {
        Commands::Env { packages, export, json } => {
            cmd_env(&config, &packages, export, json, refresh, frozen)?;
        }
        Commands::Run { packages, env_vars, command } => {
            cmd_run(&config, &packages, &env_vars, &command, refresh, frozen)?;
        }
        Commands::Shell { packages, shell, env_only, no_sweep } => {
            cmd_shell(&config, &packages, shell, refresh, env_only, no_sweep, frozen)?;
        }
        Commands::List { package } => {
            cmd_list(&config, package, refresh)?;
        }
        Commands::Info { package } => {
            cmd_info(&config, &package, refresh)?;
        }
        Commands::Validate { package, strict } => {
            cmd_validate(&config, package, strict, refresh)?;
        }
        Commands::Lock {
            packages,
            update: _,
            all_platforms,
            upgrade_packages,
        } => {
            cmd_lock(&config, &packages, refresh, all_platforms, &upgrade_packages)?;
        }
        Commands::Context { action } => match action {
            ContextAction::Save { packages, output } => {
                cmd_context_save(&config, &packages, &output, refresh, frozen)?;
            }
            ContextAction::Show { file, json, export } => {
                cmd_context_show(&file, json, export)?;
            }
            ContextAction::Run { file, command } => {
                cmd_context_run(&file, &command)?;
            }
            ContextAction::Shell { file, shell } => {
                cmd_context_shell(&config, &file, shell)?;
            }
        },
        Commands::Init { name, version, flat, config: scaffold_config } => {
            if scaffold_config {
                cmd_init_config()?;
            } else {
                let name = name.ok_or_else(|| {
                    anyhow::anyhow!(
                        "anvil init: provide a package name, or pass --config to scaffold ~/.anvil.yaml"
                    )
                })?;
                cmd_init(&name, &version, flat)?;
            }
        }
        Commands::Completions { shell } => {
            Cli::print_completions(shell);
        }
        Commands::Wrap { packages, dir, shell } => {
            cmd_wrap(&config, &packages, &dir, &shell, refresh, frozen)?;
        }
        Commands::Sync => {
            cmd_sync(&config, refresh)?;
        }
        Commands::Tree { packages } => {
            cmd_tree(&config, &packages, refresh, frozen)?;
        }
        Commands::Publish { target, path, flat } => {
            cmd_publish(&target, path.as_deref(), flat)?;
        }
    }

    Ok(())
}

/// Helper: build a Resolver honouring the `--frozen` flag.
fn build_resolver(config: &Config, refresh: bool, frozen: bool) -> Result<Resolver> {
    if frozen {
        Resolver::new_frozen(config, refresh)
    } else {
        Resolver::new(config, refresh)
    }
}

/// Verify that anvil.lock matches a fresh resolution of its own
/// recorded request set.  Any drift -- different version, different
/// content hash, missing or extra package -- aborts with a diff.
/// Called from `main` when `--locked` is set.
fn verify_lockfile_fresh(config: &Config, refresh: bool) -> Result<()> {
    let lock_path = Lockfile::find()
        .ok_or_else(|| anyhow::anyhow!("--locked: no anvil.lock found in this directory or any parent"))?;
    let lockfile = Lockfile::load(&lock_path)?;
    let current = package::Package::current_platform();
    let expected = lockfile.effective_pins(current);

    // Resolve fresh against the same request set.
    let resolver = Resolver::new_unlocked(config, refresh)?;
    let resolved = resolver.resolve(&lockfile.requests)?;
    let mut actual = std::collections::HashMap::new();
    for pkg in resolved.packages() {
        actual.insert(
            pkg.name.clone(),
            Pin {
                version: pkg.version.clone(),
                content_hash: pkg.content_hash(),
            },
        );
    }

    let diffs = Lockfile::diff_pins(&expected, &actual);
    if !diffs.is_empty() {
        let mut msg = String::from("--locked: anvil.lock is stale\n");
        for d in &diffs {
            msg.push_str(&format!("  - {}\n", d));
        }
        msg.push_str("Re-run `anvil lock` to refresh.");
        anyhow::bail!(msg);
    }
    Ok(())
}

/// Resolve packages and print environment
fn cmd_env(
    config: &Config,
    packages: &[String],
    export: bool,
    json: bool,
    refresh: bool,
    frozen: bool,
) -> Result<()> {
    let resolver = build_resolver(config, refresh, frozen)?;
    let resolved = resolver.resolve(packages)?;
    let env = resolved.environment();

    if json {
        println!("{}", serde_json::to_string_pretty(&env)?);
    } else if export {
        for (key, value) in &env {
            println!("export {}=\"{}\"", key, value);
        }
    } else {
        for (key, value) in &env {
            println!("{}={}", key, value);
        }
    }

    Ok(())
}

/// Run a command with resolved environment
fn cmd_run(
    config: &Config,
    packages: &[String],
    env_vars: &[String],
    command: &[String],
    refresh: bool,
    frozen: bool,
) -> Result<()> {
    use std::process::Command;

    // Pre-resolve hooks
    Config::run_hooks(&config.hooks.pre_resolve, &std::env::vars().collect())?;

    let resolver = build_resolver(config, refresh, frozen)?;
    let resolved = resolver.resolve(packages)?;
    let mut env = resolved.environment();

    // Post-resolve hooks
    Config::run_hooks(&config.hooks.post_resolve, &env)?;

    // Add user-specified env vars
    for var in env_vars {
        if let Some((key, value)) = var.split_once('=') {
            env.insert(key.to_string(), value.to_string());
        }
    }

    if command.is_empty() {
        anyhow::bail!("No command specified");
    }

    // Resolve command alias.  A command value may be a bare path (possibly
    // containing spaces, e.g. `/Applications/Houdini 20/bin/hython`), or
    // include baked-in arguments (e.g. `nukex: ${NUKE}/Nuke --nukex`), or
    // whitespace from a script launcher (e.g. `python3.14 ~/USD/bin/usdview`).
    let commands_map = resolved.commands();
    let resolved_cmd = commands_map
        .get(&command[0])
        .cloned()
        .unwrap_or_else(|| command[0].clone());
    let mut tokens = package::tokenize_command(&resolved_cmd)
        .with_context(|| format!("Failed to parse command alias: {:?}", resolved_cmd))?;
    if tokens.is_empty() {
        anyhow::bail!(
            "Command alias for {:?} resolved to an empty string",
            command[0]
        );
    }
    let executable = tokens.remove(0);
    let mut all_args = tokens;
    all_args.extend(command[1..].iter().cloned());

    // Pre-run hooks
    Config::run_hooks(&config.hooks.pre_run, &env)?;

    // Surface the resolved argv at `-v`/`-vv` so when an exec fails with
    // "file not found" the user can see what anvil actually tried to run.
    info!("exec: {} {:?}", executable, all_args);

    let status = Command::new(&executable)
        .args(&all_args)
        .envs(&env)
        .status()?;

    // Post-run hooks (best-effort, don't fail on non-zero)
    let _ = Config::run_hooks(&config.hooks.post_run, &env);

    std::process::exit(status.code().unwrap_or(1));
}

/// Start interactive shell with resolved environment
fn cmd_shell(
    config: &Config,
    packages: &[String],
    shell: Option<String>,
    refresh: bool,
    env_only: bool,
    no_sweep: bool,
    frozen: bool,
) -> Result<()> {
    let resolver = build_resolver(config, refresh, frozen)?;
    let resolved = resolver.resolve(packages)?;
    let mut env = resolved.environment();

    let shell_path = shell
        .or_else(|| config.default_shell.clone())
        .unwrap_or_else(|| shell::detect_shell());

    // Opt-outs, in priority order:
    //   1. --env-only flag
    //   2. ANVIL_DISABLE_COMMAND_SHIMS env var (useful in CI)
    //   3. `shell.inject_commands: false` in config
    let disabled_by_env = std::env::var_os("ANVIL_DISABLE_COMMAND_SHIMS").is_some();
    let inject = !env_only && !disabled_by_env && config.shell.inject_commands;

    if inject {
        if !no_sweep {
            shell::sweep_stale_shims(std::time::Duration::from_secs(config.shell.orphan_ttl));
        }

        let commands = resolved.commands();
        if !commands.is_empty() {
            let shim_dir = shell::materialize_commands(&commands)?;
            shell::prepend_path(&mut env, &shim_dir);
            env.insert(
                "ANVIL_COMMAND_DIR".to_string(),
                shim_dir.to_string_lossy().into_owned(),
            );
        }
    }

    shell::spawn_shell(&shell_path, &env)?;

    Ok(())
}

/// List available packages
fn cmd_list(config: &Config, package: Option<String>, refresh: bool) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;

    if let Some(name) = package {
        // List versions of specific package
        let versions = resolver.list_versions(&name)?;
        println!("{}:", name);
        for version in versions {
            println!("  - {}", version);
        }
    } else {
        // List all packages
        let packages = resolver.list_packages()?;
        if packages.is_empty() {
            if let Some(hint) = config.first_run_hint() {
                eprintln!("No packages found.\n  {}", hint.replace('\n', "\n  "));
            } else {
                eprintln!(
                    "No packages found in any of the configured paths:\n{}",
                    config
                        .all_package_paths()
                        .iter()
                        .map(|p| format!("  - {}", p.display()))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        for pkg in packages {
            println!("{}", pkg);
        }
    }

    Ok(())
}

/// Show package info
fn cmd_info(config: &Config, package: &str, refresh: bool) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;
    let pkg = resolver.get_package(package)?;

    println!("Name: {}", pkg.name);
    println!("Version: {}", pkg.version);
    // When the user asked for a bare name (e.g. `anvil info resolver`) and
    // there are several versions on disk, surface them so the asymmetry
    // between filename (`resolver-1.yaml`) and package name (`resolver`)
    // doesn't hide the others.
    if let Ok(versions) = resolver.list_versions(&pkg.name) {
        if versions.len() > 1 {
            println!("Available versions: {}", versions.join(", "));
        }
    }
    if let Some(desc) = &pkg.description {
        println!("Description: {}", desc);
    }
    if !pkg.requires.is_empty() {
        println!("Requires:");
        for req in &pkg.requires {
            println!("  - {}", req);
        }
    }
    if !pkg.environment.is_empty() {
        println!("Environment:");
        for (key, value) in &pkg.environment {
            println!("  {}: {}", key, value);
        }
    }
    if !pkg.commands.is_empty() {
        println!("Commands:");
        for (alias, target) in &pkg.commands {
            println!("  {}: {}", alias, target);
        }
    }

    Ok(())
}

/// Validate package definitions.
///
/// Dependency problems are always fatal.  Command-target problems
/// (missing / non-executable files) are reported as warnings unless
/// `strict` is set, in which case they fail validation too.
fn cmd_validate(
    config: &Config,
    package: Option<String>,
    strict: bool,
    refresh: bool,
) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;

    let packages = if let Some(name) = package {
        vec![name]
    } else {
        resolver.list_packages()?
    };

    let mut errors = 0;
    let mut warnings = 0;

    for pkg_name in packages {
        let report = resolver.validate_package_report(&pkg_name);
        match report {
            Ok(cmd_problems) => {
                if cmd_problems.is_empty() {
                    println!("✓ {}", pkg_name);
                } else {
                    let label = if strict { "✗" } else { "!" };
                    println!("{} {}: command problems:", label, pkg_name);
                    for p in &cmd_problems {
                        println!("    - {}", p);
                    }
                    if strict {
                        errors += 1;
                    } else {
                        warnings += 1;
                    }
                }
            }
            Err(e) => {
                println!("✗ {}: {}", pkg_name, e);
                errors += 1;
            }
        }
    }

    if errors > 0 {
        anyhow::bail!("{} package(s) failed validation", errors);
    }

    if warnings > 0 {
        println!(
            "\nAll dependencies resolve ({} package(s) with command warnings — use --strict to fail on these).",
            warnings
        );
    } else {
        println!("\nAll packages valid!");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tree
// ---------------------------------------------------------------------------

/// Print the resolved dependency graph as an ASCII tree.  Each top-level
/// request is a root; transitive `requires` form the children.  A node
/// that's already been printed once is shown as `name-version (*)` so
/// shared deps don't multiply the output and cycles terminate.
fn cmd_tree(
    config: &Config,
    packages: &[String],
    refresh: bool,
    frozen: bool,
) -> Result<()> {
    use std::collections::{HashMap, HashSet};

    let resolver = build_resolver(config, refresh, frozen)?;
    let resolved = resolver.resolve(packages)?;

    let by_name: HashMap<String, &package::Package> = resolved
        .packages()
        .iter()
        .map(|p| (p.name.clone(), p))
        .collect();

    let mut shown: HashSet<String> = HashSet::new();

    for (i, req) in packages.iter().enumerate() {
        let request = match package::PackageRequest::parse(req) {
            Ok(r) => r,
            Err(_) => {
                println!("{}  (unparseable request)", req);
                continue;
            }
        };
        let Some(pkg) = by_name.get(&request.name) else {
            println!("{}  (not in resolution)", req);
            continue;
        };
        if i > 0 {
            println!();
        }
        // Roots print without a connector; descendants print under
        // `print_descendants` which manages the column drawing.
        let id = pkg.id();
        let suffix = if shown.contains(&id) { " (*)" } else { "" };
        println!("{}{}", id, suffix);
        if shown.contains(&id) {
            continue;
        }
        shown.insert(id);
        print_descendants(pkg, &by_name, &mut shown, "");
    }

    Ok(())
}

/// Print the dependency subtree of `parent`.  `prefix` is the column
/// drawing accumulated from ancestor branches ("│   " when the
/// ancestor was a non-last sibling, "    " when it was last).
fn print_descendants(
    parent: &package::Package,
    by_name: &std::collections::HashMap<String, &package::Package>,
    shown: &mut std::collections::HashSet<String>,
    prefix: &str,
) {
    let mut deps: Vec<&package::Package> = Vec::new();
    for dep_str in &parent.requires {
        let Ok(req) = package::PackageRequest::parse(dep_str) else { continue };
        if let Some(dep) = by_name.get(&req.name) {
            deps.push(*dep);
        }
    }
    let n = deps.len();
    for (i, dep) in deps.iter().enumerate() {
        let is_last = i + 1 == n;
        let connector = if is_last { "└── " } else { "├── " };
        let id = dep.id();
        let already = shown.contains(&id);
        let suffix = if already { " (*)" } else { "" };
        println!("{}{}{}{}", prefix, connector, id, suffix);
        if already {
            continue;
        }
        shown.insert(id);
        let next_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
        print_descendants(dep, by_name, shown, &next_prefix);
    }
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// Verify every pin in anvil.lock against the package paths on disk.
/// Walks pins (current-platform overlay applied), and for each:
///   - confirms the pinned name+version exists on disk
///   - compares content hashes (if recorded) and reports drift
///   - validates command-alias targets resolve to executables
/// Returns non-zero on any failure; warnings (hash drift, broken
/// command targets) print but don't change the exit code.
fn cmd_sync(config: &Config, refresh: bool) -> Result<()> {
    let lock_path = Lockfile::find()
        .ok_or_else(|| anyhow::anyhow!("anvil sync: no anvil.lock found in this directory or any parent"))?;
    let lockfile = Lockfile::load(&lock_path)?;
    let current = package::Package::current_platform();
    let pins = lockfile.effective_pins(current);

    let resolver = Resolver::new_unlocked(config, refresh)?;

    let platform_label = current.unwrap_or("unknown");
    println!("Checking {} for {}: {} pin(s)", lock_path.display(), platform_label, pins.len());

    let mut ok = 0usize;
    let mut warnings = 0usize;
    let mut failures = 0usize;
    let mut names: Vec<&String> = pins.keys().collect();
    names.sort();

    for name in names {
        let pin = &pins[name];
        let id = format!("{}-{}", name, pin.version);

        // Existence check.
        let pkg = match resolver.get_package(&id) {
            Ok(p) => p,
            Err(e) => {
                println!("  fail  {} -- {}", id, e);
                failures += 1;
                continue;
            }
        };

        // Content hash drift.
        if let Some(expected) = &pin.content_hash {
            match pkg.content_hash() {
                Some(actual) if &actual != expected => {
                    println!(
                        "  warn  {} -- content hash drift (locked {}, on-disk {})",
                        id,
                        &expected[..12.min(expected.len())],
                        &actual[..12.min(actual.len())],
                    );
                    warnings += 1;
                    continue;
                }
                None => {
                    println!("  warn  {} -- pinned hash present but file unreadable", id);
                    warnings += 1;
                    continue;
                }
                _ => {}
            }
        }

        // Validate command targets.
        match resolver.validate_package_report(&id) {
            Ok(problems) if !problems.is_empty() => {
                println!("  warn  {} -- {} command issue(s):", id, problems.len());
                for p in &problems {
                    println!("          {}", p);
                }
                warnings += 1;
            }
            Ok(_) => {
                println!("  ok    {}", id);
                ok += 1;
            }
            Err(e) => {
                println!("  fail  {} -- {}", id, e);
                failures += 1;
            }
        }
    }

    println!(
        "{} ok, {} warning(s), {} failure(s)",
        ok, warnings, failures,
    );
    if failures > 0 {
        anyhow::bail!("anvil sync: {} pin(s) failed verification", failures);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Lock
// ---------------------------------------------------------------------------

/// Resolve packages and write pinned versions to `anvil.lock`.
///
/// When `all_platforms` is true, the resolver runs once per supported
/// platform and the resulting pins are unioned: pins shared by every
/// platform live in `pins`, and pins that differ live under
/// `platform_pins[<platform>]`.  This makes a single lockfile correct
/// on Linux, macOS, and Windows even when a package's variant block
/// pulls in different transitive deps per platform.
fn cmd_lock(
    config: &Config,
    packages: &[String],
    refresh: bool,
    all_platforms: bool,
    upgrade_packages: &[String],
) -> Result<()> {
    // For surgical upgrades, load the existing lockfile and reuse all
    // pins except the names being upgraded.  Without --upgrade-package
    // we still resolve fresh (the historical behaviour).
    let resolver = if upgrade_packages.is_empty() {
        Resolver::new_unlocked(config, refresh)?
    } else {
        let lock_path = Lockfile::find().ok_or_else(|| {
            anyhow::anyhow!(
                "--upgrade-package needs an existing anvil.lock; run `anvil lock` first"
            )
        })?;
        let existing = Lockfile::load(&lock_path)?;
        let mut keep = existing.effective_pins(package::Package::current_platform());
        for name in upgrade_packages {
            if keep.remove(name).is_none() {
                tracing::warn!(
                    "--upgrade-package {}: no existing pin found; resolving fresh",
                    name,
                );
            }
        }
        Resolver::new_unlocked(config, refresh)?.with_pins(keep)
    };

    // Which platforms to resolve for.
    let targets: Vec<&str> = if all_platforms {
        vec!["linux", "macos", "windows"]
    } else {
        match package::Package::current_platform() {
            Some(p) => vec![p],
            None => vec![],
        }
    };

    // Resolve per platform.
    type PinMap = std::collections::HashMap<String, Pin>;
    let mut per_platform: std::collections::BTreeMap<String, PinMap> =
        std::collections::BTreeMap::new();
    for &platform in &targets {
        let resolved = resolver.resolve_for_platform(packages, Some(platform))?;
        let mut pins = PinMap::new();
        for pkg in resolved.packages() {
            pins.insert(
                pkg.name.clone(),
                Pin {
                    version: pkg.version.clone(),
                    content_hash: pkg.content_hash(),
                },
            );
        }
        per_platform.insert(platform.to_string(), pins);
    }

    // Union: any (name, version, hash) shared by *every* resolved
    // platform goes into common `pins`; the rest goes under
    // `platform_pins`.
    let mut common: PinMap = std::collections::HashMap::new();
    let mut platform_pins: std::collections::HashMap<String, PinMap> =
        std::collections::HashMap::new();

    if let Some(first) = per_platform.values().next().cloned() {
        for (name, pin) in first {
            let same_everywhere = per_platform.values().all(|m| {
                m.get(&name)
                    .map(|p| p.version == pin.version && p.content_hash == pin.content_hash)
                    .unwrap_or(false)
            });
            if same_everywhere {
                common.insert(name, pin);
            }
        }
    }
    for (platform, pins) in &per_platform {
        for (name, pin) in pins {
            if !common.contains_key(name) {
                platform_pins
                    .entry(platform.clone())
                    .or_default()
                    .insert(name.clone(), pin.clone());
            }
        }
    }

    let lockfile = Lockfile {
        requests: packages.to_vec(),
        platforms: targets.iter().map(|s| s.to_string()).collect(),
        pins: common,
        platform_pins,
    };

    let lock_path = std::path::PathBuf::from("anvil.lock");
    lockfile.save(&lock_path)?;

    let total: usize = per_platform.values().map(|m| m.len()).sum();
    println!(
        "Locked {} pin(s) across {} platform(s) to anvil.lock",
        lockfile.pins.len()
            + lockfile
                .platform_pins
                .values()
                .map(|m| m.len())
                .sum::<usize>(),
        targets.len(),
    );
    for (name, pin) in &lockfile.pins {
        println!("  {}-{}", name, pin.version);
    }
    for (platform, pins) in &lockfile.platform_pins {
        println!("  [{}]", platform);
        for (name, pin) in pins {
            println!("    {}-{}", name, pin.version);
        }
    }
    let _ = total; // touched for clarity above

    Ok(())
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Resolve packages and save the full environment to a context file.
fn cmd_context_save(
    config: &Config,
    packages: &[String],
    output: &str,
    refresh: bool,
    frozen: bool,
) -> Result<()> {
    let resolver = build_resolver(config, refresh, frozen)?;
    let resolved = resolver.resolve(packages)?;
    let env = resolved.environment();

    let ctx = SavedContext {
        anvil_version: env!("CARGO_PKG_VERSION").to_string(),
        created: SavedContext::now(),
        platform: SavedContext::current_platform().to_string(),
        requests: packages.to_vec(),
        resolved: resolved
            .packages()
            .iter()
            .map(|p| ContextPackage {
                name: p.name.clone(),
                version: p.version.clone(),
            })
            .collect(),
        environment: env,
    };

    let path = std::path::Path::new(output);
    ctx.save(path)?;
    println!(
        "Saved context ({} packages) to {}",
        ctx.resolved.len(),
        output
    );

    Ok(())
}

/// Display the environment from a saved context file.
fn cmd_context_show(file: &str, json: bool, export: bool) -> Result<()> {
    let ctx = SavedContext::load(std::path::Path::new(file))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&ctx.environment)?);
    } else if export {
        for (key, value) in &ctx.environment {
            println!("export {}=\"{}\"", key, value);
        }
    } else {
        println!(
            "Context: {} packages, platform={}, anvil={}",
            ctx.resolved.len(),
            ctx.platform,
            ctx.anvil_version
        );
        println!("Packages:");
        for pkg in &ctx.resolved {
            println!("  {}-{}", pkg.name, pkg.version);
        }
        println!("Environment ({} variables):", ctx.environment.len());
        for (key, value) in &ctx.environment {
            println!("  {}={}", key, value);
        }
    }

    Ok(())
}

/// Run a command using a saved context's environment.
fn cmd_context_run(file: &str, command: &[String]) -> Result<()> {
    use std::process::Command;

    if command.is_empty() {
        anyhow::bail!("No command specified");
    }

    let ctx = SavedContext::load(std::path::Path::new(file))?;

    let status = Command::new(&command[0])
        .args(&command[1..])
        .envs(&ctx.environment)
        .status()?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Start a shell with a saved context's environment.
fn cmd_context_shell(config: &Config, file: &str, shell_override: Option<String>) -> Result<()> {
    let ctx = SavedContext::load(std::path::Path::new(file))?;

    let shell_path = shell_override
        .or_else(|| config.default_shell.clone())
        .unwrap_or_else(|| shell::detect_shell());

    shell::spawn_shell(&shell_path, &ctx.environment)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

/// Scaffold a new package definition.
fn cmd_init(name: &str, version: &str, flat: bool) -> Result<()> {
    let template = format!(
        r#"name: {name}
version: "{version}"
# description: one-line summary

# requires:
#   - python-3.11

environment:
  {env_key}: "${{PACKAGE_ROOT}}"
  # PATH: ${{PACKAGE_ROOT}}/bin:${{PATH}}

# commands:
#   {name}: ${{PACKAGE_ROOT}}/bin/{name}
"#,
        name = name,
        version = version,
        env_key = name.to_uppercase().replace('-', "_"),
    );

    if flat {
        let filename = format!("{}-{}.yaml", name, version);
        if std::path::Path::new(&filename).exists() {
            anyhow::bail!("{} already exists", filename);
        }
        std::fs::write(&filename, &template)?;
        println!("Created {}", filename);
    } else {
        let dir = format!("{}/{}", name, version);
        let pkg_path = format!("{}/package.yaml", dir);
        if std::path::Path::new(&pkg_path).exists() {
            anyhow::bail!("{} already exists", pkg_path);
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&pkg_path, &template)?;
        println!("Created {}", pkg_path);
    }

    Ok(())
}

/// Scaffold a global `~/.anvil.yaml` so first-time users have something to
/// edit instead of an empty file.
fn cmd_init_config() -> Result<()> {
    let path = Config::config_path();
    if path.exists() {
        anyhow::bail!("{} already exists", path.display());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let template = r#"# Anvil global config — see https://github.com/voidreamer/anvil

# Where to look for package definitions, in priority order.
# Each entry can be a directory of flat `<name>-<version>.yaml` files
# and/or nested `<name>/<version>/package.yaml` packages.
package_paths:
  - ~/packages
  # - /studio/packages
  # - ${STUDIO_ROOT}/packages

# Optional: package set aliases (use as `anvil run <alias-name> -- ...`).
# aliases:
#   maya-anim:
#     - maya-2024
#     - studio-tools

# Optional: shell that `anvil shell` uses by default.
# default_shell: zsh

# Optional: hide / restrict packages by glob.
# filters:
#   include: ["maya-*", "houdini-*"]
#   exclude: ["*-dev"]
"#;

    std::fs::write(&path, template)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    println!("Created {}", path.display());
    println!("Edit it to point `package_paths` at your package directory, then run `anvil list`.");

    Ok(())
}

// ---------------------------------------------------------------------------
// Wrap
// ---------------------------------------------------------------------------

/// Generate wrapper scripts for all commands defined by the resolved packages.
fn cmd_wrap(
    config: &Config,
    packages: &[String],
    dir: &str,
    wrapper_shell: &str,
    refresh: bool,
    frozen: bool,
) -> Result<()> {
    let resolver = build_resolver(config, refresh, frozen)?;
    let resolved = resolver.resolve(packages)?;
    let commands = resolved.commands();

    if commands.is_empty() {
        anyhow::bail!(
            "No commands defined in resolved packages. Add a `commands:` section to your package definitions."
        );
    }

    let dir_path = std::path::Path::new(dir);
    std::fs::create_dir_all(dir_path)?;

    // Build the package request string for the wrapper
    let pkg_args: Vec<String> = resolved.packages().iter().map(|p| p.id()).collect();
    let pkg_str = pkg_args.join(" ");

    let mut count = 0;
    for (alias, _target) in &commands {
        let script = generate_wrapper(wrapper_shell, &pkg_str, alias);
        let out_path = dir_path.join(alias);
        std::fs::write(&out_path, script)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&out_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&out_path, perms)?;
        }

        count += 1;
    }

    println!("Created {} wrapper(s) in {}", count, dir);
    for alias in commands.keys() {
        println!("  {}", alias);
    }

    Ok(())
}

fn generate_wrapper(shell: &str, packages: &str, command: &str) -> String {
    match shell {
        "fish" => format!(
            "#!/usr/bin/env fish\nexec anvil run {} -- {} $argv\n",
            packages, command
        ),
        "powershell" | "pwsh" => format!(
            "#!/usr/bin/env pwsh\nanvil run {} -- {} @args\n",
            packages, command
        ),
        _ => format!(
            "#!/usr/bin/env bash\nexec anvil run {} -- {} \"$@\"\n",
            packages, command
        ),
    }
}

// ---------------------------------------------------------------------------
// Publish
// ---------------------------------------------------------------------------

/// Publish a package to a target package path.
fn cmd_publish(target: &str, source: Option<&str>, flat: bool) -> Result<()> {
    use crate::package::Package;

    let source_dir = match source {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir()?,
    };

    // Load and validate the package
    let pkg = if source_dir.is_file() {
        Package::load_from_file(&source_dir, None)?
    } else {
        Package::load(&source_dir)?
    };

    let target_path = std::path::Path::new(target);
    if !target_path.exists() {
        anyhow::bail!("Target path does not exist: {}", target);
    }

    if flat {
        // Publish as flat YAML file
        let filename = format!("{}-{}.yaml", pkg.name, pkg.version);
        let dest = target_path.join(&filename);
        if dest.exists() {
            anyhow::bail!("{} already exists in target", filename);
        }

        // Re-read the source YAML to publish it verbatim
        let src_file = if source_dir.is_file() {
            source_dir.clone()
        } else {
            source_dir.join("package.yaml")
        };
        std::fs::copy(&src_file, &dest)?;
        println!("Published {}-{} to {}", pkg.name, pkg.version, dest.display());
    } else {
        // Publish as nested directory
        let dest_dir = target_path.join(&pkg.name).join(&pkg.version);
        if dest_dir.exists() {
            anyhow::bail!(
                "{}-{} already exists in target ({})",
                pkg.name,
                pkg.version,
                dest_dir.display()
            );
        }

        // Copy the entire source directory tree
        let src = if source_dir.is_file() {
            source_dir
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(source_dir.clone())
        } else {
            source_dir.clone()
        };

        copy_dir_recursive(&src, &dest_dir)?;
        println!(
            "Published {}-{} to {}",
            pkg.name,
            pkg.version,
            dest_dir.display()
        );
    }

    Ok(())
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
