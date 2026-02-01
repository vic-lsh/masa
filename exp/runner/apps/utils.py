"""
Shared utilities for application plugins.
"""

import os
import re
import sys
from typing import Optional





def get_docker_progress_flag() -> str:
    """
    Get the appropriate docker build progress flag based on environment.
    
    In CI environments (detected via CI environment variable), use 'plain'
    progress mode since TTY is not available. Otherwise, use 'tty' for
    better interactive output.
    
    Returns:
        Progress flag string: '--progress=plain' in CI, '--progress=tty' otherwise
    """
    # Check for CI environment variable (set by most CI systems including GitLab CI)
    if os.environ.get("CI", "").lower() in ("true", "1", "yes"):
        return "--progress=plain"
    # If no TTY is attached (e.g., non-interactive runner), use plain output.
    if not sys.stdout.isatty():
        return "--progress=plain"
    return "--progress=tty"






