#!/bin/bash
set -e

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )"

# Parse arguments
ARGS=""
MODE_MSG="Running all formatters..."
for arg in "$@"; do
  if [[ "$arg" == "--check" ]]; then
    ARGS="--check"
    MODE_MSG="Checking formatting..."
    break
  fi
done

echo "$MODE_MSG"

"$SCRIPT_DIR/format_rust.sh" $ARGS
"$SCRIPT_DIR/format_python.sh" $ARGS

echo "Done."
