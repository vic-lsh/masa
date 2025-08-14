#!/bin/bash

# Script to write docker container logs for each service to a file

declare -a container_names=(
  "synthetic_frontend"
)

source ./scripts/local/.env

for i in $(seq 1 $CHILD_REPLICAS); do
  container_names+=("local-child-service-$i")
done

for ((i=0; i<${#container_names[@]}; i++)); do
  service=${container_names[$i]}
  
  # Get the container ID for the current container
  container_id=$(docker ps -a --filter "name=$service\$" --format "{{.ID}}")
  
  # Send the command to follow logs
  if [ -n "$container_id" ]; then
    docker logs $container_id &> $1/$service.log
  else
    echo "Container with name ${service} not found"
  fi
done
