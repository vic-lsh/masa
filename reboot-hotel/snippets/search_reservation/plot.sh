#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi
snippets=snippets/search_reservation

# echo "Plotting fifo..."
# $pwd/$snippets/fifo/plot.sh

# echo "Plotting e2e..."
# $pwd/$snippets/e2e/plot.sh

# echo "Plotting e2e_er..."
# $pwd/$snippets/e2e_er/plot.sh

# echo "Plotting local..."
# $pwd/$snippets/local/plot.sh

# echo "Plotting local_er..."
# $pwd/$snippets/local_er/plot.sh

# echo "Plotting cmp..."
# $pwd/$snippets/cmp/plot.sh

# [ONGOING]

echo "Plotting local_p50..."
$pwd/$snippets/local_p50/plot.sh

echo "Plotting local_er_p50..."
$pwd/$snippets/local_er_p50/plot.sh

echo "Plotting local_p90..."
$pwd/$snippets/local_p90/plot.sh

echo "Plotting local_er_p90..."
$pwd/$snippets/local_er_p90/plot.sh

echo "Plotting local_p95..."
$pwd/$snippets/local_p95/plot.sh

echo "Plotting local_er_p95..."
$pwd/$snippets/local_er_p95/plot.sh
