//! Anvil - Environment resolver for VFX pipelines
//!
//! A fast, lightweight alternative to Rez for managing DCC environments.

use anyhow::{Context, Result};
use clap::Parser;
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
use context::{ContextPackage, Lockfile, SavedContext};
use resolver::Resolver;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "anvil=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let cli = Cli::parse();

    // Load config
    let config = Config::load()?;
    let refresh = cli.refresh;

    match cli.command {
        Commands::Env { packages, export, json } => {
            cmd_env(&config, &packages, export, json, refresh)?;
        }
        Commands::Run { packages, env_vars, command } => {
            cmd_run(&config, &packages, &env_vars, &command, refresh)?;
        }
        Commands::Shell { packages, shell } => {
            cmd_shell(&config, &packages, shell, refresh)?;
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
        Commands::Lock { packages, update: _ } => {
            cmd_lock(&config, &packages, refresh)?;
        }
        Commands::Context { action } => match action {
            ContextAction::Save { packages, output } => {
                cmd_context_save(&config, &packages, &output, refresh)?;
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
        Commands::Init { name, version, flat } => {
            cmd_init(&name, &version, flat)?;
        }
        Commands::Completions { shell } => {
            Cli::print_completions(shell);
        }
        Commands::Wrap { packages, dir, shell } => {
            cmd_wrap(&config, &packages, &dir, &shell, refresh)?;
        }
        Commands::Publish { target, path, flat } => {
            cmd_publish(&target, path.as_deref(), flat)?;
        }
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
) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;
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
) -> Result<()> {
    use std::process::Command;

    // Pre-resolve hooks
    Config::run_hooks(&config.hooks.pre_resolve, &std::env::vars().collect())?;

    let resolver = Resolver::new(config, refresh)?;
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
) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;
    let resolved = resolver.resolve(packages)?;
    let env = resolved.environment();

    let shell_path = shell
        .or_else(|| config.default_shell.clone())
        .unwrap_or_else(|| shell::detect_shell());

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
// Lock
// ---------------------------------------------------------------------------

/// Resolve packages and write pinned versions to `anvil.lock`.
fn cmd_lock(config: &Config, packages: &[String], refresh: bool) -> Result<()> {
    // Always resolve fresh (ignore existing lockfile).
    let resolver = Resolver::new_unlocked(config, refresh)?;
    let resolved = resolver.resolve(packages)?;

    let mut pins = std::collections::HashMap::new();
    for pkg in resolved.packages() {
        pins.insert(pkg.name.clone(), pkg.version.clone());
    }

    let lockfile = Lockfile {
        requests: packages.to_vec(),
        pins,
    };

    let lock_path = std::path::PathBuf::from("anvil.lock");
    lockfile.save(&lock_path)?;

    println!("Locked {} packages to anvil.lock:", resolved.packages().len());
    for pkg in resolved.packages() {
        println!("  {}-{}", pkg.name, pkg.version);
    }

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
) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;
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
) -> Result<()> {
    let resolver = Resolver::new(config, refresh)?;
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
