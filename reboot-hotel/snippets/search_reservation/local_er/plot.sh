#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi
snippets=snippets/search_reservation/local_er
mode=local_er
data=tmp_1112

echo "Plotting goodput..."
python3 $pwd/scripts/plots/plot_goodput.py \
	--mode $mode \
	--path $pwd/$snippets/$data

# [TODO]

# echo "Plotting throughput..."
# python3 $pwd/scripts/plots/plot_throughput.py \
# 	--path $pwd/$snippets/$mode \
# 	--mode $mode

# echo "Plotting error..."
# python3 $pwd/scripts/plots/plot_error.py \
# 	--path $pwd/$snippets/$mode \
# 	--mode $mode

# echo "Plotting tail..."
# python3 $pwd/scripts/plots/plot_tail.py \
# 	--path $pwd/$snippets/$mode \
# 	--mode $mode
