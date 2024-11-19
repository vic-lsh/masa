#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/single/e2e_reservation
mode=e2e
data=tmp_1112

echo "Plotting goodput..."
python3 $pwd/scripts/plots/plot_goodput.py \
	--mode $mode \
	--path $pwd/$snippets/$data
