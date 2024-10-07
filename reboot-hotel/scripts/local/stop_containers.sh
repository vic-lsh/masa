#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */MPD237 ]]; then
    echo "Error: plese run in the MPD237 root directory" >&2
    exit 1
fi

SESSION_NAME="stop_containers"

tmux new-session -d -s $SESSION_NAME
tmux set-option -s pane-border-status top
tmux set-option -s pane-border-format "#{pane_title}"

CMD="cd ~/MPD237/reboot/scripts/local; docker compose -f containers.yaml down"

tmux send-keys -t $SESSION_NAME "$CMD" C-m

tmux attach-session -t $SESSION_NAME
