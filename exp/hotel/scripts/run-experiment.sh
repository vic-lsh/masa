#!/bin/bash

set -e

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
COMMON_DIR="$SCRIPT_DIR/../../common/scripts"

exec "$COMMON_DIR/run-experiment.sh" "$@"
