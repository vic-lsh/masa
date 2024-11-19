#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/single/search
data=tmp_today

# echo "Running fifo..."
# mkdir -p $folder/fifo/$data
# $pwd/scripts/local/run_all.sh \

# echo "Running e2e..."
# mkdir -p $folder/e2e/$data
# $pwd/scripts/local/run_all.sh \

# echo "Running e2e_er..."
# mkdir -p $folder/e2e_er/$data
# $pwd/scripts/local/run_all.sh \

# echo "Running local..."
# mkdir -p $folder/local/$data
# $pwd/scripts/local/run_all.sh \

# echo "Running local_er..."
# mkdir -p $folder/local_er/$data
# $pwd/scripts/local/run_all.sh \
