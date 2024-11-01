reboot=~/Masa-Lo-Ding/reboot-hotel

# echo "Running prio_global..."
# $reboot/scripts/local/run_all.sh \
# 	--features prio_global \
# 	--repeats 1

echo "Running prio_global_two..."
$reboot/scripts/local/run_all.sh \
	--features prio_global_two \
	--repeats 1
