#!/bin/bash

set -e

repo_root=$(git rev-parse --show-toplevel)
pwd=$(pwd)

case "$pwd" in
    *"/exp/"*)
        app=$(basename "$pwd")
        ;;
    *"/apps/"*)
        app=$(basename "$pwd")
        ;;
    *)
        echo "Error: please run this script from within exp/<app> or apps/<app>" >&2
        exit 1
        ;;
esac

if [[ -z "$1" ]]; then
    datapath="data"
else
    datapath="data/$1"
fi

local_target="$repo_root/exp/$app/$datapath"
mkdir -p "$local_target"

if [[ ! -f "$repo_root/.env" ]]; then
    echo "Error: expected credentials in $repo_root/.env" >&2
    exit 1
fi

# shellcheck disable=SC1090
source "$repo_root/.env"

rsync -av "${remote_user}@${server}:${remote_masa_path}/exp/${app}/${datapath}/" "$local_target/"
