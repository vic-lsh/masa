#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

snippets=snippets/single/search
data=tmp_1118

# echo "Running fifo..."
# $pwd/scripts/local/run_all.sh \

# echo "Running e2e..."
# $pwd/scripts/local/run_all.sh \

# echo "Running e2e_er..."
# $pwd/scripts/local/run_all.sh \

# echo "Running local..."
# $pwd/scripts/local/run_all.sh \

# echo "Running local_er..."
# $pwd/scripts/local/run_all.sh \
