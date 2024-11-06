#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

echo "Plotting prio_global..."
$pwd/snippets/prio_global/plot.sh

echo "Plotting prio_global_early..."
$pwd/snippets/prio_global_early/plot.sh

echo "Plotting prio_global_vs_prio_global_early..."
$pwd/snippets/prio_global_vs_prio_global_early/plot.sh
