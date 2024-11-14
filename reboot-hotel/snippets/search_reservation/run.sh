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

# echo "Running local..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local \
# 	--output $snippets/local \
# 	--repeats 1

# echo "Running local_er..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local_early \
# 	--output $snippets/local_er \
# 	--repeats 1

# [ONGOING]

# P50
echo "Running local_p50..."
$pwd/scripts/local/run_all.sh \
	--features prio_local \
	--output $snippets/local_p50 \
	--repeats 1

echo "Running local_er_p50..."
$pwd/scripts/local/run_all.sh \
	--features prio_local_early \
	--output $snippets/local_er_p50 \
	--repeats 1

# # P75
# echo "Running local_p75..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local \
# 	--output $snippets/local_p75 \
# 	--repeats 1

# echo "Running local_er_p75..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local_early \
# 	--output $snippets/local_er_p75 \
# 	--repeats 1

# # P90
# echo "Running local_p90..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local \
# 	--output $snippets/local_p90 \
# 	--repeats 1

# echo "Running local_er_p90..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local_early \
# 	--output $snippets/local_er_p90 \
# 	--repeats 1

# # P95
# echo "Running local_p95..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local \
# 	--output $snippets/local_p95 \
# 	--repeats 1

# echo "Running local_er_p95..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local_early \
# 	--output $snippets/local_er_p95 \
# 	--repeats 1
