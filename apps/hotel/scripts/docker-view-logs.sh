#!/bin/bash

# Script to create a tmux session with panes for docker container logs
# Each pane corresponds to one container based on its image name

# Define the image names and a single tag
declare -a image_names=(
  "hotel_frontend"
  "hotel_search"
  "hotel_profile"
  "hotel_rate" 
  "hotel_reservation"
  "hotel_user"
)

IMAGE_TAG="latest"

SESSION_NAME="hotel-logs"

# Kill the session if it already exists
tmux kill-session -t $SESSION_NAME 2>/dev/null

# Create the tmux session with a new window named "logs"
tmux new-session -d -s $SESSION_NAME -n "logs"
tmux set-option -s pane-border-status top
tmux set-option -s pane-border-format "#{pane_title}"

first_pane=true

for ((i=0; i<${#image_names[@]}; i++)); do
  service=${image_names[$i]}
  if [ "$first_pane" = true ]; then
    tmux select-pane -T $service
    first_pane=false
  else
    tmux split-window -h -t $SESSION_NAME
    tmux select-pane -T $service
    tmux select-layout -t $SESSION_NAME tiled
  fi
  
  # Get the container ID for the current image
  current_image="${image_names[$i]}:${IMAGE_TAG}"
  container_id=$(docker ps --filter "ancestor=${current_image}" --format "{{.ID}}")
  
  # Send the command to follow logs
  if [ -n "$container_id" ]; then
    tmux send-keys -t $SESSION_NAME "echo 'Watching logs for ${image_names[$i]}:${IMAGE_TAG} (${container_id})'; docker logs -f $container_id" C-m
  else
    tmux send-keys -t $SESSION_NAME "echo 'Container with image ${image_names[$i]}:${IMAGE_TAG} not found'" C-m
  fi
done


tmux attach-session -t $SESSION_NAME
