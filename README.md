# Anvil

A fast environment resolver for VFX and animation pipelines. YAML package
definitions, instant resolution, single static binary. Think Rez, written in
Rust, with a small surface area.

[![CI](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml/badge.svg)](https://github.com/voidreamer/anvil/actions/workflows/rust-ci.yml)
[![Release](https://img.shields.io/github/v/release/voidreamer/anvil?include_prereleases)](https://github.com/voidreamer/anvil/releases)

## Install

```bash
cargo install anvil-env
```

Or build from source with `cargo build --release` and use `target/release/anvil`.

`cargo install` drops the binary in `~/.cargo/bin/anvil`. Add that directory to
`PATH` so wrappers, hooks, and project scripts can call `anvil` directly
instead of hard-coding the absolute path:

```bash
# bash / zsh
export PATH="$HOME/.cargo/bin:$PATH"
```

## Quick start

Run `anvil init --config` to scaffold a starter `~/.anvil.yaml`, then write a
package at `~/packages/maya-2024.yaml`:

```yaml
name: maya
version: "2024"
requires:
  - python-3.11
environment:
  MAYA_LOCATION: /usr/autodesk/maya2024
  PATH: ${MAYA_LOCATION}/bin:${PATH}
commands:
  maya: ${MAYA_LOCATION}/bin/maya
  mayapy: ${MAYA_LOCATION}/bin/mayapy
```

Point anvil at it via `~/.anvil.yaml`:

```yaml
package_paths:
  - ~/packages
```

Then:

```bash
anvil list                              # show available packages
anvil info maya-2024                    # inspect one
anvil env maya-2024                     # print resolved environment
anvil run maya-2024 -- maya             # launch with alias resolution
anvil shell maya-2024                   # interactive shell
```

The `commands` map lets `anvil run <packages> -- <alias>` resolve the alias to
its expanded binary path, so you never repeat paths in wrappers or scripts.

## Package definition

A package needs only a `name` and `version`. Everything else is optional.

```yaml
name: houdini
version: "20.5"
description: SideFX Houdini 20.5

requires:
  - python-3.11

environment:
  HFS: ${PACKAGE_ROOT}
  PATH: ${HFS}/bin:${PATH}
  PYTHONPATH: ${HFS}/python/lib/python3.11/site-packages:${PYTHONPATH}

commands:
  houdini: ${HFS}/bin/houdini
  hython: ${HFS}/bin/hython

variants:
  - platform: linux
    environment:
      HFS: /opt/hfs20.5
  - platform: macos
    environment:
      HFS: /Applications/Houdini/Houdini20.5/Frameworks/Houdini.framework/Versions/20.5/Resources
```

### Layouts

Flat files live in the package directory as `<name>-<version>.yaml`:

```
~/packages/
  maya-2024.yaml
  arnold-7.2.yaml
```

Nested directories live as `<name>/<version>/package.yaml` and are better when
the package ships associated files:

```
~/packages/
  maya/2024/
    package.yaml
    scripts/
    modules/
```

Both layouts can coexist in one package path.

### Version constraints

Used inside `requires` and at the CLI.

| Form | Meaning |
|---|---|
| `maya-2024` | exactly 2024 |
| `maya-2024+` | 2024 or higher |
| `maya-2024..2025` | 2024 through 2025 inclusive |
| `python-3.10\|3.11` | 3.10 or 3.11 |
| `maya` | any version, highest wins |

Names with internal hyphens work (`studio-blender-tools-1.0.0`); anvil splits
only on the last hyphen when the suffix starts with a digit.

### Environment expansion

Values resolve in this order: `${PACKAGE_ROOT}`, `${VERSION}`, `${NAME}`, then
any `${VAR}` set by previously resolved packages or the inherited environment,
and finally a leading `~/`. When two packages set the same variable without
referencing `${VAR}` on the right, anvil emits a conflict warning so a silent
overwrite does not slip through.

### Command aliases

The `commands` map lets `anvil run` pick a program from the package definition.
Values expand the same way as `environment` values, and can include baked in
arguments with whitespace or tilde segments — anvil tokenises the value with
POSIX shell rules, expands `~/` in every token, then runs the first token
with the remaining tokens prepended to whatever the user passes after `--`.

```yaml
commands:
  # Bare path
  maya: ${MAYA_LOCATION}/bin/maya

  # Program + baked-in flags (multi-token alias)
  nukex: ${NUKE_HOME}/Nuke${VERSION} --nukex

  # Launcher in front of an interpreter (e.g. Python script with a specific runtime)
  usdview: python3.14 ~/USD/bin/usdview

  # Wrapper that injects defaults; user's `-- <extra args>` are appended
  hython-debug: ${HFS}/bin/hython -d -v
```

`anvil run nukex -- --view` therefore exec's
`${NUKE_HOME}/Nuke${VERSION} --nukex --view` — packages can ship sane defaults
for every tool they expose without forcing users to memorise flag soup. Quoted
substrings are preserved as a single argv element, so paths with spaces work
without escaping the whole value.

## Commands

All twelve commands at a glance.

### `anvil env`

Print the resolved environment.

```bash
anvil env maya-2024                     # KEY=VALUE
anvil env maya-2024 --export            # shell export lines
anvil env maya-2024 --json              # JSON object
```

### `anvil run`

Run a command with the resolved environment. The first token after `--` is
looked up in the merged `commands` map.

```bash
anvil run maya-2024 -- maya
anvil run maya-2024 arnold-7.2 -- maya -file scene.ma
anvil run maya-2024 -e MAYA_DEBUG=1 -- maya
```

Exits with the command's exit code.

### `anvil shell`

Start an interactive shell with packages loaded. On Unix the shell replaces the
current process.

```bash
anvil shell maya-2024 arnold-7.2
anvil shell maya-2024 --shell zsh
```

### `anvil list`

```bash
anvil list                              # all package names
anvil list maya                         # versions of one package
```

### `anvil info`

Show one package's metadata, environment, and commands map.

```bash
anvil info maya-2024
```

### `anvil validate`

Check that package definitions parse and resolve.

```bash
anvil validate                          # all packages
anvil validate maya-2024                # one package
```

### `anvil lock`

Pin resolved versions to `anvil.lock` for reproducible environments. Subsequent
`anvil env`, `run`, and `shell` prefer the pinned versions.

```bash
anvil lock maya-2024 arnold-7.2
anvil lock maya-2024 arnold-7.2 --update
```

The lockfile is YAML. Commit it alongside the project for team wide
reproducibility.

### `anvil context`

Freeze a fully resolved environment to JSON so render farms, CI, or other
machines can re-enter it without re-resolving.

```bash
anvil context save maya-2024 arnold-7.2 -o render.ctx.json
anvil context show render.ctx.json
anvil context show render.ctx.json --json
anvil context show render.ctx.json --export
anvil context run render.ctx.json -- maya -batch -file scene.ma
anvil context shell render.ctx.json
```

### `anvil init`

Scaffold a new package definition, or a starter global config.

```bash
anvil init my-tools                     # my-tools/1.0.0/package.yaml
anvil init my-tools --version 2.0       # my-tools/2.0/package.yaml
anvil init my-tools --flat              # my-tools-1.0.0.yaml
anvil init --config                     # ~/.anvil.yaml with a commented template
```

### `anvil completions`

Generate shell completions. Evaluate the output in your shell rc file.

```bash
eval "$(anvil completions bash)"
eval "$(anvil completions zsh)"
anvil completions fish | source
anvil completions powershell | Out-String | Invoke-Expression
```

### `anvil wrap`

Create executable wrapper scripts for every command in a set of resolved
packages. The wrappers call `anvil run` internally, so they always respect
lockfiles, project configs, and command aliases. Drop the output directory on
`$PATH` and artists call the tools directly.

```bash
anvil wrap houdini-20.5 --dir ~/tools/fx
export PATH="$HOME/tools/fx:$PATH"
houdini -scene myfile.hip
```

### `anvil publish`

Copy a validated package to a shared repository. Refuses to overwrite.

```bash
anvil publish /studio/packages --path ~/dev/my-tool
anvil publish /studio/packages --path ~/dev/my-tool --flat
```

## Configuration

### Global config

Anvil reads configuration in this order:

1. `$ANVIL_CONFIG` environment variable
2. `~/.anvil.yaml`
3. `~/.config/anvil/config.yaml`

### Project config

Anvil also walks the current directory and its parents looking for
`.anvil.yaml`. When found, it is merged with the global config. `package_paths`
from the project are prepended, aliases with the same name override globals,
`default_shell` wins if the project sets it, and per-platform paths are
prepended per-platform.

### Full example

```yaml
package_paths:
  - ~/packages
  - /studio/packages
  - ${STUDIO_ROOT}/packages

default_shell: zsh

aliases:
  maya-anim:
    - maya-2024
    - animbot-2.0
    - studio-tools
  studio-blender:
    - blender-4.2
    - studio-blender-tools

platform:
  linux:
    package_paths: [/mnt/shared/packages]
  windows:
    package_paths: [P:/packages]
  macos:
    package_paths: [/Volumes/shared/packages]

hooks:
  pre_resolve:
    - echo "resolving..."
  post_resolve:
    - /studio/scripts/log_resolution.sh
  pre_run:
    - /studio/scripts/check_license.sh
  post_run:
    - /studio/scripts/cleanup.sh

filters:
  include: ["maya-*", "arnold-*", "studio-*"]
  exclude: ["*-dev", "test-*"]
```

### Hooks

Shell commands run at lifecycle points. A non zero exit from any `pre_` hook
aborts the operation; `post_run` is best effort and never aborts.

| Hook | When |
|---|---|
| `pre_resolve` | before package resolution |
| `post_resolve` | after resolution with the resolved env |
| `pre_run` | before `anvil run` executes the command |
| `post_run` | after `anvil run` finishes |

### Filters

Limit which packages are visible. Patterns support `*` and `?`.

```yaml
filters:
  include: ["maya-*", "arnold-*"]
  exclude: ["*-dev"]
```

### Environment variables

| Variable | Purpose |
|---|---|
| `ANVIL_CONFIG` | override config file location |
| `ANVIL_PACKAGES` | additional package paths, colon separated |
| `RUST_LOG` | log verbosity, e.g. `RUST_LOG=debug` (overrides `-v`) |

By default anvil only logs warnings and errors so it can be piped safely
(`eval "$(anvil env maya-2024 --export)"`). Pass `-v` for info-level diagnostics
or `-vv` for debug.

If no config file is found, anvil falls back to `$ANVIL_PACKAGES`,
`$HOME/packages`, `$HOME/.local/share/anvil/packages`, and `/opt/packages`.

## Anvil vs Rez

Anvil targets the same problem as [Rez](https://github.com/AcademySoftwareFoundation/rez)
with a smaller surface area and no Python runtime.

|  | Anvil | Rez |
|---|---|---|
| Language | Rust, single static binary | Python |
| Startup | milliseconds | seconds (Python bootstrap) |
| Package format | YAML | `package.py` |
| Runtime deps | none | Python 3.7+, pip, platform bindings |
| Resolver | greedy | SAT solver with backtracking |
| Install | `cargo install anvil-env` | pip install plus `rez bind` plus config |
| Config surface | one YAML file, three env vars | many files, dozens of settings |

Anvil suits solo TDs and small to medium studios whose packages number in the
dozens rather than thousands, where fast iteration and simple operations matter
more than advanced constraint solving. Rez keeps the edge on large dependency
graphs with complex conflicts, built in build and release tooling, and a
centralised package server.

## Development

```bash
cargo build --release
cargo test
cargo fmt
cargo clippy
RUST_LOG=debug cargo run -- env maya-2024
```

## License

MIT
