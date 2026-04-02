# Anvil

A fast, lightweight environment resolver for VFX and animation pipelines. Think Rez, but Rust-powered and simpler.

[![CI](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml)
[![Release](https://img.shields.io/github/v/release/voidreamer/anvil?include_prereleases)](https://github.com/voidreamer/anvil/releases)

## Features

- **YAML-based package definitions** -- simple, readable, version-controlled
- **Dependency resolution** -- automatic resolution with version constraints (exact, minimum, range, alternatives)
- **Two package layouts** -- flat YAML files or nested `{name}/{version}/package.yaml` directories
- **Environment variable expansion** -- `${VAR}`, `${PACKAGE_ROOT}`, `${VERSION}`, `~/` tilde expansion
- **Platform variants** -- per-platform requirements and environment overrides
- **Aliases** -- named package sets for common configurations
- **Cross-platform** -- Windows, Linux, macOS
- **Fast** -- written in Rust, resolves in milliseconds, single binary with no runtime dependencies

## Installation

```bash
cargo install anvil-pckm
```

## Quick Start

### 1. Create package definitions

Anvil supports two layouts. Use whichever you prefer -- both can coexist in the same directory.

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
  PATH: /usr/autodesk/maya2024/bin:${PATH}
  PYTHONPATH: ${PACKAGE_ROOT}/scripts:${PYTHONPATH}
EOF
```

**Nested directories** (useful when packages have associated files):

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
  - /shared/studio/packages

default_shell: bash
```

### 3. Use it

```bash
# Show resolved environment variables
anvil env maya-2024

# Launch a command with resolved environment
anvil run maya-2024 arnold-7.2 -- maya -file scene.ma

# Start an interactive shell with packages loaded
anvil shell maya-2024 arnold-7.2
```

## Package Definition

```yaml
name: arnold
version: "7.2"
description: Arnold renderer for Maya

requires:
  - maya-2024+

environment:
  ARNOLD_VERSION: ${VERSION}
  MTOA_PATH: ${PACKAGE_ROOT}
  MAYA_RENDER_DESC_PATH: ${PACKAGE_ROOT}/renderDesc
  PATH: ${PACKAGE_ROOT}/bin:${PATH}

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
```

### Version Constraints

Used in the `requires` field and when requesting packages from the CLI:

| Format | Meaning |
|--------|---------|
| `maya-2024` | Exactly version 2024 |
| `maya-2024+` | Version 2024 or higher |
| `maya-2024..2025` | Versions 2024 through 2025 (inclusive) |
| `python-3.10\|3.11` | Version 3.10 or 3.11 |
| `maya` | Any available version (highest wins) |

When multiple versions match a constraint, the highest version is selected. Versions are compared as semantic versions when possible, falling back to string comparison.

Note: package names and versions are split on the last `-` in the request string. Avoid package names that end with something that looks like a version (e.g., prefer `mypackage` over `my-package`).

### Environment Variable Expansion

Package environment values are expanded in this order:

1. `${PACKAGE_ROOT}` -- absolute path to the package directory
2. `${VERSION}` -- the package's version string
3. `${NAME}` -- the package's name
4. `${ANY_VAR}` -- any variable from previously resolved packages or the current environment
5. `~/` prefix -- expanded to the user's home directory

When multiple packages are resolved, their environments are merged in dependency order. Each package sees the environment from all previously resolved packages.

### Platform Variants

Variants apply platform-specific overrides. The `requires` list is extended (merged with base), and `environment` values overwrite the base values for matching keys.

Supported platforms: `linux`, `windows`, `macos`.

## Commands

### `anvil env`

Resolve packages and print the resulting environment.

```bash
anvil env maya-2024 arnold-7.2       # KEY=VALUE format
anvil env maya-2024 --export          # Shell export statements
anvil env maya-2024 --json            # JSON object
```

### `anvil run`

Run a command with the resolved environment.

```bash
anvil run maya-2024 -- maya
anvil run maya-2024 arnold-7.2 -- maya -file scene.ma
anvil run maya-2024 -e MAYA_DEBUG=1 -e CUSTOM=value -- maya
```

### `anvil shell`

Start an interactive shell with packages loaded. Adds `[anvil]` to the prompt.

```bash
anvil shell maya-2024 arnold-7.2
anvil shell maya-2024 --shell zsh
```

Shell detection priority: `--shell` flag, then `default_shell` from config, then `$SHELL`, then `bash`.

### `anvil list`

List available packages or versions of a specific package.

```bash
anvil list              # All package names
anvil list maya         # All versions of maya
```

### `anvil info`

Show details for a specific package: name, version, description, dependencies, and environment.

```bash
anvil info maya-2024
```

### `anvil validate`

Check that package definitions are valid and all dependencies exist.

```bash
anvil validate              # Validate all packages
anvil validate maya-2024    # Validate one package
```

## Configuration

Anvil looks for configuration in this order:

1. `$ANVIL_CONFIG` environment variable (if set)
2. `~/.anvil.yaml`
3. `~/.config/anvil/config.yaml`

```yaml
# Package search paths (required)
# Supports ${VAR} expansion and ~/ tilde expansion
package_paths:
  - ~/packages
  - /studio/packages
  - ${STUDIO_ROOT}/packages

# Default shell for 'anvil shell' (optional, defaults to $SHELL or bash)
default_shell: zsh

# Platform-specific additional package paths (optional)
# These extend the base package_paths list
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

# Named package sets (optional)
# Use as: anvil run maya-anim -- maya
aliases:
  maya-anim: [maya-2024, animbot-2.0, studio-tools]
  maya-light: [maya-2024, arnold-7.2, light-tools]
```

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `ANVIL_CONFIG` | Override config file location |
| `ANVIL_PACKAGES` | Additional package paths (colon-separated) |

### Default Package Paths

If no config file is found, anvil searches these directories for packages:

- `$HOME/packages`
- `$HOME/.local/share/anvil/packages`
- `/opt/packages`

## Development

```bash
cargo build --release
cargo test
RUST_LOG=debug cargo run -- env maya-2024
cargo fmt
cargo clippy
```

## Why not Rez?

Rez is excellent for large studios, but anvil targets a different niche:

- **Single binary** -- no Python runtime dependency, no bootstrapping
- **Millisecond resolution** -- Rust performance for fast shell startup
- **YAML packages** -- simpler than Rez's Python-based package definitions
- **Minimal footprint** -- fewer concepts to learn, easier to maintain

For smaller studios, personal pipelines, or environments where Rez is more complexity than you need.

## License

MIT
