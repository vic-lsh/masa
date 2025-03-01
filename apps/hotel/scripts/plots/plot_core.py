import argparse
import os
from typing import *

import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import numpy as np
from matplotlib import font_manager

font_paths = ["/usr/share/fonts/truetype/roboto"]
font_files = font_manager.findSystemFonts(fontpaths=font_paths)
for font_file in font_files:
    font_manager.fontManager.addfont(font_file)
plt.rcParams["font.family"] = "Roboto"
fontsize_large = 17
fontsize_medium = 13
fontsize_small = 11
fontsize_tiny = 9
plt.rcParams.update(
    {
        "font.size": fontsize_large,
        "axes.labelsize": fontsize_large,
        "axes.titlesize": fontsize_large,
        "xtick.labelsize": fontsize_medium,
        "ytick.labelsize": fontsize_medium,
        "legend.fontsize": fontsize_large,
    }
)

COLORS = ["tab:blue", "tab:orange", "tab:purple", "tab:red", "tab:green"]
MARKERS = ["o", "x", "^", "*", "s"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", type=str)
    parser.add_argument("--modes", type=str, nargs="+")
    parser.add_argument("--gen-config", type=str)
    parser.add_argument("--snippets", type=str)
    parser.add_argument("--path", type=str)
    parser.add_argument("--data", type=str)
    args = parser.parse_args()
    return args


def autolabel(ax, rects):
    for rect in rects:
        height = rect.get_height()
        ax.annotate(
            "{}".format(height),
            xy=(rect.get_x() + rect.get_width() / 2, height),
            xytext=(0, 3),
            textcoords="offset points",
            ha="center",
            va="bottom",
            fontsize=fontsize_medium,
        )


def plot_goodput_bar(
    apis: List[str], mode: str, results: List[Dict[str, Any]], fig_name: str
):
    label = (mode + " " + " ".join(apis)).lower()
    rps_values = [result["rps"] for result in results]
    goodputs = [result["goodput_load_gen"] for result in results]

    x = np.arange(len(rps_values))
    width = 0.3

    fig, ax = plt.subplots(figsize=(10, 6))
    rects = ax.bar(
        x,
        goodputs,
        width,
        color=COLORS[0],
        label=label,
        edgecolor="black",
        linewidth=1,
    )

    ax.set_xlabel("RPS")
    ax.set_ylabel("Goodput")
    ax.set_title(f"Goodput ({label})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    fig.tight_layout()
    y_max = max(goodputs)
    y_max = (y_max // 500 + 1) * 500
    plt.ylim(0, y_max)
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_goodput_apis_bar(
    apis: List[str], mode: str, results: List[Dict[str, Any]], fig_name: str
):
    if len(apis) != 2:
        return
    labels = [(f"{mode} {api}").lower() for api in apis]
    rps_values = [result["rps"] for result in results if result["api"] == apis[0]]
    goodputs = [
        [result["goodput_load_gen"] for result in results if result["api"] == api]
        for api in apis
    ]

    fig, ax = plt.subplots(figsize=(10, 6))

    x = np.arange(len(rps_values))
    width = 0.3
    rects1 = ax.bar(
        x - width * 0.5,
        goodputs[0],
        width,
        color=COLORS[0],
        label=labels[0],
        edgecolor="black",
        linewidth=1,
    )
    rects2 = ax.bar(
        x + width * 0.5,
        goodputs[1],
        width,
        color=COLORS[1],
        label=labels[1],
        edgecolor="black",
        linewidth=1,
    )

    ax.set_xlabel("RPS")
    ax.set_ylabel("Goodput")
    ax.set_title(f"Goodput ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    fig.tight_layout()
    y_max = max([max(values) for values in goodputs])
    y_max = (y_max // 500 + 1) * 500
    plt.ylim(0, y_max)
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_goodput_cmp_bar(
    modes: List[str], api: str, results: List[Dict[str, Any]], fig_name: str
):
    assert len(modes) <= 6
    modes_str = f"{', '.join(modes)}"
    labels = modes

    rps_values = [result["rps"] for result in results if result["mode"] == modes[0]]
    goodputs: List[List[int]] = []
    for mode in modes:
        goodputs.append(
            [result["goodput"] for result in results if result["mode"] == mode]
        )

    fig, ax = plt.subplots(figsize=(10, 6))

    x = np.arange(len(rps_values))
    bars: List = []
    xs: List = []
    colors: List = []
    hatches: List = []
    if len(modes) == 2:
        width = 0.3
        xs = [x - width * 0.5, x + width * 0.5]
        colors = COLORS
        hatches = ["", ""]
    elif len(modes) == 3:
        width = 0.2
        xs = [x - width, x, x + width]
        colors = COLORS
        hatches = ["", "", ""]
    elif len(modes) == 4:
        width = 0.15
        xs = [x - width * 1.5, x - width * 0.5, x + width * 0.5, x + width * 1.5]
        colors = COLORS
        hatches = ["", "", "", ""]
    elif len(modes) == 5:
        width = 0.12
        xs = [x - width * 2, x - width, x, x + width, x + width * 2]
        colors = [COLORS[0], COLORS[1], COLORS[1], COLORS[2], COLORS[2]]
        hatches = ["", "", "///", "", "///"]
    elif len(modes) == 6:
        width = 0.10
        xs = [x - width * 2, x - width, x, x + width, x + width * 2, x + width * 3]
        colors = [COLORS[0], COLORS[0], COLORS[1], COLORS[1], COLORS[2], COLORS[2]]
        hatches = ["", "///", "", "///", "", "///"]
    else:
        raise ValueError(f"Invalid number of modes: {len(modes)}")

    for i in range(len(modes)):
        bars.append(
            ax.bar(
                xs[i],
                goodputs[i],
                width,
                label=labels[i],
                color=colors[i],
                hatch=hatches[i],
                edgecolor="black",
                linewidth=1,
            )
        )

    ax.set_xlabel("RPS")
    ax.set_ylabel("Goodput")
    ax.set_title(f"Goodput {api} ({modes_str})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    fig.tight_layout()
    y_max = max([max(values) for values in goodputs])
    y_max = (y_max // 500 + 1) * 500
    plt.ylim(0, y_max)
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_time_bar(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = ["good_total", "err_svc_er_total", "err_cl_miss_total"]
    labels = ["Good Total", "Error Svc Early Return Total", "Error Client Miss Total"]
    errors = [[result[key] for result in results] for key in keys]

    fig, ax = plt.subplots(figsize=(10, 6))

    x = np.arange(len(rps_values))
    xs: List = []
    if len(keys) == 2:
        width = 0.3
        xs = [x - width * 0.5, x + width * 0.5]
    elif len(keys) == 3:
        width = 0.2
        xs = [x - width, x, x + width]
    elif len(keys) == 4:
        width = 0.15
        xs = [x - width * 1.5, x - width * 0.5, x + width * 0.5, x + width * 1.5]
    elif len(keys) == 5:
        width = 0.12
        xs = [x - width * 2, x - width, x, x + width, x + width * 2]

    bars: List = []
    for i in range(len(keys)):
        bars.append(
            ax.bar(
                xs[i],
                errors[i],
                width,
                label=labels[i],
                color=COLORS[i],
                edgecolor="black",
                linewidth=1,
            )
        )

    ax.set_xlabel("RPS")
    ax.set_ylabel("Time (ms)")
    ax.set_title(f"Time ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_small)

    fig.tight_layout()
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_tail_bar(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = ["mean", "p50", "p90", "p95", "p99"]
    labels = ["Mean", "P50", "P90", "P95", "P99"]
    errors = [[result[key] for result in results] for key in keys]

    fig, ax = plt.subplots(figsize=(10, 6))

    x = np.arange(len(rps_values))
    width = 0.16
    xs = [x - width * 2, x - width * 1, x, x + width, x + width * 2]
    bars: List = []

    for i in range(5):
        bars.append(
            ax.bar(
                xs[i],
                errors[i],
                width,
                label=labels[i],
                color=COLORS[i],
                edgecolor="black",
                linewidth=1,
            )
        )

    ax.set_xlabel("RPS")
    ax.set_ylabel("Tail (ms)")
    ax.set_ylim(0, 100)
    ax.set_title(f"Tail ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_small)

    fig.tight_layout()
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_tail_cmp_bar(
    modes: List[str], tail: str, api: str, results: List[Dict[str, Any]], fig_name: str
):
    assert len(modes) <= 6
    modes_str = f"{', '.join(modes)}"
    keys = [f"{tail}_{mode}" for mode in modes]
    labels = [f"{tail} {mode}" for mode in modes]

    rps_values = [result["rps"] for result in results if keys[0] in result]
    tails: List[List[int]] = []
    for key in keys:
        tails.append([result[key] for result in results if key in result])

    fig, ax = plt.subplots(figsize=(10, 6))

    x = np.arange(len(rps_values))
    bars: List = []
    xs: List = []
    colors: List = []
    hatches: List = []
    if len(modes) == 2:
        width = 0.3
        xs = [x - width * 0.5, x + width * 0.5]
        colors = COLORS
        hatches = ["", ""]
    elif len(modes) == 3:
        width = 0.2
        xs = [x - width, x, x + width]
        colors = COLORS
        hatches = ["", "", ""]
    elif len(modes) == 4:
        width = 0.15
        xs = [x - width * 1.5, x - width * 0.5, x + width * 0.5, x + width * 1.5]
        colors = COLORS
        hatches = ["", "", "", ""]
    elif len(modes) == 5:
        width = 0.12
        xs = [x - width * 2, x - width, x, x + width, x + width * 2]
        colors = [COLORS[0], COLORS[1], COLORS[1], COLORS[2], COLORS[2]]
        hatches = ["", "", "///", "", "///"]
    elif len(modes) == 6:
        width = 0.10
        xs = [x - width * 2, x - width, x, x + width, x + width * 2, x + width * 3]
        colors = [COLORS[0], COLORS[0], COLORS[1], COLORS[1], COLORS[2], COLORS[2]]
        hatches = ["", "///", "", "///", "", "///"]
    else:
        raise ValueError(f"Invalid number of modes: {len(modes)}")

    for i in range(len(modes)):
        bars.append(
            ax.bar(
                xs[i],
                tails[i],
                width,
                label=labels[i],
                color=colors[i],
                hatch=hatches[i],
                edgecolor="black",
                linewidth=1,
            )
        )

    ax.set_xlabel("RPS")
    ax.set_ylabel(tail)
    ax.set_ylim(0, 100)
    ax.set_title(f"{tail} {api} ({modes_str})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    fig.tight_layout()
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_throughput_bar(mode: str, results: List[Dict[str, Any]], fig_name: str):
    rps_values = [result["rps"] for result in results]
    keys = ["goodput", "lg_miss", "lg_timeout", "early_return", "unknown"]
    labels = [
        "Goodput",
        "LG Miss",
        "LG Timeout",
        "Early Return",
        "Unknown",
    ]
    throughputs = [[result[key] for result in results] for key in keys]

    fig, ax = plt.subplots(figsize=(10, 6))

    x = np.arange(len(rps_values))
    width = 0.16
    xs = [x - width * 2, x - width, x, x + width, x + width * 2]
    bars: List = []

    for i in range(5):
        bars.append(
            ax.bar(
                xs[i],
                throughputs[i],
                width,
                label=labels[i],
                color=COLORS[i],
                edgecolor="black",
                linewidth=1,
            )
        )

    ax.set_xlabel("RPS")
    ax.set_ylabel("Throughput")
    ax.set_title(f"Throughput ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    fig.tight_layout()
    y_max = max([max(values) for values in throughputs])
    y_max = (y_max // 500 + 1) * 500
    plt.ylim(0, y_max)
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)


def plot_error_bar(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = [
        "error_lg_miss",
        "error_lg_timeout",
        "error_frontend",
        "error_search",
        "error_profile",
    ]
    labels = [
        "Error LG Miss",
        "Error LG Timeout",
        "Error Frontend",
        "Error Search",
        "Error Profile",
    ]
    errors = [[result[key] for result in results] for key in keys]

    x = np.arange(len(rps_values))
    width = 0.17
    rects_x = [
        x - width * 2,
        x - width,
        x,
        x + width,
        x + width * 2,
    ]

    fig, ax = plt.subplots(figsize=(10, 6))
    rects_bar = [
        ax.bar(
            rect_x,
            errors[i],
            width,
            label=labels[i],
            color=COLORS[i],
        )
        for i, rect_x in enumerate(rects_x)
    ]

    ax.set_xlabel("RPS")
    ax.set_ylabel("Error")
    ax.set_title(f"Error vs RPS ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_small)

    fig.tight_layout()
    y_max = max([max(values) for values in errors])
    y_max = (y_max // 500 + 1) * 500
    plt.ylim(0, y_max)
    plt.grid(True, axis="y", linewidth=0.5)
    os.makedirs(os.path.dirname(fig_name), exist_ok=True)
    plt.savefig(fig_name)
