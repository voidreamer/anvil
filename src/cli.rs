//! CLI argument definitions

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pconfig")]
#[command(author = "Alejandro Cabrera <voidreamer@gmail.com>")]
#[command(version)]
#[command(about = "Pipeline environment and configuration manager", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Resolve packages and print environment variables
    Env {
        /// Packages to resolve (e.g., maya-2024 arnold-7.2)
        #[arg(required = true)]
        packages: Vec<String>,
        
        /// Output as shell export statements
        #[arg(short, long)]
        export: bool,
        
        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },
    
    /// Run a command with resolved environment
    Run {
        /// Packages to resolve
        #[arg(required = true)]
        packages: Vec<String>,
        
        /// Additional environment variables (KEY=VALUE)
        #[arg(short, long = "env")]
        env_vars: Vec<String>,
        
        /// Command to run (after --)
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    
    /// Start an interactive shell with resolved environment
    Shell {
        /// Packages to resolve
        #[arg(required = true)]
        packages: Vec<String>,
        
        /// Shell to use (defaults to $SHELL or bash)
        #[arg(short, long)]
        shell: Option<String>,
    },
    
    /// List available packages
    List {
        /// Package name to list versions of (optional)
        package: Option<String>,
    },
    
    /// Show detailed package information
    Info {
        /// Package name (e.g., maya-2024)
        package: String,
    },
    
    /// Validate package definitions
    Validate {
        /// Package to validate (optional, validates all if not specified)
        package: Option<String>,
    },
}
