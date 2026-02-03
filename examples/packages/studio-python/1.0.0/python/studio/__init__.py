"""Studio core Python library.

Example usage:
    from studio import config, paths
    
    project = config.get_current_project()
    asset_path = paths.get_asset_path("char", "hero")
"""

__version__ = "1.0.0"

import os
from pathlib import Path


def get_config_path() -> Path:
    """Get the studio config file path."""
    return Path(os.environ.get("STUDIO_CONFIG", "~/.studio/config.yaml")).expanduser()


def get_project_root() -> Path:
    """Get the current project root from environment."""
    if root := os.environ.get("STUDIO_PROJECT_ROOT"):
        return Path(root)
    return Path.cwd()


def get_user() -> str:
    """Get current user."""
    return os.environ.get("USER", os.environ.get("USERNAME", "unknown"))


# Convenience re-exports
def hello():
    """Test function to verify the package is loaded."""
    print(f"Hello from studio-python {__version__}!")
    print(f"  User: {get_user()}")
    print(f"  Project: {get_project_root()}")
