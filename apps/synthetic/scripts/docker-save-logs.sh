#!/bin/bash

# Script to write docker container logs for each service to a file

declare -a container_names=(
  "synthetic_frontend"
  "synthetic_child_0"
  "synthetic_child_1"
)

for ((i=0; i<${#container_names[@]}; i++)); do
  service=${container_names[$i]}
  
  # Get the container ID for the current container
  container_id=$(docker ps --filter "name=" --format "{{.ID}}")
  
  # Send the command to follow logs
  if [ -n "$container_id" ]; then
    docker logs $container_id &> $1/$service.log
  else
    echo "Container with name ${container_names[$i]} not found"
  fi
done
