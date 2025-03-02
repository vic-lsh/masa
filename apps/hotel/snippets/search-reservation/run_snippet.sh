#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
	echo "Error: plese run in the apps/hotel directory" >&2
	exit 1
fi

folder=snippets/search-reservation
data=tmp_1129_gcp

echo "Running fifo..."
$pwd/scripts/local/run_all_docker.sh \
	--rust-log warn \
	--tracker-capacity 1024 \
	--pctl-deadline 50 \
	--pctl-latest-exec 50 \
	--cargo-features fifo \
	--gen-config $folder/gen_config.json \
	--hotel-config $folder/hotel_config.json \
	--output-path $folder/fifo/$data \
	--repeats 1

# echo "Running fifo_er..."
# $pwd/scripts/local/run_all_docker.sh \
# 	--rust-log warn \
# 	--tracker-capacity 1024 \
# 	--pctl-deadline 50 \
# 	--pctl-latest-exec 50 \
# 	--cargo-features fifo_early \
# 	--gen-config $folder/gen_config.json \
# 	--hotel-config $folder/hotel_config.json \
# 	--output-path $folder/fifo_er/$data \
# 	--repeats 1

# echo "Running e2e..."
# $pwd/scripts/local/run_all_docker.sh \
# 	--rust-log warn \
# 	--tracker-capacity 1024 \
# 	--pctl-deadline 50 \
# 	--pctl-latest-exec 50 \
# 	--cargo-features prio_global \
# 	--gen-config $folder/gen_config.json \
# 	--hotel-config $folder/hotel_config.json \
# 	--output-path $folder/e2e/$data \
# 	--repeats 1

# echo "Running e2e_er..."
# $pwd/scripts/local/run_all_docker.sh \
# 	--rust-log warn \
# 	--tracker-capacity 1024 \
# 	--pctl-deadline 50 \
# 	--pctl-latest-exec 50 \
# 	--cargo-features prio_global_early \
# 	--gen-config $folder/gen_config.json \
# 	--hotel-config $folder/hotel_config.json \
# 	--output-path $folder/e2e_er/$data \
# 	--repeats 1

# echo "Running local..."
# $pwd/scripts/local/run_all_docker.sh \
# 	--rust-log warn \
# 	--tracker-capacity 1024 \
# 	--pctl-deadline 50 \
# 	--pctl-latest-exec 50 \
# 	--cargo-features prio_local \
# 	--gen-config $folder/gen_config.json \
# 	--hotel-config $folder/hotel_config.json \
# 	--output-path $folder/local/$data \
# 	--repeats 1

# echo "Running local_er..."
# $pwd/scripts/local/run_all_docker.sh \
# 	--rust-log warn \
# 	--tracker-capacity 1024 \
# 	--pctl-deadline 50 \
# 	--pctl-latest-exec 50 \
# 	--cargo-features prio_local_early \
# 	--gen-config $folder/gen_config.json \
# 	--hotel-config $folder/hotel_config.json \
# 	--output-path $folder/local_er/$data \
# 	--repeats 1
