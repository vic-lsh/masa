#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

folder=snippets/test
snippet=fifo
mode=fifo
data=.

echo "Plotting goodput..."
python3 $pwd/scripts/plots/plot_goodput.py \
	--mode $mode \
	--path $pwd/$folder/$snippet/$data \
