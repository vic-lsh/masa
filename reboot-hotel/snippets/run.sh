#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

echo "Running fifo_infra..."
$pwd/scripts/local/run_all.sh \
	--features fifo_infra \
	--repeats 1

echo "Running prio_global..."
$pwd/scripts/local/run_all.sh \
	--features prio_global \
	--repeats 1

echo "Running prio_global_early..."
$pwd/scripts/local/run_all.sh \
	--features prio_global_early \
	--repeats 1
