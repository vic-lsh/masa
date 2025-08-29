#!/bin/bash

# Set rps and feature 
RPS=5000
FEATURE=fifo

# change work dir 
cd ../..
./scripts/docker-stop.sh
./scripts/docker-run.sh --features $FEATURE
./scripts/loadgen-run.sh --output ./data/lat_out/rps$RPS/$FEATURE --save-logs

cd data/lat_out/rps$RPS/$FEATURE

docker cp hotel_frontend:/HandleReservation_latencies.log .
docker cp hotel_user:/CheckUser_latencies.log .
docker cp hotel_reservation:/MakeReservation_latencies.log .