#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
folder=snippets/test
data=tmp_1118

echo "Running fifo..."
mkdir -p $folder/fifo/$data
$pwd/scripts/local/run_all.sh \
	--rust-log warn \
	--tracker-capacity 1024 \
	--pctl-deadline 50 \
	--pctl-latest-exec 50 \
	--cargo-features fifo \
	--gen-config $folder/gen_config.json \
	--hotel-config $folder/hotel_config.json \
	--output-path $folder/fifo/$data \
	--repeats 1

# echo "Running e2e..."
# $pwd/scripts/local/run_all.sh \

# echo "Running e2e_er..."
# $pwd/scripts/local/run_all.sh \

echo "Running local..."
mkdir -p $folder/local/$data
$pwd/scripts/local/run_all.sh \
	--rust-log warn \
	--tracker-capacity 1024 \
	--pctl-deadline 50 \
	--pctl-latest-exec 50 \
	--cargo-features prio_local \
	--gen-config $folder/gen_config.json \
	--hotel-config $folder/hotel_config.json \
	--output-path $folder/local/$data \
	--repeats 1

# echo "Running local_er..."
# $pwd/scripts/local/run_all.sh \
