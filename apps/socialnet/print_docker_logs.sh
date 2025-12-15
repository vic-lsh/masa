#!/bin/bash

# 1. We filter directly for the network here. 
# We REMOVED '-a' so it only grabs running containers.
CONTAINERS=$(docker ps -q --filter "network=socialnet_socialnet-network")

if [ -z "$CONTAINERS" ]; then
    echo "No running containers found on network 'socialnet_socialnet-network'."
else
    for container in $CONTAINERS; do
        # Get the clean name
        NAME=$(docker inspect --format='{{.Name}}' $container | sed 's/\///')
        
        # 2. The Exclusion: Skip if name ends in "_mongo"
        if [[ "$NAME" == *"_mongo" ]]; then
            continue
        fi

        if [[ "$NAME" == *"rabbitmq" ]]; then
            continue
        fi
        
        echo "----------------------------------------------------"
        echo "LOGS FOR: $NAME"
        echo "----------------------------------------------------"
        
        # 3. Print logs with timestamps
        # docker logs --timestamps $container
        docker logs $container

        echo -e "\n"
    done
fi
