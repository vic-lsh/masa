reboot_hotel=~/projs/masa/reboot-hotel
mode=prio_global

python3 $reboot_hotel/scripts/plots/plot_goodput.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Goodput done"

python3 $reboot_hotel/scripts/plots/plot_throughput.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Throughput done"

python3 $reboot_hotel/scripts/plots/plot_error.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Error done"

python3 $reboot_hotel/scripts/plots/plot_tail.py \
	--path $reboot_hotel/snippets/$mode \
	--mode $mode
echo "Tail done"
