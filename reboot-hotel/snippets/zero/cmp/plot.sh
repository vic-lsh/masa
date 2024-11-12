#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/zero

declare -A configs=(
	["fifo_infra_vs_prio_global"]="fifo_infra prio_global"
	["fifo_infra_vs_prio_global_early"]="fifo_infra prio_global_early"
	["prio_global_vs_prio_global_early"]="prio_global prio_global_early"
)

for name in "${!configs[@]}"; do
	modes="${configs[$name]}"

	echo "Plotting goodput for $name..."
	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--path $pwd/$snippets/all_cmp \
		--snippets $pwd/$snippets \
		--modes $modes
done
