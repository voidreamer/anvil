# Anvil

A fast, lightweight environment resolver for VFX and animation pipelines. Think Rez, but Rust-powered and simpler.

[![CI](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml)
[![Release](https://img.shields.io/github/v/release/voidreamer/anvil?include_prereleases)](https://github.com/voidreamer/anvil/releases)

## Features

- **YAML-based package definitions** -- simple, readable, version-controlled
- **Dependency resolution** -- automatic recursive resolution with version constraints (exact, minimum, range, alternatives)
- **Two package layouts** -- flat YAML files or nested `{name}/{version}/package.yaml` directories
- **Command aliases** -- packages define named commands; `anvil run` resolves them automatically
- **Environment variable expansion** -- `${VAR}`, `${PACKAGE_ROOT}`, `${VERSION}`, `${NAME}`, `~/` tilde expansion
- **Platform variants** -- per-platform requirements and environment overrides (Linux, macOS, Windows)
- **Aliases** -- named package sets for common configurations
- **Lockfiles** -- pin resolved versions for reproducible environments across machines
- **Saved contexts** -- export a fully resolved environment to JSON for render farms, CI, or sharing
- **Per-project config** -- `.anvil.yaml` in the project root, merged with user/studio config
- **Conflict detection** -- warns when packages silently override each other's variables
- **Pre/post hooks** -- run scripts before/after resolution or command execution (license checks, logging, etc.)
- **Package filters** -- include/exclude packages by glob pattern per-project or per-config
- **Shell completions** -- tab completion for bash, zsh, fish, PowerShell
- **Wrapper scripts** -- generate executable wrappers for resolved commands; add to `$PATH` for seamless tool access
- **Package publishing** -- `anvil publish` to copy validated packages to shared repositories
- **Scan caching** -- cached package scans with automatic invalidation for fast repeated calls
- **Cross-platform** -- Windows, Linux, macOS with shell-specific output (bash, zsh, fish, PowerShell, cmd)
- **Fast** -- written in Rust, resolves in milliseconds, single binary with no runtime dependencies

## Installation

```bash
cargo install anvil-env
```

Or build from source:

```bash
git clone https://github.com/voidreamer/anvil.git
cd anvil
cargo build --release
# Binary at target/release/anvil
```

## Quick Start

### 1. Create package definitions

Anvil supports two layouts. Use whichever fits your workflow -- both can coexist in the same directory.

**Flat files** (recommended for simplicity):

```bash
mkdir -p ~/packages

cat > ~/packages/maya-2024.yaml << 'EOF'
name: maya
version: "2024"
description: Autodesk Maya 2024

requires:
  - python-3.10

environment:
  MAYA_VERSION: "2024"
  MAYA_LOCATION: /usr/autodesk/maya2024
  PATH: ${MAYA_LOCATION}/bin:${PATH}
  PYTHONPATH: ${PACKAGE_ROOT}/scripts:${PYTHONPATH}

commands:
  maya: ${MAYA_LOCATION}/bin/maya
  mayapy: ${MAYA_LOCATION}/bin/mayapy
EOF
```

**Nested directories** (useful when packages bundle scripts, addons, or other files):

```
~/packages/
  maya/
    2024/
      package.yaml
      scripts/
      modules/
    2025/
      package.yaml
```

### 2. Configure anvil

Create `~/.anvil.yaml`:

```yaml
package_paths:
  - ~/packages
  - /studio/shared/packages

default_shell: bash

# Named package sets for common workflows
aliases:
  maya-anim:
    - maya-2024
    - animbot-2.0
    - studio-tools
```

### 3. Use it

```bash
# Show resolved environment variables
anvil env maya-2024

# Launch maya using the command alias defined in the package
anvil run maya-2024 -- maya

# Launch with multiple packages and arguments
anvil run maya-2024 arnold-7.2 -- maya -file scene.ma

# Start an interactive shell with packages loaded
anvil shell maya-2024 arnold-7.2

# Use an alias to resolve a whole group of packages
anvil run maya-anim -- maya
```

## Package Definition

A package is a YAML file that declares a name, version, and optionally: dependencies, environment variables, command aliases, and platform variants.

### Full schema

```yaml
name: houdini                          # Required: package name
version: "20.5"                        # Required: version string
description: SideFX Houdini 20.5       # Optional: human-readable description

requires:                              # Optional: dependencies (version-constrained)
  - python-3.11

environment:                           # Optional: environment variables to set
  HOUDINI_VERSION: "${VERSION}"
  HFS: ${PACKAGE_ROOT}
  PATH: ${HFS}/bin:${PATH}
  PYTHONPATH: ${HFS}/python/lib/python3.11/site-packages:${PYTHONPATH}

commands:                              # Optional: named command aliases
  houdini: ${HFS}/bin/houdini
  hython: ${HFS}/bin/hython
  hcustom: ${HFS}/bin/hcustom

variants:                              # Optional: platform-specific overrides
  - platform: linux
    environment:
      HFS: /opt/hfs20.5
      LD_LIBRARY_PATH: ${HFS}/dsolib:${LD_LIBRARY_PATH}
  - platform: macos
    environment:
      HFS: /Applications/Houdini/Houdini20.5/Frameworks/Houdini.framework/Versions/20.5/Resources
      DYLD_LIBRARY_PATH: ${HFS}/../Libraries:${DYLD_LIBRARY_PATH}
  - platform: windows
    environment:
      HFS: C:/Program Files/Side Effects Software/Houdini 20.5
```

### Minimal package

Only `name` and `version` are required:

```yaml
name: studio-tools
version: "1.0"
environment:
  PYTHONPATH: ${PACKAGE_ROOT}/python:${PYTHONPATH}
```

### Two package layouts

**Flat files** -- YAML files directly in a package path. The filename is for your convenience; the `name` and `version` inside the file are what anvil uses:

```
~/packages/
  maya-2024.yaml
  arnold-7.2.yaml
  python-3.11.yaml
```

**Nested directories** -- the traditional `{name}/{version}/package.yaml` layout. Better when a package includes associated files (scripts, addons, libraries):

```
~/packages/
  maya/
    2024/
      package.yaml
      scripts/
      modules/
  arnold/
    7.2/
      package.yaml
      bin/
      lib/
```

Both layouts can coexist in the same package path directory. Anvil scans `.yaml`/`.yml` files as flat packages and subdirectories as nested packages in a single pass.

### Version constraints

Used in the `requires` field and when requesting packages from the CLI:

| Format | Meaning |
|--------|---------|
| `maya-2024` | Exactly version 2024 |
| `maya-2024+` | Version 2024 or higher |
| `maya-2024..2025` | Versions 2024 through 2025 (inclusive) |
| `python-3.10\|3.11` | Version 3.10 or 3.11 |
| `maya` | Any available version (highest wins) |

When multiple versions match a constraint, the highest version is selected. Versions are compared as semantic versions when possible, falling back to string comparison.

Package names and versions are split on the last `-` in the request string, but only when the suffix starts with a digit. This means hyphenated names like `studio-blender-tools` work correctly -- anvil treats the whole string as the name and resolves any version. To request a specific version: `studio-blender-tools-1.0.0`.

### Environment variable expansion

Package environment values are expanded in this order:

1. `${PACKAGE_ROOT}` -- absolute path to the package directory
2. `${VERSION}` -- the package's version string
3. `${NAME}` -- the package's name
4. `${ANY_VAR}` -- any variable from previously resolved packages or the current environment
5. `~/` prefix -- expanded to the user's home directory

When multiple packages are resolved, their environments are merged in dependency order. Each package sees the environment from all previously resolved packages, so later packages can reference variables set by earlier ones.

**Conflict detection:** If two packages both set the same variable and the later one does not reference `${VAR}` (i.e., it overwrites rather than appends), anvil emits a warning. This catches accidental overrides while allowing common append patterns like `PATH: .../bin:${PATH}`.

### Command aliases

Packages can define named commands in the `commands` field:

```yaml
commands:
  houdini: ${HFS}/bin/houdini
  hython: ${HFS}/bin/hython
  kick: ${PACKAGE_ROOT}/bin/kick
```

When you use `anvil run`, the first argument after `--` is looked up in the merged command map of all resolved packages. If it matches a defined alias, it's replaced with the fully expanded path before execution:

```bash
# "houdini" is resolved to /opt/hfs20.5/bin/houdini (or platform equivalent)
anvil run houdini-20.5 -- houdini -scene myfile.hip

# Commands that don't match any alias pass through unchanged
anvil run houdini-20.5 -- /usr/bin/env hython myscript.py
```

Command values support the same variable expansion as environment values (`${PACKAGE_ROOT}`, `${VERSION}`, etc.), and are expanded against the fully resolved environment.

### Platform variants

Variants apply platform-specific overrides. The `requires` list is extended (merged with the base list), and `environment` values overwrite the base values for matching keys.

Supported platforms: `linux`, `windows`, `macos`.

```yaml
variants:
  - platform: linux
    requires:
      - gcc-11
    environment:
      LD_LIBRARY_PATH: ${PACKAGE_ROOT}/lib:${LD_LIBRARY_PATH}
  - platform: macos
    requires:
      - clang-14
  - platform: windows
    requires:
      - msvc-2022
    environment:
      PATH: ${PACKAGE_ROOT}/bin;${PATH}
```

## Commands

### `anvil env`

Resolve packages and print the resulting environment.

```bash
anvil env maya-2024 arnold-7.2       # KEY=VALUE format
anvil env maya-2024 --export          # Shell export statements
anvil env maya-2024 --json            # JSON object
```

Useful for debugging, piping into other tools, or generating env files for IDE integration.

### `anvil run`

Run a command with the resolved environment. If the command name matches a [command alias](#command-aliases) defined by any resolved package, it's automatically expanded to the full path.

```bash
# "maya" is resolved from the package's commands: field
anvil run maya-2024 -- maya

# Multiple packages, with arguments passed through
anvil run maya-2024 arnold-7.2 -- maya -file scene.ma

# Add extra environment variables with -e
anvil run maya-2024 -e MAYA_DEBUG=1 -e CUSTOM=value -- maya
```

Exits with the command's exit code.

### `anvil shell`

Start an interactive shell with packages loaded. Adds `[anvil]` to the prompt.

```bash
anvil shell maya-2024 arnold-7.2
anvil shell maya-2024 --shell zsh
```

Shell detection priority: `--shell` flag > `default_shell` from config > `$SHELL` > `bash`.

On Unix, the shell replaces the current process (`exec`). On Windows, it spawns a child process and waits.

### `anvil list`

List available packages or versions of a specific package.

```bash
anvil list              # All package names
anvil list maya         # All versions of maya
```

### `anvil info`

Show details for a specific package: name, version, description, dependencies, environment, and commands.

```bash
anvil info maya-2024
anvil info houdini-20.5
```

### `anvil validate`

Check that package definitions are valid and all dependencies can be resolved.

```bash
anvil validate              # Validate all packages
anvil validate maya-2024    # Validate one package
```

### `anvil lock`

Resolve packages and pin the exact versions to `anvil.lock`. Subsequent `anvil env`, `run`, and `shell` commands will prefer locked versions when the lockfile is present.

```bash
# Create a lockfile
anvil lock maya-2024 arnold-7.2

# Now any resolution will use pinned versions
anvil env maya-2024            # uses versions from anvil.lock
anvil run maya-2024 -- maya    # same

# Re-resolve and update the lockfile
anvil lock maya-2024 arnold-7.2 --update
```

The lockfile is a YAML file that can be committed to version control for reproducible environments across the team.

### `anvil context`

Save a fully resolved environment to a JSON file. The context can be loaded later to run commands or start shells without re-resolving -- useful for render farms, CI pipelines, or sharing exact environments.

```bash
# Save a context
anvil context save maya-2024 arnold-7.2 -o render.ctx.json

# Inspect it
anvil context show render.ctx.json
anvil context show render.ctx.json --json
anvil context show render.ctx.json --export

# Run a command using the saved environment
anvil context run render.ctx.json -- maya -batch -file scene.ma

# Start a shell with the saved environment
anvil context shell render.ctx.json
```

### `anvil init`

Scaffold a new package definition with a template.

```bash
# Create a nested package directory
anvil init my-tools                        # my-tools/1.0.0/package.yaml
anvil init my-tools --version 2.0          # my-tools/2.0/package.yaml

# Create a flat YAML file
anvil init my-tools --flat                 # my-tools-1.0.0.yaml
```

### `anvil completions`

Generate shell completions for tab completion.

```bash
# Bash (add to ~/.bashrc)
eval "$(anvil completions bash)"

# Zsh (add to ~/.zshrc)
eval "$(anvil completions zsh)"

# Fish
anvil completions fish | source

# PowerShell
anvil completions powershell | Out-String | Invoke-Expression
```

### `anvil wrap`

Generate executable wrapper scripts for all commands defined by the resolved packages. Each wrapper calls `anvil run` under the hood, so it always resolves correctly (respects lockfiles, config, etc.).

```bash
# Generate wrappers for all commands in houdini + its dependencies
anvil wrap houdini-20.5 --dir ~/tools/fx
# Creates: ~/tools/fx/houdini, ~/tools/fx/hython, ~/tools/fx/python, ...

# Add to PATH and use directly
export PATH="$HOME/tools/fx:$PATH"
houdini -scene myfile.hip
```

This is how you create **suites** -- generate wrappers for a department's tool set, add the directory to `$PATH`, and artists get seamless access to all tools without knowing about anvil.

### `anvil publish`

Copy a validated package to a shared package repository.

```bash
# Publish from a package directory (nested layout)
anvil publish /studio/packages --path ~/dev/my-tool

# Publish as a flat YAML file
anvil publish /studio/packages --path ~/dev/my-tool --flat

# Publish from current directory
cd ~/dev/my-tool
anvil publish /studio/packages
```

Refuses to overwrite existing packages. Validates the package before copying.

## Configuration

### Global config

Anvil looks for a global (user-level) configuration in this order:

1. `$ANVIL_CONFIG` environment variable (if set)
2. `~/.anvil.yaml`
3. `~/.config/anvil/config.yaml`

If no config file is found, anvil uses default package paths (see below).

### Per-project config

Anvil also searches for a `.anvil.yaml` file in the current directory and its ancestors. If found, the project config is merged with the global config:

- **`package_paths`** -- project paths are **prepended** (higher priority than global)
- **`aliases`** -- project aliases are added; same-name aliases override the global ones
- **`default_shell`** -- project value wins if set
- **`platform`** -- project platform paths are prepended per-platform

This allows each project/show to define its own package locations and aliases without modifying the user's global config:

```yaml
# /projects/myshow/.anvil.yaml
package_paths:
  - /projects/myshow/packages

aliases:
  show-tools:
    - maya-2024
    - myshow-assets-1.0
    - myshow-pipeline-2.3
```

### Full config example

```yaml
# Package search paths (supports ${VAR} expansion and ~/ tilde expansion)
package_paths:
  - ~/packages                     # Local/dev packages (highest priority)
  - /studio/packages               # Shared studio packages
  - ${STUDIO_ROOT}/packages        # Variable-based paths

# Default shell for 'anvil shell' (optional, defaults to $SHELL or bash)
default_shell: zsh

# Named package sets (optional)
# Use as: anvil run maya-anim -- maya
aliases:
  maya-anim:
    - maya-2024
    - animbot-2.0
    - studio-tools
  maya-light:
    - maya-2024
    - arnold-7.2
    - light-tools
  studio-blender:
    - blender-4.2
    - studio-blender-tools
    - studio-python

# Platform-specific additional package paths (optional)
# These extend the base package_paths list on matching platforms
platform:
  linux:
    package_paths:
      - /mnt/shared/packages
  windows:
    package_paths:
      - P:/packages
  macos:
    package_paths:
      - /Volumes/shared/packages

# Lifecycle hooks (optional)
# Shell commands run at specific points during resolution/execution
hooks:
  pre_resolve:
    - echo "Resolving packages..."
  post_resolve:
    - /studio/scripts/log_resolution.sh
  pre_run:
    - /studio/scripts/check_license.sh
  post_run:
    - /studio/scripts/cleanup.sh

# Package filters (optional)
# When include is set, only matching packages are visible
# Exclude is applied after include. Patterns use glob syntax (*, ?)
filters:
  include:
    - "maya-*"
    - "arnold-*"
    - "studio-*"
  exclude:
    - "*-dev"
    - "test-*"
```

### Hooks

Hooks are shell commands that run at specific lifecycle points:

| Hook | When | Fails on error? |
|------|------|-----------------|
| `pre_resolve` | Before package resolution | Yes |
| `post_resolve` | After resolution, with resolved env | Yes |
| `pre_run` | Before command execution in `anvil run` | Yes |
| `post_run` | After command finishes in `anvil run` | No (best-effort) |

If a pre-hook exits non-zero, the operation is aborted. Hooks receive the resolved environment as their environment.

### Package filters

Filters control which packages are visible. Useful for restricting a project to only approved packages:

```yaml
# Only show Maya and Arnold packages, but hide dev versions
filters:
  include: ["maya-*", "arnold-*"]
  exclude: ["*-dev"]
```

When `include` is non-empty, only matching packages pass. `exclude` is applied after `include`. Patterns support `*` (any chars) and `?` (single char).

### Environment variables

| Variable | Purpose |
|----------|---------|
| `ANVIL_CONFIG` | Override config file location |
| `ANVIL_PACKAGES` | Additional package paths (colon-separated) |
| `RUST_LOG` | Control log verbosity (e.g., `RUST_LOG=debug anvil env maya`) |

### Default package paths

If no config file is found, anvil searches these directories for packages:

- Paths from `$ANVIL_PACKAGES` (colon-separated)
- `$HOME/packages`
- `$HOME/.local/share/anvil/packages`
- `/opt/packages`

## Anvil vs Rez

Anvil is designed as a practical alternative to [Rez](https://github.com/AcademySoftwareFoundation/rez) for studios that need fast, reliable environment resolution without the operational overhead.

|  | Anvil | Rez |
|---|---|---|
| **Language** | Rust (single static binary) | Python |
| **Startup time** | Milliseconds | Seconds (Python bootstrap + imports) |
| **Package format** | YAML | Python (`package.py`) |
| **Runtime dependencies** | None | Python 3.7+, pip, platform bindings |
| **Resolution strategy** | Greedy (highest matching version) | SAT solver with backtracking |
| **Installation** | `cargo install anvil-env` or download binary | pip install + `rez bind` + config |
| **Learning curve** | 6 commands, YAML only | Many subsystems, Python API |
| **Config surface** | 1 YAML file, 3 env vars | Multiple config files, dozens of settings |

### Where Anvil shines

- **Zero bootstrap** -- no Python runtime, no virtual environments, no `rez bind`, no platform bindings. Copy the binary and go.
- **Instant startup** -- sub-millisecond resolution means no lag when launching tools or shells. Artists don't wait.
- **Simple packages** -- YAML files that any TD can write, review in PRs, and store alongside code in version control.
- **Low ops burden** -- a single binary and a directory of YAML files. No database, no daemon, no package server required.
- **Flat-file packages** -- for simple packages (wrappers, environment configs), a single YAML file is enough. No directory hierarchy needed.
- **Cross-platform first** -- native Windows, Linux, macOS support with per-platform variants and shell-specific output (bash, zsh, fish, PowerShell, cmd).

### Where Rez has the edge (for now)

- **SAT solver** -- handles complex constraint satisfaction that greedy resolution cannot (important for large dependency graphs with conflicts).
- **Build system** -- `rez-build` / `rez-release` for building compiled packages with cmake/make integration.
- **Package repository** -- centralized package server with memcached integration for cached resolution.

### Who should use Anvil

Anvil is built for studios and teams where Rez's complexity isn't justified by the scale of the dependency graph. If your packages number in the dozens (not thousands), if your version constraints are straightforward, and if you value fast iteration and simple ops over advanced constraint solving -- Anvil is the right tool.

This includes solo TDs, small studios, and medium studios that want to ship a working pipeline without dedicating engineering time to maintaining a package management system.

## Roadmap

Planned features, roughly in priority order. Contributions welcome.

### Near term

- [x] **`anvil context`** -- save and restore resolved environments to a file (like `rez-env --output`)
- [x] **Lockfiles** -- pin resolved versions for reproducible environments across machines and CI
- [x] **Per-project config** -- `.anvil.yaml` in the project root, merged with user/studio config
- [x] **Conflict warnings** -- detect and warn when multiple packages set the same environment variable
- [x] **`anvil init`** -- scaffold a new package definition from a template
- [x] **Resolution caching** -- cache filesystem scan results to skip re-traversal on repeated calls
- [x] **Pre/post hooks** -- run scripts before or after resolution, shell entry, or command execution
- [x] **Package filters** -- include or exclude packages by pattern, label, or path
- [x] **Shell completions** -- tab completion for bash, zsh, fish, PowerShell

### Medium term

- [x] **`anvil publish`** -- publish validated packages to shared package repositories
- [x] **`anvil wrap`** -- generate wrapper scripts for resolved commands; create tool suites for departments
- [ ] **Remote package sources** -- fetch packages from HTTP, S3, or GCS endpoints

### Longer term

- [ ] **Backtracking resolver** -- upgrade to a solver that backtracks on conflicts for complex dependency graphs
- [ ] **Package server** -- lightweight HTTP service for centralized package hosting and discovery
- [ ] **Standalone wrappers** -- wrapper scripts that embed the environment (no anvil needed at runtime)
- [ ] **Web dashboard** -- visibility into available packages, resolution results, and usage across the studio
- [ ] **Audit logging** -- track who resolved what, when, for compliance and debugging

## Development

```bash
cargo build --release       # Optimized binary (LTO, stripped)
cargo test                  # Run tests
cargo fmt                   # Format code
cargo clippy                # Lint

# Debug logging
RUST_LOG=debug cargo run -- env maya-2024
```

## License

MIT
