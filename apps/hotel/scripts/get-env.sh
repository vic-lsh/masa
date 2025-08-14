FRONTEND_PORT=$(cat ./scripts/gen_config.json | jq -r ".Addr" | sed -r 's/.*:([0-9]+)$/\1/')
echo "FRONTEND_PORT=$FRONTEND_PORT"
RATE_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".RateReplicas // 1")
echo "RATE_REPLICAS=$RATE_REPLICAS"
PROFILE_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".ProfileReplicas // 1")
echo "PROFILE_REPLICAS=$PROFILE_REPLICAS"
RESERVATION_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".ReservationReplicas // 1")
echo "RESERVATION_REPLICAS=$RESERVATION_REPLICAS"
GEO_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".GeoReplicas // 1")
echo "GEO_REPLICAS=$GEO_REPLICAS"
SEARCH_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".SearchReplicas // 1")
echo "SEARCH_REPLICAS=$SEARCH_REPLICAS"
USER_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".UserReplicas // 1")
echo "USER_REPLICAS=$USER_REPLICAS"

