#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/search_reservation

# echo "Running fifo..."
# $pwd/scripts/local/run_all.sh \
# 	--features fifo \
# 	--output $snippets/fifo \
# 	--repeats 1

# echo "Running e2e..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_global \
# 	--output $snippets/e2e \
# 	--repeats 1

# echo "Running e2e_er..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_global_early \
# 	--output $snippets/e2e_er \
# 	--repeats 1

echo "Running local..."
$pwd/scripts/local/run_all.sh \
	--features prio_local \
	--output $snippets/local \
	--repeats 1

echo "Running local_er..."
$pwd/scripts/local/run_all.sh \
	--features prio_local_early \
	--output $snippets/local_er \
	--repeats 1
