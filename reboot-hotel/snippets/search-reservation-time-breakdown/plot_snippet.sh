#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

folder=snippets/search-reservation-time-breakdown
data=tmp_1203_gcp

plot_single() {
    local folder="$1"
    local mode="$2"
    local snippet="$3"
    local data="$4"

    # echo "Plotting goodput..."
    # python3 $pwd/scripts/plots/plot_goodput.py \
    #     --mode $mode \
    #     --gen-config $pwd/$folder/gen_config.json \
    #     --path $pwd/$folder/$snippet/$data

    echo "Plotting time..."
    python3 $pwd/scripts/plots/plot_time.py \
        --mode $mode \
        --gen-config $pwd/$folder/gen_config.json \
        --path $pwd/$folder/$snippet/$data

    # echo "Plotting tail..."
    # python3 $pwd/scripts/plots/plot_tail.py \
    #     --mode $mode \
    #     --gen-config $pwd/$folder/gen_config.json \
    #     --path $pwd/$folder/$snippet/$data

    # echo "Plotting throughput..."
    # python3 $pwd/scripts/plots/plot_throughput.py \
    #     --mode $mode \
    #     --gen-config $pwd/$folder/gen_config.json \
    #     --path $pwd/$folder/$snippet/$data
}

plot_multi() {
    local folder="$1"
    local modes="$2"
    local snippet="cmp"
    local data="$3"

    mkdir -p $pwd/$folder/$snippet

    echo "Plotting goodput..."
    python3 $pwd/scripts/plots/plot_goodput_cmp.py \
        --modes $modes \
        --gen-config $pwd/$folder/gen_config.json \
        --path $pwd/$folder/$snippet \
        --snippets $pwd/$folder \
        --data $data

    echo "Plotting tail..."
    python3 $pwd/scripts/plots/plot_tail_cmp.py \
        --modes $modes \
        --gen-config $pwd/$folder/gen_config.json \
        --path $pwd/$folder/$snippet \
        --snippets $pwd/$folder \
        --data $data
}

# echo "Analyzing fifo..."
# plot_single $folder fifo fifo $data
# echo ""

# echo "Analyzing e2e..."
# plot_single $folder e2e e2e $data
# echo ""

# echo "Analyzing e2e_er..."
# plot_single $folder e2e_er e2e_er $data
# echo ""

# echo "Analyzing local..."
# plot_single $folder local local $data
# echo ""

echo "Analyzing local_er..."
plot_single $folder local_er local_er $data
echo ""

modes_list=(
    # "fifo e2e_er local_er"
    # "fifo e2e e2e_er local local_er"
)

for modes in "${modes_list[@]}"; do
    echo "Analyzing $modes..."
    plot_multi $folder "$modes" $data
done
