pwd=$(pwd)
if [[ "$pwd" != */apps/* ]]; then
    echo "Error: please run in an application directory" >&2
    exit 1
fi
app=$(basename $pwd)

source ../.env

rsync -av ${remote_user}@${server}:${remote_masa_path}/apps/${app}/data/ data
