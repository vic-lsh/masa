#!/usr/bin/env bash

# Script to write docker container logs for each service to a file

# Define the image names and a single tag
declare -a container_names=(
  "hotel_frontend"
)

source ./scripts/local/.env

declare -A replicated_services
replicated_services["rate"]=$RATE_REPLICAS
replicated_services["profile"]=$PROFILE_REPLICAS
replicated_services["reservation"]=$RESERVATION_REPLICAS
replicated_services["geo"]=$GEO_REPLICAS
replicated_services["search"]=$SEARCH_REPLICAS
replicated_services["user"]=$USER_REPLICAS

for service in "${!replicated_services[@]}"; do
    count="${replicated_services[$service]}"
    for i in $(seq 1 $count); do
      container_names+=("local-$service-service-$i")
    done
done

for ((i=0; i<${#container_names[@]}; i++)); do
  name=${container_names[$i]}
  docker logs $name &> $1/$name.log
done
