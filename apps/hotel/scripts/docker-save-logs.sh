#!/bin/bash

# Script to write docker container logs for each service to a file

# Define the image names and a single tag
declare -a image_names=(
  "hotel_frontend"
  "hotel_search"
  "hotel_profile"
  "hotel_rate" 
  "hotel_reservation"
  "hotel_user"
)

IMAGE_TAG="latest"

for ((i=0; i<${#image_names[@]}; i++)); do
  service=${image_names[$i]}
  
  # Get the container ID for the current image
  current_image="${image_names[$i]}:${IMAGE_TAG}"
  container_id=$(docker ps --filter "ancestor=${current_image}" --format "{{.ID}}")
  
  # Send the command to follow logs
  if [ -n "$container_id" ]; then
    docker logs $container_id &> $1/$service.log
  else
    echo "Container with image ${image_names[$i]}:${IMAGE_TAG} not found"
  fi
done
