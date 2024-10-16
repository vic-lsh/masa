#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */MPD237 ]]; then
    echo "Error: please run in the MPD237 root directory" >&2
    exit 1
fi

SESSION_NAME="build"
USERNAME=$(whoami)
NODE_MIN=0
NODE_MAX=2

echo "Syncing source files..."
DIRS="/users/$USERNAME/MPD237/reboot/"
for dir in $DIRS; do
    for ((i = $NODE_MIN; i <= $NODE_MAX; i++)); do
        if [[ $i -eq 0 ]]; then
            continue
        fi
        echo "Syncing $dir to node $i"
        rsync -av --exclude='target' $dir node$i:$dir >/dev/null
    done
done

tmux new-session -d -s $SESSION_NAME
tmux set-option -s pane-border-status top
tmux set-option -s pane-border-format "#{pane_title}"

first_pane=true

for ((i = $NODE_MIN; i <= $NODE_MAX; i++)); do
    node=node$i

    CMD="cd ~/MPD237/reboot && cargo build --release"
    SSH_CMD="ssh $node -t 'bash -l -c \"$CMD\"; /bin/bash -i'"
    EXE_CMD=
    if [[ $i -eq 0 ]]; then
        EXE_CMD=$CMD
    else
        EXE_CMD=$SSH_CMD
    fi

    if [ "$first_pane" = true ]; then
        tmux send-keys -t $SESSION_NAME "$EXE_CMD" C-m
        tmux select-pane -T $node
        first_pane=false
    else
        tmux split-window -h -t $SESSION_NAME
        tmux select-pane -T $node
        tmux select-layout -t $SESSION_NAME tiled
        tmux send-keys -t $SESSION_NAME "$EXE_CMD" C-m
    fi
done

tmux attach-session -t $SESSION_NAME
