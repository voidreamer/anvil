# Anvil Feature Walkthrough

A hands-on tour of every anvil feature, using the example packages shipped with this repo.

## Setup

```bash
# Build anvil
cargo build --release
export PATH="$PWD/target/release:$PATH"

# Point anvil at the example packages
export ANVIL_CONFIG="$PWD/examples/anvil.yaml"
```

All commands below assume you're in the repo root.

---

## 1. Basics

### List packages

```bash
anvil list
# blender
# houdini
# nuke
# python
# studio-blender-tools
# studio-python
# usd
```

### List versions of a package

```bash
anvil list blender
# blender:
#   - 4.2
#   - 4.3
```

### Show package info

```bash
anvil info houdini-20.5
# Name: houdini
# Version: 20.5
# Description: SideFX Houdini 20.5
# Requires:
#   - python-3.11
# Environment:
#   ...
# Commands:
#   houdini: houdini
#   hython: hython
#   ...
```

### Validate packages

```bash
anvil validate
# ✓ blender
# ✓ houdini
# ✓ nuke
# ✓ python
# ✓ studio-blender-tools
# ✓ studio-python
# ✓ usd
# All packages valid!
```

---

## 2. Environment resolution

### Print resolved environment

```bash
# KEY=VALUE format
anvil env python-3.11 | grep PYTHON
# PYTHON_VERSION=3.11

# Shell export format (source-able)
anvil env python-3.11 --export | grep PYTHON
# export PYTHON_VERSION="3.11"

# JSON format (for scripts)
anvil env python-3.11 --json | python3 -c "import sys,json; print(json.load(sys.stdin)['PYTHON_VERSION'])"
# 3.11
```

### Dependency resolution

Houdini depends on python-3.11 — both are resolved automatically:

```bash
anvil env houdini-20.5 | grep -E "PYTHON_VERSION|HOUDINI_VERSION"
# PYTHON_VERSION=3.11
# HOUDINI_VERSION=20.5
```

### Version constraints

```bash
# Exact version
anvil info blender-4.2

# Minimum version (resolves to 4.3, the highest available)
anvil info blender-4.2+

# Any version (also 4.3)
anvil info blender
```

### Aliases

Aliases are defined in the config and expand to a list of packages:

```bash
# studio-houdini = [houdini-20.5, studio-python]
anvil env studio-houdini | grep -E "HOUDINI_VERSION|STUDIO_ROOT"
# HOUDINI_VERSION=20.5
# STUDIO_ROOT=...
```

### Conflict detection

When two packages set the same variable without appending, anvil warns:

```bash
RUST_LOG=anvil=warn anvil env studio-blender-tools-1.0.0 2>&1 | grep -i override
# WARN studio-blender-tools-1.0.0 overrides PYTHONPATH (previously set by studio-python-1.0.0)
```

---

## 3. Running commands

### Run with resolved environment

```bash
# The command after -- runs with the resolved environment
anvil run python-3.11 -- echo "hello from anvil"
```

### Command alias resolution

Packages define named commands. `anvil run` resolves them automatically:

```bash
# "python" is looked up in the commands: field of python-3.11
anvil run python-3.11 -- python --version

# Houdini's package defines: houdini, hython, hcustom, mantra
anvil info houdini-20.5 | grep Commands -A5
```

### Extra environment variables

```bash
anvil run python-3.11 -e MY_VAR=hello -- env | grep MY_VAR
# MY_VAR=hello
```

---

## 4. Interactive shell

```bash
# Start a shell with packages loaded
# (will replace the current process — type 'exit' to return)
anvil shell python-3.11

# Specify a different shell
anvil shell python-3.11 --shell /bin/bash
```

---

## 5. Package layouts

### Flat files (single YAML)

```bash
# nuke and usd are flat files
ls examples/packages/nuke-15.1.yaml examples/packages/usd-24.08.yaml

anvil info nuke-15.1
anvil info usd-24.08
```

### Nested directories (with bundled files)

```bash
# blender, houdini, python are nested
ls examples/packages/blender/4.2/package.yaml
ls examples/packages/studio-blender-tools/1.0.0/

anvil info blender-4.2
```

Both layouts coexist in the same directory and are scanned in one pass.

---

## 6. Scaffold a new package

All commands below run from the repo root.

### Nested (default)

```bash
anvil init my-tool --version 2.0
# Created my-tool/2.0/package.yaml

cat my-tool/2.0/package.yaml
rm -rf my-tool
```

### Flat file

```bash
anvil init quick-fix --flat
# Created quick-fix-1.0.0.yaml

cat quick-fix-1.0.0.yaml
rm quick-fix-1.0.0.yaml
```

---

## 7. Lockfiles

Pin resolved versions so the same environment is reproduced everywhere:

```bash
# Resolve and lock
anvil lock houdini-20.5
# Locked 2 packages to anvil.lock:
#   python-3.11
#   houdini-20.5

cat anvil.lock

# Subsequent resolutions prefer locked versions
RUST_LOG=anvil=info anvil env houdini 2>&1 | head -3
# INFO Using lockfile: ".../anvil.lock"

# Clean up
rm anvil.lock
```

---

## 8. Saved contexts

Save a fully resolved environment to a portable JSON file:

```bash
# Save context
anvil context save houdini-20.5 -o /tmp/houdini.ctx.json
# Saved context (2 packages) to /tmp/houdini.ctx.json

# Show what's in it
anvil context show /tmp/houdini.ctx.json

# Show as JSON (for scripts)
anvil context show /tmp/houdini.ctx.json --json | head -5

# Show as shell exports (for sourcing)
anvil context show /tmp/houdini.ctx.json --export | grep HOUDINI

# Run a command using the saved context (no re-resolution needed)
anvil context run /tmp/houdini.ctx.json -- echo "Running from saved context"

rm /tmp/houdini.ctx.json
```

