python ~/Masa-Lo-Ding/reboot-hotel/scripts/plots/plot_goodput.py \
	--path ~/Masa-Lo-Ding/reboot-hotel/snippets/prio_global_two
echo "Goodput done"

python ~/Masa-Lo-Ding/reboot-hotel/scripts/plots/plot_throughput.py \
	--path ~/Masa-Lo-Ding/reboot-hotel/snippets/prio_global_two
echo "Throughput done"

python ~/Masa-Lo-Ding/reboot-hotel/scripts/plots/plot_error.py \
	--path ~/Masa-Lo-Ding/reboot-hotel/snippets/prio_global_two
echo "Error done"
