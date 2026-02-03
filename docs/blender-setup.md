# Using pconfig with Blender

This guide shows how to set up Blender with your studio's custom addons and Python libraries using pconfig.

## Directory Structure

```
~/packages/
├── blender/
│   ├── 4.2/
│   │   └── package.yaml
│   └── 4.3/
│       └── package.yaml
├── studio-blender-tools/
│   ├── 1.0.0/
│   │   ├── package.yaml
│   │   └── addons/
│   │       └── studio_tools/
│   │           └── __init__.py
│   └── 1.1.0/
│       └── ...
└── studio-python-core/
    └── 1.0.0/
        ├── package.yaml
        └── python/
            └── studio/
                └── __init__.py
```

## Package Definitions

### blender/4.2/package.yaml

```yaml
name: blender
version: "4.2"
description: Blender 4.2 LTS

environment:
  # Path to Blender install (adjust for your system)
  BLENDER_HOME: /opt/blender/4.2
  PATH: ${BLENDER_HOME}:${PATH}
  
  # User scripts location (where addons are loaded from)
  BLENDER_USER_SCRIPTS: ${BLENDER_USER_SCRIPTS:-}
  
  # Python path for Blender's embedded Python
  BLENDER_PYTHON: ${BLENDER_HOME}/4.2/python/bin/python3.11

variants:
  - platform: macos
    environment:
      BLENDER_HOME: /Applications/Blender.app/Contents/MacOS
      BLENDER_PYTHON: /Applications/Blender.app/Contents/Resources/4.2/python/bin/python3.11
  
  - platform: windows
    environment:
      BLENDER_HOME: C:/Program Files/Blender Foundation/Blender 4.2
      BLENDER_PYTHON: ${BLENDER_HOME}/4.2/python/bin/python.exe

commands:
  blender: ${BLENDER_HOME}/blender
```

### studio-blender-tools/1.0.0/package.yaml

```yaml
name: studio-blender-tools
version: 1.0.0
description: Studio Blender addons and tools

requires:
  - blender-4.2+
  - studio-python-core-1.0+

environment:
  # Add our addons to Blender's addon path
  BLENDER_USER_SCRIPTS: ${PACKAGE_ROOT}
  
  # Also add to PYTHONPATH for imports
  PYTHONPATH: ${PACKAGE_ROOT}/addons:${PYTHONPATH:-}
```

### studio-python-core/1.0.0/package.yaml

```yaml
name: studio-python-core
version: 1.0.0
description: Core Python libraries for studio pipeline

environment:
  PYTHONPATH: ${PACKAGE_ROOT}/python:${PYTHONPATH:-}
  STUDIO_CONFIG: ${HOME}/.studio/config.yaml
```

## Usage

### Launch Blender with Studio Tools

```bash
# Run Blender with all studio tools
pconfig run blender-4.2 studio-blender-tools -- blender

# Or use an alias (see config below)
pconfig run studio-blender -- blender

# Open a specific file
pconfig run studio-blender -- blender /path/to/scene.blend
```

### Interactive Shell

```bash
# Start a shell with Blender environment
pconfig shell blender-4.2 studio-blender-tools

# Now you can:
blender                    # Launch Blender
python -c "import studio"  # Use studio Python libs
```

### Export Environment (for IDE/scripts)

```bash
# Get environment as shell exports
pconfig env blender-4.2 studio-blender-tools --export > /tmp/blender_env.sh
source /tmp/blender_env.sh

# Or as JSON for programmatic use
pconfig env studio-blender --json > env.json
```

## Configuration

### ~/.pconfig.yaml

```yaml
# Package search paths
package_paths:
  - ~/packages                    # Local packages
  - /studio/packages              # Shared studio packages
  - ${STUDIO_PACKAGES:-}          # Custom path from env

# Aliases for common combinations
aliases:
  studio-blender:
    - blender-4.2
    - studio-blender-tools
    - studio-python-core
  
  studio-blender-dev:
    - blender-4.3
    - studio-blender-tools
    - studio-python-core

# Platform overrides
platform:
  macos:
    package_paths:
      - /Volumes/Studio/packages
  
  linux:
    package_paths:
      - /mnt/studio/packages
```

## Creating a New Addon Package

1. Create the directory structure:

```bash
mkdir -p ~/packages/my-addon/1.0.0/addons/my_addon
```

2. Create `package.yaml`:

```yaml
name: my-addon
version: 1.0.0
description: My custom Blender addon

requires:
  - blender-4.2+

environment:
  BLENDER_USER_SCRIPTS: ${PACKAGE_ROOT}
```

3. Add your addon code in `addons/my_addon/__init__.py`:

```python
bl_info = {
    "name": "My Addon",
    "author": "Your Name",
    "version": (1, 0, 0),
    "blender": (4, 2, 0),
    "category": "Pipeline",
}

def register():
    print("My Addon registered!")

def unregister():
    pass
```

4. Test it:

```bash
pconfig run blender-4.2 my-addon -- blender
# Your addon will appear in Edit > Preferences > Add-ons
```

## Tips

### Version Constraints

```bash
# Exact version
pconfig run blender-4.2 -- blender

# Minimum version (4.2 or higher)
pconfig run blender-4.2+ -- blender

# Version range
pconfig run blender-4.0..4.2 -- blender

# Multiple options
pconfig run blender-4.2|4.3 -- blender

# Any version
pconfig run blender -- blender
```

### Debugging Environment

```bash
# See what packages are resolved
pconfig info blender-4.2

# See full environment
pconfig env blender-4.2 studio-blender-tools

# Validate all packages
pconfig validate
```

### Integration with VS Code

Create `.vscode/settings.json` in your project:

```json
{
  "python.envFile": "${workspaceFolder}/.env"
}
```

Generate the env file:

```bash
pconfig env studio-blender --export > .env
```

Now VS Code's Python extension will use the correct paths for autocomplete.
