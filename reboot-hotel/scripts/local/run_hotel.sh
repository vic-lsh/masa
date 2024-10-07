#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

SESSION_NAME="hotel"
SVCS=("hotel_geo" "hotel_rate" "hotel_search" "hotel_frontend")

tmux new-session -d -s $SESSION_NAME -n "local"
tmux set-option -s pane-border-status top
tmux set-option -s pane-border-format "#{pane_title}"

first_pane=true

for i in "${!SVCS[@]}"; do
    svc=${SVCS[$i]}
    wait_secs=$((i * 1))

    RUN_CMD="cargo run --release --features \"prio_global\" --bin $svc > tmp_$svc.log 2>&1"
    CMD="cd ~/Masa-Lo-Ding/reboot-hotel; sleep $wait_secs; $RUN_CMD"

    if [ "$first_pane" = true ]; then
        tmux select-pane -T $svc
        first_pane=false
    else
        tmux split-window -h -t $SESSION_NAME
        tmux select-pane -T $svc
        tmux select-layout -t $SESSION_NAME tiled
    fi
    tmux send-keys -t $SESSION_NAME "$CMD" C-m
done

tmux attach -t $SESSION_NAME
