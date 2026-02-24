#!/bin/bash

kind_node_image_ulimit() {
    local repo_root="$1"
    local base_image="${KIND_NODE_IMAGE:-kindest/node:v1.32.0}"
    local tag_suffix
    tag_suffix="$(echo "$base_image" | tr '/:@' '-')"
    local image_tag="masa/kind-node-ulimit:${tag_suffix}"

    if ! docker image inspect "$image_tag" >/dev/null 2>&1; then
        docker build \
            -f "$repo_root/scripts/kind-node.Dockerfile" \
            --build-arg "BASE_IMAGE=$base_image" \
            -t "$image_tag" \
            "$repo_root/scripts"
    fi

    printf '%s\n' "$image_tag"
}
