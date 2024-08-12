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

    # plt.tight_layout()
    plt.xlabel("Latency (ms)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


plot_cdf(results, f"Latency CDF (RPS={rps})", f"fig_e2e_cdf.png")


def plot_cdf_relative(results: List[float], title: str, fig_name: str):
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

    plt.legend()  # Show legend with the label

    # plt.tight_layout()
    plt.xlabel("Relative Latency (%)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


results_relative: List[float] = []
for i in range(len(results["masa"])):
    results_relative.append(results["fifo"][i] * 100 / results["masa"][i])
plot_cdf_relative(
    results_relative,
    f"Relative Latency CDF (fifo / masa) (rps={rps})",
    f"fig_e2e_relative_cdf.png",
)
