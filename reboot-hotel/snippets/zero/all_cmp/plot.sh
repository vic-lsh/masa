#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

declare -A configs=(
	["fifo_infra_vs_prio_global"]="fifo_infra prio_global"
	["fifo_infra_vs_prio_global_early"]="fifo_infra prio_global_early"
	["prio_global_vs_prio_global_early"]="prio_global prio_global_early"
)

for mode in "${!configs[@]}"; do
	modes_cmp="${configs[$mode]}"

	echo "Plotting goodput for $mode..."
	python3 $pwd/scripts/plots/plot_goodput_cmp.py \
		--path $pwd/snippets/all_cmp \
		--snippets $pwd/snippets \
		--modes $modes_cmp

	# echo "Plotting tail..."
	# python3 $pwd/scripts/plots/plot_tail_cmp.py \
	#     --path $pwd/snippets/$mode \
	#     --snippets $pwd/snippets \
	#     --modes $modes_cmp
done
