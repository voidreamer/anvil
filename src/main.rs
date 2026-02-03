//! Pipeline Config - Environment manager for VFX pipelines
//!
//! A lightweight alternative to Rez for managing DCC environments.

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod cli;
mod config;
mod package;
mod resolver;
mod shell;

use cli::{Cli, Commands};
use config::Config;
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
    
    match cli.command {
        Commands::Env { packages, export, json } => {
            cmd_env(&config, &packages, export, json)?;
        }
        Commands::Run { packages, env_vars, command } => {
            cmd_run(&config, &packages, &env_vars, &command)?;
        }
        Commands::Shell { packages, shell } => {
            cmd_shell(&config, &packages, shell)?;
        }
        Commands::List { package } => {
            cmd_list(&config, package)?;
        }
        Commands::Info { package } => {
            cmd_info(&config, &package)?;
        }
        Commands::Validate { package } => {
            cmd_validate(&config, package)?;
        }
    }
    
    Ok(())
}

/// Resolve packages and print environment
fn cmd_env(config: &Config, packages: &[String], export: bool, json: bool) -> Result<()> {
    let resolver = Resolver::new(config)?;
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
) -> Result<()> {
    use std::process::Command;
    
    let resolver = Resolver::new(config)?;
    let resolved = resolver.resolve(packages)?;
    let mut env = resolved.environment();
    
    // Add user-specified env vars
    for var in env_vars {
        if let Some((key, value)) = var.split_once('=') {
            env.insert(key.to_string(), value.to_string());
        }
    }
    
    if command.is_empty() {
        anyhow::bail!("No command specified");
    }
    
    let status = Command::new(&command[0])
        .args(&command[1..])
        .envs(&env)
        .status()?;
    
    std::process::exit(status.code().unwrap_or(1));
}

/// Start interactive shell with resolved environment
fn cmd_shell(config: &Config, packages: &[String], shell: Option<String>) -> Result<()> {
    let resolver = Resolver::new(config)?;
    let resolved = resolver.resolve(packages)?;
    let env = resolved.environment();
    
    let shell_path = shell
        .or_else(|| config.default_shell.clone())
        .unwrap_or_else(|| shell::detect_shell());
    
    shell::spawn_shell(&shell_path, &env)?;
    
    Ok(())
}

/// List available packages
fn cmd_list(config: &Config, package: Option<String>) -> Result<()> {
    let resolver = Resolver::new(config)?;
    
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
fn cmd_info(config: &Config, package: &str) -> Result<()> {
    let resolver = Resolver::new(config)?;
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
    
    Ok(())
}

/// Validate package definitions
fn cmd_validate(config: &Config, package: Option<String>) -> Result<()> {
    let resolver = Resolver::new(config)?;
    
    let packages = if let Some(name) = package {
        vec![name]
    } else {
        resolver.list_packages()?
    };
    
    let mut errors = 0;
    
    for pkg_name in packages {
        match resolver.validate_package(&pkg_name) {
            Ok(()) => println!("✓ {}", pkg_name),
            Err(e) => {
                println!("✗ {}: {}", pkg_name, e);
                errors += 1;
            }
        }
    }
    
    if errors > 0 {
        anyhow::bail!("{} package(s) failed validation", errors);
    }
    
    println!("\nAll packages valid!");
    Ok(())
}
