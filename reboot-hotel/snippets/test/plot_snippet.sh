#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi
folder=snippets/test
data=tmp_1118

echo "Plotting fifo..."
$pwd/$folder/fifo/plot.sh \
    --data $data

echo "Plotting local..."
$pwd/$folder/local/plot.sh \
    --data $data

echo "Plotting cmp..."
$pwd/$folder/cmp/plot.sh \
    --data $data
