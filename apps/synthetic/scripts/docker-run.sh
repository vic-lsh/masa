#!/bin/bash

../scripts/docker-run.sh "$@"

# configure CPU shares of presampled replicas
source ./scripts/local/.env

i=$((CONSTANT_REPLICAS + RANDOM_REPLICAS + 1))

jq -r '.child_presampled_services[] | "\(.[0]) \(.[1] // 1)"' ./scripts/local/config.docker.json | while read -r replicas share; do
    for j in $(seq 1 $replicas); do
        docker update --cpus="$share" "local-child-service-$i"
        i=$((i + 1))
    done
done
