import argparse
from typing import *

import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import numpy as np

plt.rcParams["font.family"] = "Roboto"
fontsize = 17
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
COLORS = ["tab:blue", "tab:orange", "tab:purple", "tab:red"]
MARKERS = ["o", "x", "^", "*"]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--path", type=str, required=True)
    parser.add_argument("--mode", type=str, required=True)
    args = parser.parse_args()
    return args


def plot_goodput(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    goodputs_load_gen = [result["goodput_load_gen"] for result in results]
    goodputs_fe = [result["goodput_fe"] for result in results]

    fig = plt.figure(figsize=(10, 6))

    plt.plot(
        rps_values,
        goodputs_load_gen,
        label="Goodput Load Gen",
        color=COLORS[0],
        marker=MARKERS[0],
    )
    plt.plot(
        rps_values,
        goodputs_fe,
        label="Goodput FE",
        color=COLORS[1],
        marker=MARKERS[1],
    )

    plt.xlabel("RPS")
    plt.ylabel("Goodput")
    plt.ylim(-200, 2200)
    plt.title(f"Goodput vs RPS ({mode})")
    plt.legend(loc="upper left")
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


def plot_throughput(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    tput_all = [result["tput_all"] for result in results]
    tput_good = [result["tput_good"] for result in results]
    tput_neutral = [result["tput_neutral"] for result in results]
    tput_error = [result["tput_error"] for result in results]

    fig = plt.figure(figsize=(10, 6))

    plt.plot(
        rps_values,
        tput_all,
        label="Throughput All",
        color=COLORS[0],
        marker=MARKERS[0],
    )
    plt.plot(
        rps_values,
        tput_good,
        label="Throughput Good",
        color=COLORS[1],
        marker=MARKERS[1],
    )
    plt.plot(
        rps_values,
        tput_neutral,
        label="Throughput Neutral",
        color=COLORS[2],
        marker=MARKERS[2],
    )
    plt.plot(
        rps_values,
        tput_error,
        label="Throughput Error",
        color=COLORS[3],
        marker=MARKERS[3],
    )

    plt.xlabel("RPS")
    plt.ylabel("Throughput")
    plt.ylim(-200, 2200)
    plt.title(f"Throughput vs RPS ({mode})")
    plt.legend(loc="upper left")
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


def plot_error(results: List[Dict[str, Any]], fig_name: str, mode: str):
    rps_values = [result["rps"] for result in results]
    error_all = [result["error_all"] for result in results]
    error_search = [result["error_search"] for result in results]
    error_profile = [result["error_profile"] for result in results]
    error_load_gen = [result["error_load_gen"] for result in results]

    fig = plt.figure(figsize=(10, 6))

    plt.plot(
        rps_values,
        error_all,
        label="Error All",
        color=COLORS[0],
        marker=MARKERS[0],
    )
    plt.plot(
        rps_values,
        error_search,
        label="Error Search",
        color=COLORS[1],
        marker=MARKERS[1],
    )
    plt.plot(
        rps_values,
        error_profile,
        label="Error Profile",
        color=COLORS[2],
        marker=MARKERS[2],
    )
    plt.plot(
        rps_values,
        error_load_gen,
        label="Error Load Gen",
        color=COLORS[3],
        marker=MARKERS[3],
    )

    plt.xlabel("RPS")
    plt.ylabel("Error")
    plt.ylim(-200, 2200)
    plt.title(f"Error vs RPS ({mode})")
    plt.legend(loc="upper left")
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


def plot_goodput_per_rps(result: Dict[str, Any], fig_name: str):
    fig, ax = plt.subplots(figsize=(10, 6))
    ax.plot(result["goodput_per_rps"], color="tab:blue")

    ax.set_title(f"Goodput per RPS", fontsize=fontsize)
    ax.set_xlabel("Time (Seconds)", fontsize=fontsize)
    ax.set_ylabel("Goodput", fontsize=fontsize)

    plt.grid(True, linestyle="--", alpha=0.7)
    plt.savefig(fig_name)
    plt.show()


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
    plt.show()


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
    plt.show()


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
    plt.show()


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
    plt.show()


def plot_ratio_pdf(results: List[float], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    plt.hist(results, bins=1000, density=True, histtype="step", label="masa / fifo")

    plt.legend()

    plt.xlabel("Latency Ratio (%)")
    plt.ylabel("PDF")
    plt.title(title)
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()
