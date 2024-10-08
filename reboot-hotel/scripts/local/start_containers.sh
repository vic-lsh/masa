#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

SESSION_NAME="start_containers"

tmux new-session -d -s $SESSION_NAME
tmux set-option -s pane-border-status top
tmux set-option -s pane-border-format "#{pane_title}"

CMD=" \
cd ~/Masa-Lo-Ding/reboot-hotel/scripts/local; \
docker compose -f containers.yaml up -d \
"

tmux send-keys -t $SESSION_NAME "$CMD" C-m

tmux attach-session -t $SESSION_NAME
