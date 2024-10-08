#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */MPD237 ]]; then
    echo "Error: plese run in the MPD237 root directory" >&2
    exit 1
fi

SESSION_NAME="run"
NODE_MIN=0
NODE_MAX=2

SVCS=("hotel_geo" "hotel_rate" "hotel_profile" "hotel_search")
WAIT_SECS=("0" "0" "0" "9" "12")

tmux new-session -d -s $SESSION_NAME -n "node$NODE_MIN"

for i in $(seq $NODE_MIN $NODE_MAX); do
    if [ $i -ne 0 ]; then
        tmux new-window -t $SESSION_NAME -n "node$i"
    fi

    tmux set-option -s pane-border-status top
    tmux set-option -s pane-border-format "#{pane_title}"

    node=node$i
    first_pane=true

    if [[ $i -eq 0 ]]; then
        continue
    fi

    if [[ $i -eq 1 ]]; then
        svc="hotel_frontend"
        wait_secs=${WAIT_SECS[4]}

        RUN_CMD="cargo run --release --bin $svc"
        CMD="cd ~/MPD237/reboot; sleep $wait_secs; $RUN_CMD"
        SSH_CMD="ssh $node -t 'bash -i -l -c \"$CMD; exec bash\"'"

        tmux select-pane -T $svc
        tmux send-keys -t $SESSION_NAME "$SSH_CMD" C-m

        continue
    fi

    for j in "${!SVCS[@]}"; do
        svc=${SVCS[$j]}
        wait_secs=${WAIT_SECS[$j]}

        RUN_CMD="cargo run --release --bin $svc"
        if [[ $svc == "hotel_rate" ]]; then
            RUN_CMD="cargo run --release --bin $svc -- --config ~/MPD237/reboot/snippets/variations/config.json"
        fi
        if [[ $svc == "hotel_profile" ]]; then
            RUN_CMD="cargo run --release --bin $svc -- --config ~/MPD237/reboot/snippets/variations/config.json"
        fi
        CMD="cd ~/MPD237/reboot; sleep $wait_secs; $RUN_CMD"
        SSH_CMD="ssh $node -t 'bash -i -l -c \"$CMD; exec bash\"'"

        if [ "$first_pane" = true ]; then
            tmux select-pane -T $svc
            first_pane=false
        else
            tmux split-window -h -t $SESSION_NAME
            tmux select-pane -T $svc
            tmux select-layout -t $SESSION_NAME tiled
        fi
        tmux send-keys -t $SESSION_NAME "$SSH_CMD" C-m
    done
done

tmux attach-session -t $SESSION_NAME
