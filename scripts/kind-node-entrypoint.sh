#!/bin/sh
set -eu

# systemd in kind nodes needs more than the default soft nofile limit.
hard="$(ulimit -Hn 2>/dev/null || echo "")"
if [ -n "$hard" ] && [ "$hard" != "unlimited" ]; then
    ulimit -Sn "$hard" 2>/dev/null || true
fi

exec /usr/local/bin/entrypoint.real "$@"
