#!/bin/sh
set -eu

# systemd in kind nodes can fail early if fd / inotify limits are too low.
# Raise limits opportunistically; ignore failures when the runtime forbids it.
hard="$(ulimit -Hn 2>/dev/null || echo "")"
if [ -n "$hard" ] && [ "$hard" != "unlimited" ]; then
    ulimit -Sn "$hard" 2>/dev/null || true
fi

#
# Newer CI kernels/runtimes can expose low inotify defaults inside Kind nodes,
# which makes systemd fail during early boot with EMFILE-like errors.
# Bump these values before launching PID 1.
if [ -w /proc/sys/fs/inotify/max_user_instances ]; then
    current_instances="$(cat /proc/sys/fs/inotify/max_user_instances 2>/dev/null || echo "")"
    if [ -n "$current_instances" ] && [ "$current_instances" -lt 8192 ]; then
        echo 8192 > /proc/sys/fs/inotify/max_user_instances 2>/dev/null || true
    fi
fi

if [ -w /proc/sys/fs/inotify/max_user_watches ]; then
    current_watches="$(cat /proc/sys/fs/inotify/max_user_watches 2>/dev/null || echo "")"
    if [ -n "$current_watches" ] && [ "$current_watches" -lt 524288 ]; then
        echo 524288 > /proc/sys/fs/inotify/max_user_watches 2>/dev/null || true
    fi
fi

exec /usr/local/bin/entrypoint.real "$@"
