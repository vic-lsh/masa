reboot_hotel=~/Masa-Lo-Ding/reboot-hotel
snippets=snippets/search_reservation
path=e2e_er
data=tmp_1112
mode=e2e_er

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
