# 🔨 Anvil

Forge your environment — A fast, lightweight package manager for VFX/Animation pipelines. 

Think Rez, but Rust-powered and simpler.

[![CI](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml)
[![Release](https://img.shields.io/github/v/release/voidreamer/anvil?include_prereleases)](https://github.com/voidreamer/anvil/releases)

## Features

- **YAML-based package definitions** — Simple, readable, version-controlled
- **Environment resolution** — Automatic dependency resolution with version constraints
- **DCC launcher** — Launch Maya, Houdini, Blender, Nuke with correct environment
- **Fast** — Written in Rust, resolves in milliseconds
- **Cross-platform** — Windows, Linux, macOS

## Installation

### From releases

```bash
# Linux/macOS
curl -sSL https://github.com/voidreamer/anvil/releases/latest/download/anvil-linux-x64 -o anvil
chmod +x anvil
sudo mv anvil /usr/local/bin/

# Or with cargo
cargo install anvil
```

### From source

```bash
git clone https://github.com/voidreamer/anvil.git
cd anvil
cargo install --path .
```

## Quick Start

### 1. Create a package repository

```bash
mkdir ~/pipeline-packages
cd ~/pipeline-packages

# Create a package definition
mkdir -p maya/2024
cat > maya/2024/package.yaml << 'EOF'
name: maya
version: "2024"
description: Autodesk Maya 2024

requires:
  - python-3.10

environment:
  MAYA_VERSION: "2024"
  PATH: /usr/autodesk/maya2024/bin:${PATH}
  PYTHONPATH: ${PACKAGE_ROOT}/scripts:${PYTHONPATH}

commands:
  maya: maya
  mayapy: mayapy
EOF
```

### 2. Configure anvil

```bash
cat > ~/.anvil.yaml << 'EOF'
package_paths:
  - ~/pipeline-packages
  - /shared/packages

default_shell: bash
EOF
```

### 3. Use it

```bash
# Resolve and print environment
anvil env maya-2024

# Launch Maya with resolved environment  
anvil run maya-2024 -- maya

# Launch with multiple packages
anvil run maya-2024 arnold-7.2 studio-tools -- maya

# Interactive shell with packages
anvil shell maya-2024 arnold-7.2
```

## Package Definition

Packages are defined in YAML:

```yaml
# ~/packages/arnold/7.2/package.yaml
name: arnold
version: "7.2"
description: Arnold renderer for Maya

requires:
  - maya-2024+  # Maya 2024 or higher
  
variants:
  - platform: linux
    requires:
      - gcc-11
  - platform: windows
    requires:
      - msvc-2022

environment:
  ARNOLD_VERSION: ${VERSION}
  MTOA_PATH: ${PACKAGE_ROOT}
  MAYA_RENDER_DESC_PATH: ${PACKAGE_ROOT}/renderDesc
  
commands:
  kick: ${PACKAGE_ROOT}/bin/kick
```

### Version Constraints

```yaml
requires:
  - maya-2024        # Exactly 2024
  - maya-2024+       # 2024 or higher
  - maya-2024..2025  # 2024 to 2025 (inclusive)
  - python-3.10|3.11 # 3.10 or 3.11
```

## Commands

### `anvil env`

Resolve packages and print environment variables.

```bash
# Print resolved environment
anvil env maya-2024 arnold-7.2

# Export format for shell
anvil env maya-2024 --export

# JSON output
anvil env maya-2024 --json
```

### `anvil run`

Run a command with resolved environment.

```bash
# Run Maya
anvil run maya-2024 -- maya -file scene.ma

# Run with specific packages
anvil run maya-2024 arnold-7.2 yeti-4.0 -- maya

# Pass environment variables
anvil run maya-2024 -e MAYA_DEBUG=1 -- maya
```

### `anvil shell`

Start an interactive shell with resolved environment.

```bash
# Bash shell with packages
anvil shell maya-2024 arnold-7.2

# Specific shell
anvil shell maya-2024 --shell zsh
```

### `anvil list`

List available packages.

```bash
# List all packages
anvil list

# List versions of a package
anvil list maya

# Show package details
anvil info maya-2024
```

### `anvil validate`

Validate package definitions.

```bash
# Validate all packages
anvil validate

# Validate specific package
anvil validate maya-2024
```

## Configuration

Global config at `~/.anvil.yaml`:

```yaml
# Package search paths
package_paths:
  - ~/packages
  - /studio/packages
  - ${STUDIO_ROOT}/packages

# Default shell for `anvil shell`
default_shell: bash

# Platform overrides
platform:
  linux:
    package_paths:
      - /mnt/packages
  windows:
    package_paths:
      - P:/packages

# Aliases for common package sets
aliases:
  maya-anim: [maya-2024, animbot-2.0, studio-tools]
  maya-light: [maya-2024, arnold-7.2, light-tools]
```

## Documentation

- [Blender Setup Guide](docs/blender-setup.md) — Using anvil with Blender and custom addons
- [Examples](examples/) — Sample packages and configuration

## Development

```bash
# Build
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run -- env maya-2024

# Format
cargo fmt

# Lint
cargo clippy
```

## Why not Rez?

Rez is great, but:

1. **Complexity** — Rez has many features we don't need
2. **Python dependency** — Rez requires Python; this is a single binary
3. **Speed** — Rust is faster for environment resolution
4. **Simplicity** — YAML packages are easier than Python packages

Pipeline-config is designed for smaller studios or simpler setups where Rez is overkill.

## License

MIT © Alejandro Cabrera