---

## 9. Wrapper scripts

Generate executable wrappers for all commands from resolved packages. This is how you create **tool suites** for departments:

```bash
# Generate wrappers for houdini + its dependencies
anvil wrap houdini-20.5 --dir /tmp/anvil-wrappers
# Created 7 wrapper(s) in /tmp/anvil-wrappers
#   python
#   hython
#   houdini
#   ...

# Each wrapper is an executable that calls anvil run
cat /tmp/anvil-wrappers/houdini
# #!/usr/bin/env bash
# exec anvil run python-3.11 houdini-20.5 -- houdini "$@"

# Artists add this to PATH and use tools directly:
# export PATH="/tmp/anvil-wrappers:$PATH"
# houdini -scene myfile.hip

rm -rf /tmp/anvil-wrappers
```

---

## 10. Publishing packages

Copy a validated package to a shared repository:

```bash
mkdir -p /tmp/anvil-repo

# Publish a nested package
anvil publish /tmp/anvil-repo --path examples/packages/houdini/20.5
# Published houdini-20.5 to /tmp/anvil-repo/houdini/20.5

# Publish a flat file
anvil publish /tmp/anvil-repo --path examples/packages/nuke-15.1.yaml --flat
# Published nuke-15.1 to /tmp/anvil-repo/nuke-15.1.yaml

ls /tmp/anvil-repo/
# houdini/  nuke-15.1.yaml

rm -rf /tmp/anvil-repo
```

---

## 11. Per-project config

Anvil searches for `.anvil.yaml` in the current directory and its parents. This section creates a self-contained example in `/tmp`:

```bash
mkdir -p /tmp/myproject/packages

cat > /tmp/myproject/packages/mytools-1.0.yaml << 'EOF'
name: mytools
version: "1.0"
environment:
  MYTOOLS: enabled
EOF

cat > /tmp/myproject/.anvil.yaml << 'EOF'
package_paths:
  - ./packages
EOF

# Use an empty global config so only the project config is active
cd /tmp/myproject
ANVIL_CONFIG=/dev/null anvil list
# mytools

ANVIL_CONFIG=/dev/null anvil env mytools | grep MYTOOLS
# MYTOOLS=enabled

cd - && rm -rf /tmp/myproject
```

---

## 12. Hooks

Run scripts before/after resolution or command execution:

```bash
mkdir -p /tmp/hooktest/packages

cat > /tmp/hooktest/packages/demo-1.0.yaml << 'EOF'
name: demo
version: "1.0"
environment:
  DEMO: active
EOF

cat > /tmp/hooktest/.anvil.yaml << 'EOF'
package_paths:
  - ./packages
hooks:
  pre_run:
    - echo "[pre-run hook] checking license..."
  post_run:
    - echo "[post-run hook] logging usage"
EOF

cd /tmp/hooktest
anvil run demo-1.0 -- echo "MAIN COMMAND"
# [pre-run hook] checking license...
# MAIN COMMAND
# [post-run hook] logging usage

cd - && rm -rf /tmp/hooktest
```

If a pre-hook exits non-zero, the operation is aborted.

---

## 13. Package filters

Restrict which packages are visible:

```bash
mkdir -p /tmp/filtertest/packages

cat > /tmp/filtertest/packages/maya-2024.yaml << 'EOF'
name: maya
version: "2024"
environment:
  MAYA: "2024"
EOF

cat > /tmp/filtertest/packages/nuke-15.yaml << 'EOF'
name: nuke
version: "15"
environment:
  NUKE: "15"
EOF

cat > /tmp/filtertest/packages/test-debug-1.0.yaml << 'EOF'
name: test-debug
version: "1.0"
environment:
  DEBUG: "1"
EOF

cat > /tmp/filtertest/.anvil.yaml << 'EOF'
package_paths:
  - ./packages
filters:
  include:
    - "maya*"
    - "nuke*"
  exclude:
    - "test-*"
EOF

cd /tmp/filtertest
anvil list
# maya
# nuke
# (test-debug is excluded)

cd - && rm -rf /tmp/filtertest
```

---

## 14. Shell completions

Generate tab completion scripts:

```bash
# Bash
anvil completions bash > /tmp/anvil.bash
head -3 /tmp/anvil.bash
# _anvil() {
#     local i cur prev opts cmd
#     COMPREPLY=()

# Zsh
anvil completions zsh > /tmp/anvil.zsh
head -3 /tmp/anvil.zsh
# #compdef anvil
# autoload -U is-at-least

rm /tmp/anvil.bash /tmp/anvil.zsh
```

To enable permanently, add to your shell's rc file:

```bash
# Bash: add to ~/.bashrc
eval "$(anvil completions bash)"

# Zsh: add to ~/.zshrc
eval "$(anvil completions zsh)"

# Fish
anvil completions fish | source
```

---

## 15. Scan caching

Anvil caches package scan results automatically. The second run is faster:

```bash
RUST_LOG=anvil=info anvil list 2>&1 | grep -i "loaded\|cached"
# INFO Loaded 7 packages

RUST_LOG=anvil=info anvil list 2>&1 | grep -i "loaded\|cached"
# INFO Using cached package scan
# INFO Loaded 7 packages (cached)
```

The cache is invalidated automatically when package files change.
