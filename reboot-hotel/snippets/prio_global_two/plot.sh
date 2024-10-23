reboot_hotel=~/Masa-Lo-Ding/reboot-hotel
mode=prio_global_two

python $reboot_hotel/scripts/plots/plot_goodput.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Goodput done"

python $reboot_hotel/scripts/plots/plot_throughput.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Throughput done"

python $reboot_hotel/scripts/plots/plot_error.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Error done"
