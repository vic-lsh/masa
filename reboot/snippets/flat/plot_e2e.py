from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 1600
modes = ["masa", "fifo"]

results: Dict[str, List[int]] = {"masa": [], "fifo": []}

for mode in modes:
    file = f"r{rps}_{mode}.csv"
    df = pd.read_csv(file)

    latencies = list(df["latency"])
    results[mode] = latencies


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
        plt.hist(latencies, bins=1000, density=True, histtype="step", label=mode)

    plt.xlabel("Latency (ms)")
    plt.ylabel("PDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


plot_cdf(results, f"Latency CDF (rps={rps})", f"fig_e2e_cdf.png")
plot_pdf(results, f"Latency PDF (rps={rps})", f"fig_e2e_pdf.png")


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
for i in range(len(results["masa"])):
    results_ratio.append(results["masa"][i] * 100 / results["fifo"][i])

plot_ratio_cdf(
    results_ratio,
    f"Latency Ratio CDF (masa / fifo) (rps={rps})",
    f"fig_e2e_ratio_cdf.png",
)

plot_ratio_pdf(
    results_ratio,
    f"Latency Ratio PDF (masa / fifo) (rps={rps})",
    f"fig_e2e_ratio_pdf.png",
)
