#!/bin/bash

# change work dir 
cd ../..
./scripts/docker-stop.sh
./scripts/docker-run.sh --features fifo
./scripts/loadgen-run.sh

cd data/lat_out/rps3000/fifo

docker cp hotel_frontend:/HandleReservation_latencies.log .
docker cp hotel_user:/CheckUser_latencies.log .
docker cp hotel_reservation:/MakeReservation_latencies.log .