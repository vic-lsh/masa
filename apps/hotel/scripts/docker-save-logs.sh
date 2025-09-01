#!/usr/bin/env bash

# Script to write docker container logs for each service to a file

output_path=""
follow="false"
while [[ $# -gt 0 ]]; do
    case $1 in
    --follow)
        follow="true"
        shift 1
        ;;
    --output)
        output_path=$2
        shift 2
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done


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

mkdir -p $output_path
for ((i=0; i<${#container_names[@]}; i++)); do
    name=${container_names[$i]}
    if [[ "$follow" = "true" ]]; then
        docker logs -f $name &> $output_path/$name.log &
    else
        docker logs $name &> $output_path/$name.log
    fi
done
