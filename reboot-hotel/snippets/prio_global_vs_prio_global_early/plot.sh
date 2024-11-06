#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

mode=prio_global_vs_prio_global_early
modes_cmp="prio_global prio_global_early"

echo "Plotting goodput..."
python3 $pwd/scripts/plots/plot_goodput_cmp.py \
	--path $pwd/snippets/$mode \
	--snippets $pwd/snippets \
	--modes $modes_cmp

echo "Plotting tail..."
python3 $pwd/scripts/plots/plot_tail_cmp.py \
	--path $pwd/snippets/$mode \
	--snippets $pwd/snippets \
	--modes $modes_cmp
