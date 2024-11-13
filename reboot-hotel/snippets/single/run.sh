#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/single

# echo "Running fifo_search..."
# $pwd/scripts/local/run_all.sh \
# 	--features fifo \
# 	--output $snippets/fifo_search \
# 	--repeats 1

# echo "Running fifo_reservation..."
# $pwd/scripts/local/run_all.sh \
# 	--features fifo \
# 	--output $snippets/fifo_reservation \
# 	--repeats 1

# echo "Running e2e_search..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_global \
# 	--output $snippets/e2e_search \
# 	--repeats 1

# echo "Running e2e_reservation..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_global \
# 	--output $snippets/e2e_reservation \
# 	--repeats 1

# echo "Running e2e_er_search..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_global_early \
# 	--output $snippets/e2e_er_search \
# 	--repeats 1

# echo "Running e2e_er_reservation..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_global_early \
# 	--output $snippets/e2e_er_reservation \
# 	--repeats 1

# echo "Running local_search..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local \
# 	--output $snippets/local_search \
# 	--repeats 1

# echo "Running local_reservation..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local \
# 	--output $snippets/local_reservation \
# 	--repeats 1

# echo "Running local_er_search..."
# $pwd/scripts/local/run_all.sh \
# 	--features prio_local_early \
# 	--output $snippets/local_er_search \
# 	--repeats 1

echo "Running local_er_reservation..."
$pwd/scripts/local/run_all.sh \
	--features prio_local_early \
	--output $snippets/local_er_reservation \
	--repeats 1
