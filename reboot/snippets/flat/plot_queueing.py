from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 1600
modes = ["masa", "fifo"]
results_queueing: Dict[str, List[int]] = {}

for mode in modes:
    file = f"tmp_{mode}_log_filtered.csv"
    df = pd.read_csv(file)
    results_queueing[mode] = list(df["queueing"])


def plot_cdf(results: Dict[str, List[int]], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    for mode in modes:
        latencies = [r / 1e3 for r in results[mode]]
        latencies.sort()
        cdf = np.arange(1, len(latencies) + 1) / len(latencies)
        plt.plot(latencies, cdf, label=mode)

    plt.xlabel("Latency (ms)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


def plot_pdf(results: Dict[str, List[int]], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    for mode in modes:
        latencies = [r / 1e3 for r in results[mode]]

        counts, bin_edges = np.histogram(latencies, bins=1000, density=True)
        bin_centers = 0.5 * (bin_edges[:-1] + bin_edges[1:])
        bin_width = bin_edges[1] - bin_edges[0]
        # Calculate expected value E[X] = sum(x * p(x) * width of bin)
        expected_value = np.sum(bin_centers * counts * bin_width)

        plt.hist(
            latencies,
            bins=1000,
            density=True,
            histtype="step",
            label=f"{mode} (E[X]={expected_value:.2f}ms)",
        )

    plt.xlabel("Latency (ms)")
    plt.ylabel("PDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


plot_cdf(
    results_queueing,
    f"Queueing Latency at Second Hop (rps={rps})",
    "fig_queueing_second_hop_cdf.png",
)

plot_pdf(
    results_queueing,
    f"Queueing Latency at Second Hop (rps={rps})",
    "fig_queueing_second_hop_pdf.png",
)


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


results_queueing_ratio: List[float] = []
for i in range(len(results_queueing["masa"])):
    masa = results_queueing["masa"][i]
    fifo = results_queueing["fifo"][i]
    ratio = masa / fifo * 100
    results_queueing_ratio.append(ratio)

plot_ratio_cdf(
    results_queueing_ratio,
    f"Queueing Latency Ratio at Second Hop (masa / fifo) (rps={rps})",
    "fig_queueing_ratio_second_hop_cdf.png",
)

plot_ratio_pdf(
    results_queueing_ratio,
    f"Queueing Latency Ratio at Second Hop (masa / fifo) (rps={rps})",
    "fig_queueing_ratio_second_hop_pdf.png",
)


def plot_2d_histogram(results: Dict[str, List[int]], title, fig_name):
    fig, ax = plt.subplots(figsize=(10, 6))

    masa_latencies = [r / 1e3 for r in results["masa"]]
    fifo_latencies = [r / 1e3 for r in results["fifo"]]
    h, xedges, yedges, img = ax.hist2d(
        masa_latencies,
        fifo_latencies,
        range=[[0, 4], [0, 4]],
        bins=100,
        cmap="inferno",
        density=True,
    )
    ax.set_xlabel("masa Latency (ms)")
    ax.set_ylabel("fifo Latency (ms)")
    ax.set_title(title)

    cbar = fig.colorbar(img, ax=ax)
    cbar.ax.set_ylabel("Density")

    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


results_queueing_slower: Dict[str, List[int]] = {"masa": [], "fifo": []}
results_queueing_faster: Dict[str, List[int]] = {"masa": [], "fifo": []}

for i in range(len(results_queueing["masa"])):
    masa = results_queueing["masa"][i]
    fifo = results_queueing["fifo"][i]
    if masa > fifo:
        results_queueing_slower["masa"].append(masa)
        results_queueing_slower["fifo"].append(fifo)
    else:
        results_queueing_faster["masa"].append(masa)
        results_queueing_faster["fifo"].append(fifo)

plot_cdf(
    results_queueing_slower,
    f"Queueing Slower Latency at Second Hop (masa > fifo) (rps={rps})",
    "fig_queueing_slower_second_hop_cdf.png",
)
plot_pdf(
    results_queueing_slower,
    f"Queueing Slower Latency at Second Hop (masa > fifo) (rps={rps})",
    "fig_queueing_slower_second_hop_pdf.png",
)
plot_2d_histogram(
    results_queueing_slower,
    f"Queueing Slower Latency at Second Hop 2D Histogram (masa > fifo) (rps={rps})",
    "fig_queueing_slower_second_hop_2d_hist.png",
)

plot_cdf(
    results_queueing_faster,
    f"Queueing Faster Latency at Second Hop (masa < fifo) (rps={rps})",
    "fig_queueing_faster_second_hop_cdf.png",
)
plot_pdf(
    results_queueing_faster,
    f"Queueing Faster Latency at Second Hop (masa < fifo) (rps={rps})",
    "fig_queueing_faster_second_hop_pdf.png",
)
plot_2d_histogram(
    results_queueing_faster,
    f"Queueing Faster Latency at Second Hop 2D Histogram (masa < fifo) (rps={rps})",
    "fig_queueing_faster_second_hop_2d_hist.png",
)

plot_2d_histogram(
    results_queueing,
    f"Queueing Latency at Second Hop 2D Histogram (rps={rps})",
    "fig_queueing_second_hop_2d_hist.png",
)
