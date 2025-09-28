#!/bin/bash

 for image in hotel_frontend hotel_geo hotel_profile hotel_rate hotel_reservation hotel_search hotel_user; do
     kind load docker-image --name hotel ${image}:latest
 done
