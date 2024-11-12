reboot_hotel=~/Masa-Lo-Ding/reboot-hotel
snippets=snippets/single
mode=local_er
path=local_er_search
data=tmp_1112

echo "Plotting goodput..."
python3 $reboot_hotel/scripts/plots/plot_goodput.py \
	--mode $mode \
	--path $reboot_hotel/$snippets/$path \
	--data $data

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
