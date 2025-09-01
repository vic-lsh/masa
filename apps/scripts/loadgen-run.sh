#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */apps/* ]]; then
    echo "Error: please run in an application directory" >&2
    exit 1
fi
app=$(basename $pwd)

save_logs_arg=""
output_path=

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --output)
        output_path=$2
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

if [[ -z $output_path ]]; then
    echo "--output must be set"
    exit 1
fi

container_name=hotel_client_bench
network=local_hotel_network

docker rm -f $container_name

mkdir -p $output_path

# Make sure that this is consistent with the network name created by docker compose
docker run \
    --name $container_name \
    --network $network \
    hotel_client_bench \
    &> $output_path/loadgen.log

# this should be hard-coded in the docker entrypoint.sh
container_trace_path="/tmp/masa-load-gen"

# Copy traces from inside the container to outside
docker cp $container_name:$container_trace_path $output_path
# hacky way to make sure that the trace files are actually placed in $output_path
mv $output_path/masa-load-gen/* $output_path
rm -rf $output_path/masa-load-gen
