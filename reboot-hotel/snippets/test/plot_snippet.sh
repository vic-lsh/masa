#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi
folder=snippets/test

echo "Plotting fifo..."
$pwd/$folder/fifo/plot.sh

# echo "Plotting e2e..."
# $pwd/$folder/e2e/plot.sh

# echo "Plotting e2e_er..."
# $pwd/$folder/e2e_er/plot.sh

echo "Plotting local..."
$pwd/$folder/local/plot.sh

# echo "Plotting local_er..."
# $pwd/$folder/local_er/plot.sh

echo "Plotting cmp..."
$pwd/$folder/cmp/plot.sh
