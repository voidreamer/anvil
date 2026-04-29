//! CLI argument definitions

use clap::{CommandFactory, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "anvil")]
#[command(author = "Alejandro Cabrera <voidreamer@gmail.com>")]
#[command(version)]
#[command(about = "Forge your environment 🔨 — Fast package resolution for VFX pipelines", long_about = None)]
pub struct Cli {
    /// Ignore any cached package scan and re-read all package files.
    #[arg(long, global = true)]
    pub refresh: bool,

    /// Increase log verbosity: `-v` enables info, `-vv` enables debug.
    /// `RUST_LOG` overrides this when set.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Verify that anvil.lock is up to date.  Re-resolves the locked
    /// request set fresh and compares against the pins on disk; any
    /// drift (different version, different content hash, missing or
    /// extra package) fails the command.  Useful in CI.
    #[arg(long, global = true, conflicts_with = "frozen")]
    pub locked: bool,

    /// Use anvil.lock verbatim and never fall back to fresh
    /// resolution.  Any package the resolver would otherwise pick
    /// from the package paths must already be pinned, otherwise the
    /// command fails.  Useful for render farms and other non-mutating
    /// runs that must never silently drift.
    #[arg(long, global = true, conflicts_with = "locked")]
    pub frozen: bool,

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

        /// Don't materialise `commands:` as PATH shims — compose the env only.
        #[arg(long)]
        env_only: bool,

        /// Skip the orphan-shim sweep on entry (debugging aid).
        #[arg(long)]
        no_sweep: bool,
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

        /// Treat command-target warnings (missing / non-executable files)
        /// as validation failures.
        #[arg(long)]
        strict: bool,
    },

    /// Pin resolved versions to a lockfile for reproducible environments
    Lock {
        /// Packages to resolve and pin
        #[arg(required = true)]
        packages: Vec<String>,

        /// Re-resolve even if anvil.lock already exists
        #[arg(long)]
        update: bool,

        /// Resolve for every supported platform (linux, macos, windows)
        /// and union the results, so a single lockfile is correct on
        /// any of them.  Variant-specific `requires:` are recorded under
        /// the relevant platform.
        #[arg(long)]
        all_platforms: bool,
    },

    /// Save and restore complete resolved environments
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },

    /// Scaffold a new package definition (or `--config` for the global config)
    Init {
        /// Package name (e.g., my-tools). Omit when using `--config`.
        name: Option<String>,

        /// Package version (default: 1.0.0)
        #[arg(long, default_value = "1.0.0")]
        version: String,

        /// Create as a flat YAML file instead of a nested directory
        #[arg(long, conflicts_with = "config")]
        flat: bool,

        /// Scaffold a global `~/.anvil.yaml` instead of a package
        #[arg(long, conflicts_with_all = ["flat", "name"])]
        config: bool,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate wrapper scripts for resolved package commands
    Wrap {
        /// Packages to resolve
        #[arg(required = true)]
        packages: Vec<String>,

        /// Output directory for wrapper scripts
        #[arg(short, long, default_value = ".")]
        dir: String,

        /// Shell for wrapper scripts (bash, zsh, fish, powershell)
        #[arg(long, default_value = "bash")]
        shell: String,
    },

    /// Publish a package to a target package path
    Publish {
        /// Target package path to publish to
        target: String,

        /// Source package directory (default: current directory)
        #[arg(short, long)]
        path: Option<String>,

        /// Publish as a flat YAML file instead of a nested directory
        #[arg(long)]
        flat: bool,
    },
}

impl Cli {
    /// Generate shell completions and write to stdout.
    pub fn print_completions(shell: clap_complete::Shell) {
        clap_complete::generate(
            shell,
            &mut Self::command(),
            "anvil",
            &mut std::io::stdout(),
        );
    }
}

#[derive(Subcommand)]
pub enum ContextAction {
    /// Resolve packages and save the full environment to a context file
    Save {
        /// Packages to resolve
        #[arg(required = true)]
        packages: Vec<String>,

        /// Output file path
        #[arg(short, long, default_value = "context.json")]
        output: String,
    },

    /// Display the environment from a saved context
    Show {
        /// Context file to load
        file: String,

        /// Output as JSON
        #[arg(short, long)]
        json: bool,

        /// Output as shell export statements
        #[arg(short, long)]
        export: bool,
    },

    /// Run a command using a saved context's environment
    Run {
        /// Context file to load
        file: String,

        /// Command to run (after --)
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// Start a shell with a saved context's environment
    Shell {
        /// Context file to load
        file: String,

        /// Shell to use
        #[arg(short, long)]
        shell: Option<String>,
    },
}
