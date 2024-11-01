pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

echo "Plotting prio_global..."
$pwd/prio_global/plot.sh

echo "Plotting prio_global_two..."
$pwd/prio_global_two/plot.sh
