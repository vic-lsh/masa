#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
	echo "Error: plese run in the reboot-hotel directory" >&2
	exit 1
fi

folder=snippets/test
snippet=local
mode=local
data=""
while [[ "$#" -gt 0 ]]; do
	case $1 in
	--data)
		data="$2"
		shift
		;;
	*)
		echo "Unknown parameter passed: $1"
		exit 1
		;;
	esac
	shift
done
if [ -z "$data" ]; then
	echo "Expected a data folder using --data"
	exit 1
fi

echo "Plotting goodput..."
python3 $pwd/scripts/plots/plot_goodput.py \
	--mode $mode \
	--gen-config $pwd/$folder/gen_config.json \
	--path $pwd/$folder/$snippet/$data

echo "Plotting tail..."
python3 $pwd/scripts/plots/plot_tail.py \
	--mode $mode \
	--gen-config $pwd/$folder/gen_config.json \
	--path $pwd/$folder/$snippet/$data
