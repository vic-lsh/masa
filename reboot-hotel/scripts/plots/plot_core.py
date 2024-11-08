import argparse
from typing import *

import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import numpy as np

plt.rcParams["font.family"] = "Roboto"
fontsize = 17
fontsize_medium = 13
fontsize_small = 11
plt.rcParams.update(
    {
        "font.size": fontsize,
        "axes.labelsize": fontsize,
        "axes.titlesize": fontsize,
        "xtick.labelsize": fontsize,
        "ytick.labelsize": fontsize,
        "legend.fontsize": fontsize,
    }
)

GRAPH_IDS = ["Hotel"]
COLORS = ["tab:blue", "tab:orange", "tab:purple", "tab:red", "tab:green"]
MARKERS = ["o", "x", "^", "*", "s"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--path", type=str)
    parser.add_argument("--mode", type=str)
    parser.add_argument("--snippets", type=str)
    parser.add_argument("--modes", type=str, nargs="+")
    args = parser.parse_args()
    return args


def plot_goodput_line(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    goodputs_load_gen = [result["goodput_load_gen"] for result in results]
    # goodputs_fe = [result["goodput_fe"] for result in results]

    fig = plt.figure(figsize=(10, 6))

    plt.plot(
        rps_values,
        goodputs_load_gen,
        label="Goodput Load Gen",
        color=COLORS[0],
        marker=MARKERS[0],
    )
    # plt.plot(
    #     rps_values,
    #     goodputs_fe,
    #     label="Goodput FE",
    #     color=COLORS[1],
    #     marker=MARKERS[1],
    # )

    plt.xlabel("RPS")
    plt.ylabel("Goodput")
    plt.ylim(-200, 2200)
    plt.title(f"Goodput vs RPS ({mode})")
    plt.legend(loc="upper left", fontsize=fontsize_medium)
    plt.grid(True)
    plt.savefig(fig_name)


def plot_goodput_bar(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    goodputs_load_gen = [result["goodput_load_gen"] for result in results]
    # goodputs_fe = [result["goodput_fe"] for result in results]

    x = np.arange(len(rps_values))
    width = 0.3

    fig, ax = plt.subplots(figsize=(10, 6))
    rects1 = ax.bar(x - width * 0.5, goodputs_load_gen, width, label="Goodput Load Gen")
    # rects2 = ax.bar(x + width * 0.5, goodputs_fe, width, label="Goodput FE")

    ax.set_xlabel("RPS")
    ax.set_ylabel("Goodput")
    ax.set_title(f"Goodput vs RPS ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    def autolabel(rects):
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

    autolabel(rects1)
    # autolabel(rects2)

    fig.tight_layout()
    plt.ylim(-200, 2200)
    plt.grid(True, linestyle="--", linewidth=0.5)
    plt.savefig(fig_name)


def plot_goodput_cmp_bar(
    results: List[Dict[str, Any]], fig_name: str, modes: List[str]
):
    modes_str = f"{', '.join(modes)}"
    keys = [f"goodput_{mode}" for mode in modes]
    assert len(keys) == 2, "Only 2 modes available"
    labels = [f"Goodput {mode}" for mode in modes]

    rps_values = [result["rps"] for result in results if keys[0] in result]
    goodputs_lhs = [result[keys[0]] for result in results if keys[0] in result]
    goodputs_rhs = [result[keys[1]] for result in results if keys[1] in result]

    x = np.arange(len(rps_values))
    width = 0.3

    fig, ax = plt.subplots(figsize=(10, 6))
    rects1 = ax.bar(x - width * 0.5, goodputs_lhs, width, label=labels[0])
    rects2 = ax.bar(x + width * 0.5, goodputs_rhs, width, label=labels[1])

    ax.set_xlabel("RPS")
    ax.set_ylabel("Goodput")
    ax.set_title(f"Goodput vs RPS ({modes_str})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    def autolabel(rects):
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

    # autolabel(rects1)
    autolabel(rects2)

    fig.tight_layout()
    plt.ylim(-200, 2200)
    plt.grid(True, linestyle="--", linewidth=0.5)
    plt.savefig(fig_name)


def plot_throughput_line(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = ["tput_good", "tput_lg_miss", "tput_lg_timeout", "tput_svc_early"]
    labels = [
        "Throughput Good",
        "Throughput LG Miss",
        "Throughput LG Timeout",
        "Throughput Service Early",
    ]
    tputs = [[result[key] for result in results] for key in keys]

    fig = plt.figure(figsize=(10, 6))
    for i in range(len(keys)):
        plt.plot(
            rps_values, tputs[i], label=labels[i], color=COLORS[i], marker=MARKERS[i]
        )

    plt.xlabel("RPS")
    plt.ylabel("Throughput")
    plt.ylim(-200, 2200)
    plt.title(f"Throughput vs RPS ({mode})")
    plt.legend(loc="upper left", fontsize=fontsize_medium)
    plt.grid(True)
    plt.savefig(fig_name)


def plot_throughput_bar(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = ["tput_good", "tput_lg_miss", "tput_lg_timeout", "tput_svc_early"]
    labels = [
        "Throughput Good",
        "Throughput LG Miss",
        "Throughput LG Timeout",
        "Throughput Service Early",
    ]
    tputs = [[result[key] for result in results] for key in keys]

    x = np.arange(len(rps_values))
    width = 0.22
    rects_x = [
        x - width * 1.5,
        x - width * 0.5,
        x + width * 0.5,
        x + width * 1.5,
    ]

    fig, ax = plt.subplots(figsize=(10, 6))
    rects_bar = [
        ax.bar(rect_x, tputs[i], width, label=labels[i], color=COLORS[i])
        for i, rect_x in enumerate(rects_x)
    ]

    ax.set_xlabel("RPS")
    ax.set_ylabel("Throughput")
    ax.set_title(f"Throughput vs RPS ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    def autolabel(rects):
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

    for rects in rects_bar:
        autolabel(rects)

    fig.tight_layout()
    plt.ylim(-200, 2200)
    plt.grid(True, linestyle="--", linewidth=0.5)
    plt.savefig(fig_name)


def plot_error_line(results: List[Dict[str, Any]], fig_name: str, mode: str):
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

    fig = plt.figure(figsize=(10, 6))
    for i in range(len(keys)):
        plt.plot(
            rps_values, errors[i], label=labels[i], color=COLORS[i], marker=MARKERS[i]
        )

    plt.xlabel("RPS")
    plt.ylabel("Error")
    plt.ylim(-200, 2200)
    plt.title(f"Error vs RPS ({mode})")
    plt.legend(loc="upper left", fontsize=fontsize_medium)
    plt.grid(True)
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
        ax.bar(rect_x, errors[i], width, label=labels[i], color=COLORS[i])
        for i, rect_x in enumerate(rects_x)
    ]

    ax.set_xlabel("RPS")
    ax.set_ylabel("Error")
    ax.set_title(f"Error vs RPS ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_small)

    def autolabel(rects):
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

    for rects in rects_bar:
        autolabel(rects)

    fig.tight_layout()
    plt.ylim(-200, 2200)
    plt.grid(True, linestyle="--", linewidth=0.5)
    plt.savefig(fig_name)


def plot_tail_line(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = ["mean", "p90", "p95", "p99"]
    labels = ["Mean", "P90", "P95", "P99"]
    errors = [[result[key] for result in results] for key in keys]

    fig = plt.figure(figsize=(10, 6))
    for i in range(len(keys)):
        plt.plot(
            rps_values, errors[i], label=labels[i], color=COLORS[i], marker=MARKERS[i]
        )

    plt.xlabel("RPS")
    plt.ylabel("Tail (ms)")
    plt.ylim(0, 100)
    plt.title(f"Tail vs RPS ({mode})")
    plt.legend(loc="upper left", fontsize=fontsize_medium)
    plt.grid(True)
    plt.savefig(fig_name)


def plot_tail_bar(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    keys = ["mean", "p90", "p95", "p99"]
    labels = ["Mean", "P90", "P95", "P99"]
    errors = [[result[key] for result in results] for key in keys]

    x = np.arange(len(rps_values))
    width = 0.22
    rects_x = [
        x - width * 1.5,
        x - width * 0.5,
        x + width * 0.5,
        x + width * 1.5,
    ]

    fig, ax = plt.subplots(figsize=(10, 6))
    rects_bar = [
        ax.bar(rect_x, errors[i], width, label=labels[i], color=COLORS[i])
        for i, rect_x in enumerate(rects_x)
    ]

    ax.set_xlabel("RPS")
    ax.set_ylabel("Tail (ms)")
    ax.set_ylim(0, 100)
    ax.set_title(f"Tail vs RPS ({mode})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_small)

    def autolabel(rects):
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

    for rects in rects_bar:
        autolabel(rects)

    fig.tight_layout()
    plt.grid(True, linestyle="--", linewidth=0.5)
    plt.savefig(fig_name)


def plot_tail_cmp_bar(
    results: List[Dict[str, Any]], fig_name: str, modes: List[str], tail: str
):
    modes_str = f"{', '.join(modes)}"
    keys = [f"{tail}_{mode}" for mode in modes]
    labels = [f"{tail} {mode}" for mode in modes]
    assert len(keys) == 2, "Only 2 modes available"

    rps_values = [result["rps"] for result in results if keys[0] in result]
    goodputs_lhs = [result[keys[0]] for result in results if keys[0] in result]
    goodputs_rhs = [result[keys[1]] for result in results if keys[1] in result]

    x = np.arange(len(rps_values))
    width = 0.3

    fig, ax = plt.subplots(figsize=(10, 6))
    rects1 = ax.bar(x - width * 0.5, goodputs_lhs, width, label=labels[0])
    rects2 = ax.bar(x + width * 0.5, goodputs_rhs, width, label=labels[1])

    ax.set_xlabel("RPS")
    ax.set_ylabel(tail)
    ax.set_ylim(0, 100)
    ax.set_title(f"{tail} vs RPS ({modes_str})")
    ax.set_xticks(x)
    ax.set_xticklabels(rps_values)
    ax.legend(loc="upper left", fontsize=fontsize_medium)

    def autolabel(rects):
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

    # autolabel(rects1)
    autolabel(rects2)

    fig.tight_layout()
    plt.grid(True, linestyle="--", linewidth=0.5)
    plt.savefig(fig_name)


def plot_goodput_per_rps(result: Dict[str, Any], fig_name: str):
    fig, ax = plt.subplots(figsize=(10, 6))
    ax.plot(result["goodput_per_rps"], color="tab:blue")

    ax.set_title(f"Goodput per RPS", fontsize=fontsize)
    ax.set_xlabel("Time (Seconds)", fontsize=fontsize)
    ax.set_ylabel("Goodput", fontsize=fontsize)

    plt.grid(True, linestyle="--", alpha=0.7)
    plt.savefig(fig_name)


def plot_cdf(results: List[Tuple[str, List[int]]], title: str, fig_name: str):
    assert len(results) <= 3, "Only 3 colors available"

    fig = plt.figure(figsize=(10, 6))

    for i in range(len(results)):
        label, latencies = results[i]
        latencies_ms = [latency / 1e3 for latency in latencies]
        latencies_ms.sort()
        cdf = np.arange(1, len(latencies_ms) + 1) / len(latencies_ms)
        plt.plot(latencies_ms, cdf, label=label, color=COLORS[i])

        # p90 = float(np.percentile(latencies_ms, 90))
        # plt.axvline(
        #     x=p90,
        #     linestyle="--",
        #     label=f"{label} p90: {p90:.2f}ms",
        #     color="forestgreen",
        # )
        # plt.scatter([p90], [0.90], color="forestgreen")
        p95 = float(np.percentile(latencies_ms, 95))
        plt.axvline(
            x=p95,
            linestyle="--",
            label=f"{label} p95: {p95:.2f}ms",
            color="forestgreen",
        )
        plt.scatter([p95], [0.95], color="forestgreen")

    plt.xlabel("Latency (ms)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)


def plot_pdf(results: List[Tuple[str, List[int]]], title: str, fig_name: str):
    assert len(results) <= 3, "Only 3 colors available"

    fig = plt.figure(figsize=(10, 6))

    for i in range(len(results)):
        label, latencies = results[i]
        latencies_ms = [latency / 1e3 for latency in latencies]
        latencies_ms.sort()

        counts, bin_edges = np.histogram(latencies_ms, bins=1000, density=False)
        bin_centers = 0.5 * (bin_edges[:-1] + bin_edges[1:])
        # Calculate sum value S[X] = sum(x * c(x))
        sum_value = np.sum(bin_centers * counts)
        # Calculate expected value E[X] = sum(x * c(x) / sum(c(x)))
        sum_counts = np.sum(counts)
        mean_value = np.sum(bin_centers * counts / sum_counts)

        plt.hist(
            latencies_ms,
            bins=1000,
            density=False,
            histtype="step",
            label=f"{label}",
            color=COLORS[i],
        )

        plt.plot(
            [],
            [],
            " ",
            label=f"{label} counts: {sum_counts}",
            color="forestgreen",
        )
        plt.plot(
            [],
            [],
            " ",
            label=f"{label} mean: {mean_value:.2f}ms",
            color="forestgreen",
        )

    plt.xlabel("Latency (ms)")
    plt.ylabel("Counts")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)


def plot_2d_histogram(results: Dict[str, List[int]], title, fig_name):
    fig, ax = plt.subplots(figsize=(10, 6))

    masa_latencies = [r / 1e3 for r in results["queue_edf"]]
    fifo_latencies = [r / 1e3 for r in results["queue_fifo"]]
    h, xedges, yedges, img = ax.hist2d(
        masa_latencies,
        fifo_latencies,
        range=[[0, 30], [0, 30]],
        bins=100,
        cmap="inferno",
        density=True,
    )
    ax.set_xlabel("masa Latency (ms)")
    ax.set_ylabel("fifo Latency (ms)")
    ax.set_title(title)

    ax.set_aspect("equal")
    ax.xaxis.set_major_locator(ticker.LinearLocator(7))
    ax.yaxis.set_major_locator(ticker.LinearLocator(7))

    cbar = fig.colorbar(img, ax=ax)
    cbar.ax.set_ylabel("Density")

    plt.grid(True)
    plt.savefig(fig_name)


def plot_ratio_cdf(results: List[float], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    results.sort()
    cdf = np.arange(1, len(results) + 1) / len(results)

    plt.plot(results, cdf)

    pois = [100, 200, 300, 400, 500]
    for poi in pois:
        # plt.axvline(x=poi, color="r", linestyle="--")
        if results[-1] >= poi:
            y_value = next((y for x, y in zip(results, cdf) if x >= poi), None)
            if y_value is not None:
                plt.axhline(
                    y=y_value,
                    color="forestgreen",
                    linestyle="--",
                    label=f"CDF at x={poi}: {y_value:.2f}",
                )

    plt.legend()

    plt.xlabel("Latency Ratio (%)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.grid(True)
    plt.savefig(fig_name)


def plot_ratio_pdf(results: List[float], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    plt.hist(results, bins=1000, density=True, histtype="step", label="masa / fifo")

    plt.legend()

    plt.xlabel("Latency Ratio (%)")
    plt.ylabel("PDF")
    plt.title(title)
    plt.grid(True)
    plt.savefig(fig_name)
