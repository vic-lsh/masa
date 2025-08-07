FRONTEND_PORT=$(cat ./scripts/gen_config.json | jq -r ".Addr" | sed -r 's/.*:([0-9]+)$/\1/')
echo "FRONTEND_PORT=$FRONTEND_PORT"
CONSTANT_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".child_constant_replicas")
PRESAMPLED_REPLICAS=$(cat ./scripts/local/config.docker.json | jq -r ".child_presampled_services | add")
RANDOM_REPLICAS="1"
CHILD_REPLICAS=$((CONSTANT_REPLICAS + RANDOM_REPLICAS + PRESAMPLED_REPLICAS))
echo "CHILD_REPLICAS=$CHILD_REPLICAS"
