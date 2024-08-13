from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 1600
modes = ["masa", "fifo"]

results_raw: Dict[str, List] = {}
for mode in modes:
    file = f"r{rps}_{mode}.csv"
    df = pd.read_csv(file)

    spans: List[Dict] = []
    for _, row in df.iterrows():
        id = row["request_id"]
        span = row["span"]
        slo = row["slo"]
        latency = row["latency"]
        spans.append({"id": id, "span": span, "slo": slo, "latency": latency})
    results_raw[mode] = spans

ids_to_modes: Dict[int, List[str]] = {}
for mode in modes:
    spans = results_raw[mode]
    for span in spans:
        id = span["id"]
        ids_to_modes.setdefault(id, []).append(mode)

common_ids = set(
    id for id, modes in ids_to_modes.items() if len(modes) == 2 and modes[0] != modes[1]
)

# [TODO] Output this to a separate binary.
results_e2e: Dict[str, List[int]] = {"masa": [], "fifo": []}
for mode in modes:
    spans_filtered = []
    spans = results_raw[mode]
    for span in spans:
        id = span["id"]
        latency = span["latency"]
        if id in common_ids:
            spans_filtered.append((id, latency))
    spans_filtered.sort(key=lambda x: x[0])
    results_e2e[mode] = [x[1] for x in spans_filtered]


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


plot_cdf(results_e2e, f"E2E Latency CDF (rps={rps})", f"fig_e2e_cdf.png")

plot_pdf(results_e2e, f"E2E Latency PDF (rps={rps})", f"fig_e2e_pdf.png")


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

# [TODO] Plot a 2D histogram of masa vs fifo latencies.
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
    results_slower, f"E2E Slower Latency CDF (rps={rps})", f"fig_e2e_slower_cdf.png"
)
plot_pdf(
    results_slower, f"E2E Slower Latency PDF (rps={rps})", f"fig_e2e_slower_pdf.png"
)
plot_cdf(
    results_faster, f"E2E Faster Latency CDF (rps={rps})", f"fig_e2e_faster_cdf.png"
)
plot_pdf(
    results_faster, f"E2E Faster Latency PDF (rps={rps})", f"fig_e2e_faster_pdf.png"
)
