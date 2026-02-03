#!/bin/bash
set -e

# Run python tests using uv and pytest
# Configuration is picked up from pyproject.toml and pytest.ini
uv run pytest "$@"
