#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

docker compose -f $SCRIPT_DIR/local/containers+svcs.yaml up -d
