#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi
snippets=snippets/single

echo "Plotting fifo_search..."
$pwd/$snippets/fifo_search/plot.sh

echo "Plotting fifo_reservation..."
$pwd/$snippets/fifo_reservation/plot.sh

echo "Plotting e2e_search..."
$pwd/$snippets/e2e_search/plot.sh

echo "Plotting e2e_reservation..."
$pwd/$snippets/e2e_reservation/plot.sh

echo "Plotting e2e_er_search..."
$pwd/$snippets/e2e_er_search/plot.sh

echo "Plotting e2e_er_reservation..."
$pwd/$snippets/e2e_er_reservation/plot.sh

echo "Plotting local_search..."
$pwd/$snippets/local_search/plot.sh

echo "Plotting local_reservation..."
$pwd/$snippets/local_reservation/plot.sh

echo "Plotting local_er_search..."
$pwd/$snippets/local_er_search/plot.sh

echo "Plotting local_er_reservation..."
$pwd/$snippets/local_er_reservation/plot.sh

echo "Plotting cmp..."
$pwd/$snippets/cmp/plot.sh
