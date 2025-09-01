#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */apps/* ]]; then
    echo "Error: please run in an application directory" >&2
    exit 1
fi
app=$(basename $pwd)

output_arg="--output-path /tmp/masa-load-gen"
save_logs_arg=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --output)
        output_arg="--output-path $2"
        shift 2
        ;;
    --save-logs)
        save_logs_arg="--save-logs"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

container_name=hotel_client_bench
network=local_hotel_network

docker rm -f $container_name

# Make sure that this is consistent with the network name created by docker compose
docker run \
    --name $container_name \
    --network $network \
    hotel_client_bench
