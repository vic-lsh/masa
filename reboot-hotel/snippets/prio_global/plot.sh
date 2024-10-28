reboot_hotel=~/Masa-Lo-Ding/reboot-hotel
mode=prio_global

python $reboot_hotel/scripts/plots/plot_goodput.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode \
	--alias no_early_return
echo "Goodput done"

python $reboot_hotel/scripts/plots/plot_throughput.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode \
	--alias no_early_return
echo "Throughput done"

python $reboot_hotel/scripts/plots/plot_error.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode \
	--alias no_early_return
echo "Error done"

python $reboot_hotel/scripts/plots/plot_tail.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode \
	--alias no_early_return
echo "Tail done"
