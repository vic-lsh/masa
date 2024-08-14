from typing import *

import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import numpy as np
import pandas as pd

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

rps = 400
modes = ["masa", "fifo"]

results_e2e: Dict[str, List[int]] = {}
for mode in modes:
    file = f"r{rps}_{mode}_filtered.csv"
    df = pd.read_csv(file)
    results_e2e[mode] = list(df["latency"])


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


def plot_2d_histogram(results: Dict[str, List[int]], title, fig_name):
    fig, ax = plt.subplots(figsize=(10, 6))

    masa_latencies = [r / 1e3 for r in results["masa"]]
    fifo_latencies = [r / 1e3 for r in results["fifo"]]
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


plot_cdf(results_e2e, f"E2E Latency CDF (rps={rps})", f"fig_e2e_cdf.png")

plot_pdf(results_e2e, f"E2E Latency PDF (rps={rps})", f"fig_e2e_pdf.png")

plot_2d_histogram(
    results_e2e,
    f"E2E Latency 2D Histogram (rps={rps})",
    f"fig_e2e_2d_hist.png",
)

exit()


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


results_ratio: List[float] = []
for i in range(len(results_e2e["masa"])):
    masa = results_e2e["masa"][i]
    fifo = results_e2e["fifo"][i]
    ratio = masa / fifo * 100
    results_ratio.append(ratio)

plot_ratio_cdf(
    results_ratio,
    f"E2E Latency Ratio CDF (masa / fifo) (rps={rps})",
    f"fig_e2e_ratio_cdf.png",
)

plot_ratio_pdf(
    results_ratio,
    f"E2E Latency Ratio PDF (masa / fifo) (rps={rps})",
    f"fig_e2e_ratio_pdf.png",
)


results_slower: Dict[str, List[int]] = {"masa": [], "fifo": []}
results_faster: Dict[str, List[int]] = {"masa": [], "fifo": []}

for i in range(len(results_e2e["masa"])):
    masa = results_e2e["masa"][i]
    fifo = results_e2e["fifo"][i]
    if masa > fifo:
        results_slower["masa"].append(masa)
        results_slower["fifo"].append(fifo)
    else:
        results_faster["masa"].append(masa)
        results_faster["fifo"].append(fifo)


plot_cdf(
    results_slower,
    f"E2E Slower Latency CDF (masa > fifo) (rps={rps})",
    f"fig_e2e_slower_cdf.png",
)
plot_pdf(
    results_slower,
    f"E2E Slower Latency PDF (masa > fifo) (rps={rps})",
    f"fig_e2e_slower_pdf.png",
)
plot_2d_histogram(
    results_slower,
    f"E2E Slower Latency 2D Histogram (masa > fifo) (rps={rps})",
    f"fig_e2e_slower_2d_hist.png",
)

plot_cdf(
    results_faster,
    f"E2E Faster Latency CDF (masa < fifo) (rps={rps})",
    f"fig_e2e_faster_cdf.png",
)
plot_pdf(
    results_faster,
    f"E2E Faster Latency PDF (masa < fifo) (rps={rps})",
    f"fig_e2e_faster_pdf.png",
)
plot_2d_histogram(
    results_faster,
    f"E2E Faster Latency 2D Histogram (masa < fifo) (rps={rps})",
    f"fig_e2e_faster_2d_hist.png",
)
