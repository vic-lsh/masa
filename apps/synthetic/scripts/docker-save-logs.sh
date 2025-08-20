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
  name=${container_names[$i]}
  docker logs $name &> $1/$name.log
done
