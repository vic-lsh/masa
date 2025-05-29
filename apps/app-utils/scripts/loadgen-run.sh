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

cargo run --release --bin $app_client_bench -- \
   --gen-config ./scripts/gen_config.json \
   $output_arg \
   $save_logs_arg
