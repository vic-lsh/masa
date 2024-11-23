#!/bin/bash

set -e

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

services=(
    "hotel_frontend"
    "hotel_geo"
    "hotel_rate"
    "hotel_search"
    "hotel_profile"
    "hotel_reservation"
    "hotel_user"
    "recommendation-server"
)

for svc in "${services[@]}"; do
    ./scripts/docker-build.sh --binary "$svc"
done
