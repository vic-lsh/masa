reboot_hotel=~/Masa-Lo-Ding/reboot-hotel
snippets=snippets/single
path=e2e_search
data=tmp_1111
mode=e2e

echo "Plotting goodput..."
python3 $reboot_hotel/scripts/plots/plot_goodput.py \
	--path $reboot_hotel/$snippets/$path \
	--data $data \
	--mode $mode

# [TODO] Change $mode to $path.

# echo "Plotting throughput..."
# python3 $reboot_hotel/scripts/plots/plot_throughput.py \
# 	--path $reboot_hotel/$snippets/$mode \
# 	--mode $mode

# echo "Plotting error..."
# python3 $reboot_hotel/scripts/plots/plot_error.py \
# 	--path $reboot_hotel/$snippets/$mode \
# 	--mode $mode

# echo "Plotting tail..."
# python3 $reboot_hotel/scripts/plots/plot_tail.py \
# 	--path $reboot_hotel/$snippets/$mode \
# 	--mode $mode
