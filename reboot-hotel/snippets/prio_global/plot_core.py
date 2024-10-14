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

# MODES = ["prio_local", "prio_global", "fifo_two"]
MODES = ["prio_global"]
GRAPH_IDS = ["Hotel"]
COLORS = ["tab:blue", "tab:orange", "tab:purple"]


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
        # p99 = float(np.percentile(latencies_ms, 99))
        # plt.axvline(
        #     x=p99,
        #     linestyle="--",
        #     label=f"{label} p99: {p99:.2f}ms",
        #     color="forestgreen",
        # )
        # plt.scatter([p99], [0.99], color="forestgreen")

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
